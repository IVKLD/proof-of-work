mod archive;
mod config;
mod indicator;
mod niri;
mod stamper;
#[cfg(test)]
mod soak_test;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::Local;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use serde::{Deserialize, Serialize};

use config::Config;

#[derive(Parser)]
#[command(
    name = "worktrack",
    about = "Screen screenshot & time tracker for Wayland (niri)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Запустить трекер в фоне
    Start,
    /// Сделать тестовый снимок экрана прямо сейчас
    Shot,
    /// Внутренний процесс демона
    Daemon,
    /// Поставить трекер на паузу (индикатор станет жёлтым)
    Pause,
    /// Снять с паузы (индикатор станет красным, снимок сразу)
    Resume,
    /// Сбросить рабочую смену за сегодня (с подтверждением)
    Reset {
        /// Пропустить подтверждение (сброс без запроса)
        #[arg(short, long)]
        yes: bool,
    },
    /// Остановить трекер и собрать архив за сегодня
    Stop,
    /// Показать текущий статус трекера и количество снимков
    Status,
    /// Показать открытые окна
    Windows,
    /// Показать доступные рабочие пространства (workspaces) в niri
    Workspaces,
    /// Показать доступные мониторы для WORKTRACK_OUTPUT
    Outputs,
    /// Собрать архив за сегодняшний день
    Pack,
    /// Отправить архив за сегодня на FTP (с подтверждением и отчётом по отработанным часам)
    Upload {
        /// Пропустить подтверждение (отправить сразу без запроса)
        #[arg(short, long)]
        yes: bool,
    },
    /// Сгенерировать автодополнение для терминала (bash, zsh, fish)
    Completions {
        /// Оболочка: bash, zsh, fish, elvish, powershell
        shell: Shell,
    },
    /// Скачать и установить последний готовый бинарник с GitHub Releases без компиляции
    Update,
}

fn runtime_file(name: &str) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(dir).join(format!("worktrack.{name}"))
}

fn resolve_target_output(
    cfg_out: Option<&str>,
    ws_out: Option<&str>,
    all_outputs: &[niri::OutputInfo],
) -> String {
    cfg_out
        .or(ws_out)
        .map(|s| s.to_string())
        .or_else(|| niri::get_focused_output().map(|o| o.name))
        .or_else(|| all_outputs.first().map(|o| o.name.clone()))
        .unwrap_or_else(|| "DP-1".to_string())
}

fn is_process_running(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    if unsafe { libc::kill(pid, 0) != 0 } {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        let cmdline_path = format!("/proc/{pid}/cmdline");
        if let Ok(cmdline) = fs::read_to_string(&cmdline_path) {
            return cmdline.contains("worktrack");
        }
    }
    true
}

fn get_running_pid() -> Option<i32> {
    let pid = fs::read_to_string(runtime_file("pid")).ok()?.trim().parse().ok()?;
    is_process_running(pid).then_some(pid)
}

fn append_log(day_dir: &Path, msg: &str) {
    let log_path = day_dir.join("events.log");
    let now = Local::now().format("%H:%M:%S");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = writeln!(f, "{now} {msg}");
    }
}

fn restore_focus(
    prev_win_id: Option<u64>,
    prev_active_ws: Option<niri::Workspace>,
    prev_focused_output: Option<String>,
) {
    if let Some(win_id) = prev_win_id && niri::focus_window(win_id).is_ok() {
        return;
    }
    if let Some(prev_ws) = prev_active_ws {
        if let Some(ref out) = prev_ws.output {
            let _ = niri::focus_monitor(out);
        }
        let _ = prev_ws.focus();
        if let Some(ref prev_out) = prev_focused_output {
            let _ = niri::focus_monitor(prev_out);
        }
    } else {
        let _ = niri::focus_workspace_previous();
    }
}

fn capture_screen(
    day_dir: &Path,
    cfg: &Config,
    target_output_holder: Option<&Arc<std::sync::RwLock<Option<String>>>>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let now_stamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let time_str = Local::now().format("%H-%M-%S").to_string();

    let ext = match cfg.image_format.as_str() {
        "png" => "png",
        _ => "jpg",
    };
    let file_name = format!("{time_str}.{ext}");
    let mut file_path = day_dir.join(&file_name);
    if file_path.exists() {
        let ms = Local::now().timestamp_subsec_millis();
        let alt_name = format!("{time_str}_{ms:03}.{ext}");
        file_path = day_dir.join(&alt_name);
    }
    let file_str = file_path.to_string_lossy().to_string();

    // 1. Resolve target workspace and output
    let all_workspaces = niri::get_all_workspaces().unwrap_or_default();
    let all_outputs = niri::get_all_outputs().unwrap_or_default();
    let target_ws = cfg
        .workspace
        .as_deref()
        .and_then(|q| niri::find_workspace(&all_workspaces, q, cfg.output.as_deref()));

    let out_name = resolve_target_output(
        cfg.output.as_deref(),
        target_ws.as_ref().and_then(|w| w.output.as_deref()),
        &all_outputs,
    );

    let (
        target_output,
        target_ws_id,
        needs_restore,
        prev_win_id,
        prev_active_ws,
        prev_focused_output,
    ) = if let Some(ref ws) = target_ws {
        if ws.is_active {
            (out_name, Some(ws.id), false, None, None, None)
        } else {
            let prev_win = niri::get_focused_window_id();
            let prev_active = all_workspaces
                .iter()
                .find(|w| w.output.as_deref() == Some(&out_name) && w.is_active)
                .cloned();
            let prev_focused_out = niri::get_focused_output().map(|o| o.name);

            let _ = niri::focus_monitor(&out_name);
            let _ = ws.focus();
            std::thread::sleep(Duration::from_millis(380));

            (
                out_name,
                Some(ws.id),
                true,
                prev_win,
                prev_active,
                prev_focused_out,
            )
        }
    } else {
        if let Some(ref q) = cfg.workspace {
            append_log(
                day_dir,
                &format!("WARN: configured workspace '{q}' not found; falling back to output"),
            );
        }
        (out_name, None, false, None, None, None)
    };

    if let Some(holder) = target_output_holder {
        let _ = holder.write().map(|mut lock| {
            if lock.as_deref() != Some(&target_output) {
                *lock = Some(target_output.clone());
            }
        });
    }

    // 2. Perform capture
    let res = niri::capture_screenshot(&file_str, Some(&target_output), cfg.jpeg_quality);

    // Compute blocked rects immediately while layout state matches captured frame
    let blocked_rects = match res {
        Ok(ref out_name) => {
            if let Some(ws_id) = target_ws_id {
                niri::get_blocked_window_rects_for_workspace(ws_id, out_name, &cfg.blocked_apps)
            } else {
                niri::get_blocked_window_rects_on_output(out_name, &cfg.blocked_apps)
            }
        }
        Err(_) => Vec::new(),
    };

    // 3. Restore previous workspace/window if we switched
    if needs_restore {
        restore_focus(prev_win_id, prev_active_ws, prev_focused_output);
    }

    // 4. Apply watermark & blackout blocked apps if any
    match res {
        Ok(_) => {
            if let Err(err) = stamper::stamp_screenshot(
                &file_path,
                &now_stamp,
                &blocked_rects,
                cfg.jpeg_quality,
                cfg.max_width,
            ) {
                append_log(day_dir, &format!("WARN: could not stamp {file_name}: {err}"));
            }
            Ok(file_path)
        }
        Err(err) => {
            append_log(day_dir, &format!("ERROR: screenshot failed: {err}"));
            let _ = fs::remove_file(&file_path);
            Err(err)
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ShiftState {
    pub date: String,
    pub first_start: String,
    pub current_session_start: Option<String>,
    pub total_worked_secs: u64,
}

impl ShiftState {
    fn new(date: String, start_time: Option<String>, now_s: String) -> Self {
        let first_start = start_time.unwrap_or_else(|| now_s.clone());
        Self {
            date,
            first_start,
            current_session_start: Some(now_s),
            total_worked_secs: 0,
        }
    }
}

fn shift_state_path(day_dir: &Path) -> PathBuf {
    day_dir.join("shift.json")
}

fn is_screenshot_file(p: &Path) -> bool {
    p.extension()
        .is_some_and(|ext| ext == "jpg" || ext == "jpeg" || ext == "png")
}

fn get_first_screenshot_time(dir: &Path, today: &str) -> Option<String> {
    let mut shots: Vec<String> = fs::read_dir(dir.join(today))
        .ok()?
        .flatten()
        .filter(|e| is_screenshot_file(&e.path()))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    shots.sort();
    shots.into_iter().find_map(|shot| {
        let clean = Path::new(&shot).file_stem()?.to_str()?.split('_').next()?;
        let parts: Vec<&str> = clean.split('-').collect();
        (parts.len() == 3 && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())))
            .then(|| parts.join(":"))
    })
}

fn load_shift_state(day_dir: &Path, today: &str, cfg: &Config) -> (Option<String>, u64) {
    let p = shift_state_path(day_dir);
    if let Ok(content) = fs::read_to_string(&p)
        && let Ok(state) = serde_json::from_str::<ShiftState>(&content)
        && state.date == today
    {
        let first_start = cfg.start_time.clone().or(Some(state.first_start));
        return (first_start, state.total_worked_secs);
    }

    let first_start = cfg
        .start_time
        .clone()
        .or_else(|| get_first_screenshot_time(&cfg.dir, today));
    let shots = count_screenshots_today(&cfg.dir);
    let prior_secs = (shots as u64) * cfg.interval_secs;
    (first_start, prior_secs)
}

fn save_shift_state(day_dir: &Path, state: &ShiftState) {
    let p = shift_state_path(day_dir);
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = fs::write(p, json);
    }
}

fn run_daemon(cfg: Config) {
    let lock_path = runtime_file("lock");
    let lock_file = match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(_) => return,
    };
    use std::os::fd::AsRawFd;
    if unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        // Another daemon instance is already active and holding the kernel lock
        return;
    }
    let _lock_guard = lock_file;

    let pid = std::process::id() as i32;
    let _ = fs::write(runtime_file("pid"), pid.to_string());

    let running = Arc::new(AtomicBool::new(true));
    let paused = Arc::new(AtomicBool::new(runtime_file("paused").exists()));

    // Hook signals
    let r_sig = running.clone();
    let _ = ctrlc::set_handler(move || {
        r_sig.store(false, Ordering::SeqCst);
    });

    // Resolve initial target output for indicator
    let all_workspaces = niri::get_all_workspaces().unwrap_or_default();
    let initial_target = cfg
        .workspace
        .as_deref()
        .and_then(|q| niri::find_workspace(&all_workspaces, q, cfg.output.as_deref()))
        .and_then(|w| w.output)
        .or_else(|| cfg.output.clone())
        .or_else(|| niri::get_focused_output().map(|o| o.name));

    let target_output = Arc::new(std::sync::RwLock::new(initial_target));

    let initial_day = Local::now().format("%Y-%m-%d").to_string();
    let initial_dir = cfg.dir.join(&initial_day);
    let _ = fs::create_dir_all(&initial_dir);

    let (first_start_opt, mut prior_worked_secs) =
        load_shift_state(&initial_dir, &initial_day, &cfg);
    let mut session_start_time = Local::now().format("%H:%M:%S").to_string();
    let mut first_start_time = first_start_opt.unwrap_or_else(|| session_start_time.clone());
    let mut session_worked_secs: u64 = 0;

    let daily_goal_secs = (cfg.daily_hours * 3600.0).round() as u64;
    let initial_rem_work = if prior_worked_secs >= daily_goal_secs {
        -((prior_worked_secs - daily_goal_secs) as i64)
    } else {
        (daily_goal_secs - prior_worked_secs) as i64
    };

    let mut current_shift = ShiftState {
        date: initial_day.clone(),
        first_start: first_start_time.clone(),
        current_session_start: Some(session_start_time.clone()),
        total_worked_secs: prior_worked_secs,
    };
    save_shift_state(&initial_dir, &current_shift);

    // Start Wayland indicator thread
    let ind_running = running.clone();
    let ind_paused = paused.clone();
    let countdown = Arc::new(std::sync::atomic::AtomicI64::new(cfg.interval_secs as i64));
    let work_remaining = Arc::new(std::sync::atomic::AtomicI64::new(initial_rem_work));
    let ind_countdown = countdown.clone();
    let ind_work_rem = work_remaining.clone();
    let ind_scale = cfg.scale;
    let ind_target_out = target_output.clone();
    let ind_thread = std::thread::spawn(move || {
        indicator::run_indicator(
            ind_paused,
            ind_running,
            ind_countdown,
            ind_work_rem,
            ind_scale,
            ind_target_out,
        );
    });

    append_log(&initial_dir, "START");

    let mut current_day = initial_day;
    let mut last_capture = std::time::Instant::now()
        .checked_sub(Duration::from_secs(cfg.interval_secs))
        .unwrap_or_else(std::time::Instant::now);

    let mut prev_paused = paused.load(Ordering::Relaxed);
    let capturing = Arc::new(AtomicBool::new(false));
    let mut last_second_tick = std::time::Instant::now();

    while running.load(Ordering::SeqCst) {
        let reset_file = runtime_file("reset");
        if reset_file.exists() {
            let _ = fs::remove_file(&reset_file);
            prior_worked_secs = 0;
            session_worked_secs = 0;
            let now_s = Local::now().format("%H:%M:%S").to_string();
            current_shift = ShiftState::new(current_day.clone(), cfg.start_time.clone(), now_s.clone());
            first_start_time = current_shift.first_start.clone();
            session_start_time = now_s.clone();
            save_shift_state(&cfg.dir.join(&current_day), &current_shift);
            work_remaining.store(daily_goal_secs as i64, Ordering::Relaxed);
            let _ = fs::write(
                runtime_file("stats"),
                format!("0\n0\n{daily_goal_secs}\n{daily_goal_secs}\n{first_start_time}\n{now_s}"),
            );
            append_log(&cfg.dir.join(&current_day), "RESET");
        }

        let is_paused = runtime_file("paused").exists();
        paused.store(is_paused, Ordering::Relaxed);

        let today = Local::now().format("%Y-%m-%d").to_string();
        if today != current_day {
            // Finalize previous day shift
            current_shift.current_session_start = None;
            save_shift_state(&cfg.dir.join(&current_day), &current_shift);

            current_day = today.clone();
            let day_dir = cfg.dir.join(&today);
            let _ = fs::create_dir_all(&day_dir);
            prior_worked_secs = 0;
            session_worked_secs = 0;
            let now_s = Local::now().format("%H:%M:%S").to_string();
            current_shift = ShiftState::new(today.clone(), cfg.start_time.clone(), now_s.clone());
            first_start_time = current_shift.first_start.clone();
            session_start_time = now_s;
            save_shift_state(&day_dir, &current_shift);
        }

        if prev_paused != is_paused {
            prev_paused = is_paused;
            let day_dir = cfg.dir.join(&today);
            let _ = fs::create_dir_all(&day_dir);
            current_shift.total_worked_secs = prior_worked_secs + session_worked_secs;
            save_shift_state(&day_dir, &current_shift);
            if is_paused {
                append_log(&day_dir, "PAUSE");
            } else {
                append_log(&day_dir, "RESUME");
                // Immediately take screenshot on unpause
                last_capture = std::time::Instant::now()
                    .checked_sub(Duration::from_secs(cfg.interval_secs))
                    .unwrap_or_else(std::time::Instant::now);
            }
        }

        if last_second_tick.elapsed() >= Duration::from_secs(1) {
            let elapsed_ticks = last_second_tick.elapsed().as_secs();
            last_second_tick += Duration::from_secs(elapsed_ticks);
            if !is_paused {
                // If elapsed_ticks > 3, computer was suspended/asleep or frozen.
                // Do not count suspended sleep time as worked time!
                let actual_elapsed = elapsed_ticks.min(2);
                session_worked_secs += actual_elapsed;
            }

            let total_worked = prior_worked_secs + session_worked_secs;
            let rem_work = if total_worked >= daily_goal_secs {
                -((total_worked - daily_goal_secs) as i64)
            } else {
                (daily_goal_secs - total_worked) as i64
            };
            work_remaining.store(rem_work, Ordering::Relaxed);

            current_shift.total_worked_secs = total_worked;
            if session_worked_secs.is_multiple_of(15) {
                save_shift_state(&cfg.dir.join(&current_day), &current_shift);
            }

            let _ = fs::write(
                runtime_file("stats"),
                format!("{prior_worked_secs}\n{session_worked_secs}\n{daily_goal_secs}\n{rem_work}\n{first_start_time}\n{session_start_time}"),
            );
        }

        if is_paused {
            countdown.store(-1, Ordering::Relaxed);
        } else {
            let elapsed = last_capture.elapsed().as_secs();
            let rem = (cfg.interval_secs as i64) - (elapsed as i64);
            countdown.store(rem.max(0), Ordering::Relaxed);
        }

        if !is_paused
            && last_capture.elapsed() >= Duration::from_secs(cfg.interval_secs)
            && !capturing.load(Ordering::SeqCst)
        {
            last_capture = std::time::Instant::now();
            countdown.store(cfg.interval_secs as i64, Ordering::Relaxed);
            capturing.store(true, Ordering::SeqCst);

            let cap_cfg = cfg.clone();
            let day_dir = cfg.dir.join(&today);
            let _ = fs::create_dir_all(&day_dir);
            let cap_target = target_output.clone();
            let cap_flag = capturing.clone();

            std::thread::spawn(move || {
                struct FlagGuard(Arc<AtomicBool>);
                impl Drop for FlagGuard {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::SeqCst);
                    }
                }
                let _guard = FlagGuard(cap_flag);
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = capture_screen(&day_dir, &cap_cfg, Some(&cap_target));
                }));
            });
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    let today = Local::now().format("%Y-%m-%d").to_string();
    let day_dir = cfg.dir.join(&today);
    current_shift.current_session_start = None;
    current_shift.total_worked_secs = prior_worked_secs + session_worked_secs;
    save_shift_state(&day_dir, &current_shift);
    append_log(&day_dir, "STOP");

    for f in ["stats", "pid", "paused", "reset"] {
        let _ = fs::remove_file(runtime_file(f));
    }
    let _ = ind_thread.join();
}

fn format_duration_hm(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h} ч {m:02} мин")
    } else if m > 0 {
        format!("{m} мин")
    } else {
        format!("{s} сек")
    }
}

fn format_duration_hms(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h} ч {m:02} мин {s:02} сек")
    } else if m > 0 {
        format!("{m} мин {s:02} сек")
    } else {
        format!("{s} сек")
    }
}

fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn file_size_suffix(path: &Path) -> String {
    fs::metadata(path)
        .map(|m| format!(" ({})", format_file_size(m.len())))
        .unwrap_or_default()
}

fn confirm_action(prompt: &str) -> Result<bool, Box<dyn std::error::Error>> {
    print!("{prompt} [y/N]: ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let ans = input.trim().to_lowercase();
    Ok(matches!(ans.as_str(), "y" | "yes" | "д" | "да"))
}

struct RuntimeStats {
    prior: u64,
    session: u64,
    _goal: u64,
    rem: i64,
    first_start: Option<String>,
    session_start: Option<String>,
}

fn read_stats_file() -> Option<RuntimeStats> {
    let content = fs::read_to_string(runtime_file("stats")).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() < 4 {
        return None;
    }
    Some(RuntimeStats {
        prior: lines[0].parse().ok()?,
        session: lines[1].parse().ok()?,
        _goal: lines[2].parse().ok()?,
        rem: lines[3].parse().ok()?,
        first_start: lines.get(4).filter(|s| !s.is_empty()).map(|s| s.to_string()),
        session_start: lines.get(5).filter(|s| !s.is_empty()).map(|s| s.to_string()),
    })
}

fn get_today_worked_secs(cfg: &Config) -> (u64, usize) {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let count = count_screenshots_today(&cfg.dir);
    if get_running_pid().is_some()
        && let Some(st) = read_stats_file()
    {
        return (st.prior + st.session, count);
    }
    let (_, prior) = load_shift_state(&cfg.dir.join(&today), &today, cfg);
    (prior, count)
}

fn print_status(cfg: &Config) {
    let pid = get_running_pid();
    let is_running = pid.is_some();
    match pid {
        Some(p) => {
            let paused = runtime_file("paused").exists();
            if paused {
                println!("Статус: НА ПАУЗЕ (PID {p})");
            } else {
                println!("Статус: РАБОТАЕТ (PID {p})");
            }
        }
        None => {
            println!("Статус: ОСТАНОВЛЕН");
        }
    }

    let count = count_screenshots_today(&cfg.dir);
    let daily_goal_secs = (cfg.daily_hours * 3600.0).round() as u64;
    let today = Local::now().format("%Y-%m-%d").to_string();
    let day_dir = cfg.dir.join(&today);

    let (prior_secs, session_secs, total_worked_secs, rem_secs, first_start, session_start) =
        if is_running && let Some(st) = read_stats_file() {
            (st.prior, st.session, st.prior + st.session, st.rem, st.first_start, st.session_start)
        } else {
            let (first_s, prior) = load_shift_state(&day_dir, &today, cfg);
            let rem = (daily_goal_secs as i64) - (prior as i64);
            (prior, 0, prior, rem, first_s, None)
        };

    if let Some(ref first_s) = first_start {
        if is_running
            && let Some(ref s_start) = session_start
            && s_start != first_s
        {
            println!("Время начала смены: {first_s} (текущая сессия с {s_start})");
        } else {
            println!("Время начала смены: {first_s}");
        }
    } else {
        println!("Смена сегодня ещё не начата");
    }

    println!("Скриншотов за сегодня: {count}");

    let goal_str = format_duration_hm(daily_goal_secs);
    let total_str = format_duration_hm(total_worked_secs);
    let percent = if daily_goal_secs > 0 {
        ((total_worked_secs as f64 / daily_goal_secs as f64) * 100.0).min(100.0)
    } else {
        100.0
    };

    println!("Цель на день: {goal_str} (WORKTRACK_DAILY_HOURS={})", cfg.daily_hours);
    if is_running && prior_secs > 0 {
        println!("  - В прошлых сменах сегодня: {}", format_duration_hm(prior_secs));
        println!("  - В текущей смене: {}", format_duration_hm(session_secs));
    }
    println!("Всего отработано сегодня: {total_str} ({percent:.0}%)");

    if rem_secs <= 0 {
        let overtime = (-rem_secs) as u64;
        if overtime >= 60 {
            println!("Норма выполнена! Переработка: {}", format_duration_hm(overtime));
        } else {
            println!("Норма на сегодня полностью выполнена!");
        }
    } else {
        let rem_u = rem_secs as u64;
        println!("Осталось отработать сегодня: {}", format_duration_hm(rem_u));
        let est_end = Local::now() + chrono::Duration::seconds(rem_secs);
        if is_running {
            println!("Расчётное окончание рабочего дня: {}", est_end.format("%H:%M:%S"));
        } else {
            println!("Расчётное окончание (если продолжить сейчас): {}", est_end.format("%H:%M:%S"));
        }
    }

    if let Some(ref ws) = cfg.workspace {
        println!("Настроенный воркспейс: WORKTRACK_WORKSPACE=\"{ws}\"");
    } else if let Some(ref out) = cfg.output {
        println!("Настроенный монитор: WORKTRACK_OUTPUT=\"{out}\"");
    } else {
        println!("Снимается активный монитор с фокусом");
    }
}

fn print_workspaces(cfg: &Config) {
    let Ok(workspaces) = niri::get_all_workspaces() else {
        println!("Ошибка получения рабочих пространств niri");
        return;
    };
    let windows = niri::get_all_windows().unwrap_or_default();
    let matched_ws = cfg
        .workspace
        .as_deref()
        .and_then(|q| niri::find_workspace(&workspaces, q, cfg.output.as_deref()));

    println!("Рабочие пространства niri (workspaces):");
    println!("{}", "-".repeat(70));
    for ws in &workspaces {
        let is_active = if ws.is_active { " [активен]" } else { "" };
        let is_focused = if ws.is_focused { " [фокус]" } else { "" };
        let name = ws.name.as_deref().unwrap_or("<без имени>");
        let out = ws.output.as_deref().unwrap_or("?");
        let idx = ws.idx.map_or_else(|| "?".into(), |i| i.to_string());
        let match_tag = if matched_ws.as_ref().is_some_and(|m| m.id == ws.id) {
            " <== WORKTRACK_WORKSPACE"
        } else {
            ""
        };

        let ws_wins: Vec<&niri::Window> = windows
            .iter()
            .filter(|w| w.workspace_id == Some(ws.id))
            .collect();

        println!(
            "  * id: {:<2} idx: {:<2} output: {:<5} name: {:<15} окон: {:<2}{is_active}{is_focused}{match_tag}",
            ws.id, idx, out, name, ws_wins.len()
        );

        for w in ws_wins {
            let app = w.app_id.as_deref().unwrap_or("?");
            let title = w.title.as_deref().unwrap_or("");
            let blocked = if niri::is_window_blocked(w, &cfg.blocked_apps) {
                " [ЗАПРЕЩЕНО]"
            } else {
                ""
            };
            println!("      - [id: {}] {app}: {title}{blocked}", w.id);
        }
    }

    println!();
    if let Some(ref target) = cfg.workspace {
        if let Some(w) = matched_ws {
            let name = w.name.as_deref().unwrap_or("<без имени>");
            let out = w.output.as_deref().unwrap_or("?");
            println!("Настроен воркспейс: WORKTRACK_WORKSPACE=\"{target}\" (найден: \"{name}\" на {out})");
        } else {
            println!("Настроен воркспейс: WORKTRACK_WORKSPACE=\"{target}\" (ВНИМАНИЕ: не найден в niri!)");
        }
    } else {
        println!(
            "Воркспейс не настроен (WORKTRACK_WORKSPACE не задан). Снимается монитор: {}",
            cfg.output.as_deref().unwrap_or("активный с фокусом")
        );
    }
}

fn count_screenshots_today(dir: &Path) -> usize {
    let today = Local::now().format("%Y-%m-%d").to_string();
    fs::read_dir(dir.join(today))
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| is_screenshot_file(&e.path()))
                .count()
        })
        .unwrap_or(0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Reset SIGPIPE to default so piping to `head` exits cleanly without Rust panic
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();
    let cfg = Config::load();

    match cli.command {
        None => {
            print_status(&cfg);
            println!(
                "\nДоступные команды: worktrack <start|shot|pause|resume|reset|status|workspaces|windows|stop|pack|upload|completions>"
            );
            println!("Подробная справка по аргументам: worktrack --help");
        }

        Some(Commands::Start) => {
            if let Some(pid) = get_running_pid() {
                println!("Трекер уже запущен (PID {pid})");
                return Ok(());
            }

            let _ = fs::remove_file(runtime_file("paused"));

            let exe = std::env::current_exe()?;
            let mut cmd = Command::new(exe);
            cmd.arg("daemon")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());

            #[cfg(unix)]
            unsafe {
                use std::os::unix::process::CommandExt;
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }

            let child = cmd.spawn()?;
            let _ = fs::write(runtime_file("pid"), child.id().to_string());
            println!("Трекер запущен (PID {})", child.id());
        }

        Some(Commands::Shot) => {
            let today = Local::now().format("%Y-%m-%d").to_string();
            let day_dir = cfg.dir.join(&today);
            fs::create_dir_all(&day_dir)?;
            let path = capture_screen(&day_dir, &cfg, None)?;
            let count = count_screenshots_today(&cfg.dir);
            println!("Снимок сохранён в {path:?} (всего за день: {count})");
        }

        Some(Commands::Daemon) => {
            run_daemon(cfg);
        }

        Some(Commands::Pause) => {
            if get_running_pid().is_some() {
                let _ = fs::File::create(runtime_file("paused"));
                println!("Трекер поставлен на паузу (индикатор жёлтый)");
            } else {
                println!("Трекер не запущен");
            }
        }

        Some(Commands::Resume) => {
            if get_running_pid().is_some() {
                let _ = fs::remove_file(runtime_file("paused"));
                println!("Трекер снят с паузы (индикатор красный)");
            } else {
                println!("Трекер не запущен");
            }
        }

        Some(Commands::Reset { yes }) => {
            let today = Local::now().format("%Y-%m-%d").to_string();
            let day_dir = cfg.dir.join(&today);
            let _ = fs::create_dir_all(&day_dir);

            if !yes {
                println!("Внимание: сброс рабочей смены за сегодня ({today}).");
                println!("Накопленное время и таймер рабочего дня будут обнулены.");
                if !confirm_action("Вы уверены, что хотите сбросить смену?")? {
                    println!("Сброс отменён.");
                    return Ok(());
                }
            }

            if get_running_pid().is_some() {
                let reset_file = runtime_file("reset");
                let _ = fs::File::create(&reset_file);
                for _ in 0..10 {
                    if !reset_file.exists() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                println!("Рабочая смена успешно сброшена! Счётчик обнулён.");
            } else {
                let mut shift = ShiftState::new(
                    today.clone(),
                    cfg.start_time.clone(),
                    Local::now().format("%H:%M:%S").to_string(),
                );
                shift.current_session_start = None;
                save_shift_state(&day_dir, &shift);
                append_log(&day_dir, "RESET");
                println!("Рабочая смена за сегодня успешно сброшена (счётчик обнулён).");
            }
        }

        Some(Commands::Status) => {
            print_status(&cfg);
        }

        Some(Commands::Completions { shell }) => {
            let mut cmd = Cli::command();
            let bin_name = cmd.get_name().to_string();
            generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
        }

        Some(Commands::Windows) => {
            let windows = niri::get_all_windows()?;
            println!("{:<10} {:<28} TITLE", "ID", "APP_ID");
            println!("{}", "-".repeat(70));
            for w in windows {
                let app = w.app_id.as_deref().unwrap_or("<unknown>");
                let title = w.title.as_deref().unwrap_or("");
                println!("{:<10} {:<28} {}", w.id, app, title);
            }
        }

        Some(Commands::Workspaces) => {
            print_workspaces(&cfg);
        }

        Some(Commands::Outputs) => {
            let outputs = niri::get_all_outputs()?;
            println!("{:<12} {:<20} MODEL", "NAME", "MAKE");
            println!("{}", "-".repeat(50));
            for o in outputs {
                let make = o.make.as_deref().unwrap_or("-");
                let model = o.model.as_deref().unwrap_or("-");
                println!("{:<12} {:<20} {}", o.name, make, model);
            }
        }

        Some(Commands::Stop) => {
            let (total_worked, _count) = get_today_worked_secs(&cfg);

            if let Some(pid) = get_running_pid() {
                println!("Останавливаю процесс (PID {pid})...");
                unsafe { libc::kill(pid, libc::SIGTERM) };

                // Wait up to 3 seconds for daemon to cleanly terminate and write STOP log
                for _ in 0..30 {
                    if !is_process_running(pid) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                if is_process_running(pid) {
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                }
            } else {
                println!("Процесс трекера не запущен");
            }

            for f in ["paused", "pid", "reset"] {
                let _ = fs::remove_file(runtime_file(f));
            }

            let today = Local::now().format("%Y-%m-%d").to_string();
            let day_dir = cfg.dir.join(&today);
            let (_, saved_worked) = load_shift_state(&day_dir, &today, &cfg);
            let worked_secs = if saved_worked > 0 { saved_worked } else { total_worked };
            let worked_str = format_duration_hms(worked_secs);
            let count_now = count_screenshots_today(&cfg.dir);

            println!("\n{:=<55}", "");
            println!("  Смена завершена");
            println!("{:-<55}", "");
            println!("  • Отработано сегодня:  {worked_str}");
            println!("  • Скриншотов за день: {count_now}");
            println!("{:=<55}\n", "");

            match archive::pack_day(&cfg.dir, &today) {
                Ok(archive_path) => {
                    let size_str = file_size_suffix(&archive_path);
                    println!("Архив готов: {archive_path:?}{size_str}");
                    println!("Для отправки архива на FTP выполните:\n  worktrack upload");
                }
                Err(err) => {
                    println!("Предупреждение: не удалось собрать архив: {err}");
                }
            }
        }

        Some(Commands::Pack) => {
            let today = Local::now().format("%Y-%m-%d").to_string();
            let archive_path = archive::pack_day(&cfg.dir, &today)?;
            let (total_worked, count) = get_today_worked_secs(&cfg);
            let size_str = file_size_suffix(&archive_path);
            println!("Архив собран: {archive_path:?}{size_str}");
            println!("Всего отработано сегодня: {}", format_duration_hms(total_worked));
            println!("Скриншотов за день: {count}");
            println!("Для отправки архива на FTP выполните:\n  worktrack upload");
        }

        Some(Commands::Upload { yes }) => {
            let today = Local::now().format("%Y-%m-%d").to_string();
            let archive_path = cfg.dir.join(format!("{today}.tar.gz"));
            let archive_path = if !archive_path.exists() {
                let day_dir = cfg.dir.join(&today);
                if !day_dir.exists() {
                    return Err(format!(
                        "Директория за сегодня {day_dir:?} не найдена. Нечего отправлять."
                    )
                    .into());
                }
                println!("Архив ещё не создан. Упаковываю снимки за {today}...");
                archive::pack_day(&cfg.dir, &today)?
            } else {
                archive_path
            };

            let (total_worked, count) = get_today_worked_secs(&cfg);
            let worked_str = format_duration_hms(total_worked);
            let goal_secs = (cfg.daily_hours * 3600.0).round() as u64;
            let goal_str = format_duration_hm(goal_secs);
            let archive_size = fs::metadata(&archive_path)
                .map(|m| format_file_size(m.len()))
                .unwrap_or_else(|_| "неизвестно".to_string());

            let destination_info = if let Some(url) = cfg.ftp.target_url(None) {
                url
            } else if let Some(ref cmd) = cfg.upload_cmd {
                format!("Команда: {cmd}")
            } else {
                "Не настроено (укажите WORKTRACK_FTP_* в ~/.config/worktrack.env)".to_string()
            };

            println!("\n{:=<60}", "");
            println!("  ОТЧЁТ ПЕРЕД ВЫГРУЗКОЙ ({today})");
            println!("{:-<60}", "");
            println!("  • Отработано сегодня:  {worked_str} (цель: {goal_str})");
            println!("  • Скриншотов в архиве: {count}");
            println!(
                "  • Файл архива:         {} ({archive_size})",
                archive_path.display()
            );
            println!("  • Назначение FTP:      {destination_info}");
            println!("{:=<60}\n", "");

            if !cfg.ftp.is_configured() && cfg.upload_cmd.is_none() {
                return Err(
                    "Выгрузка не настроена. Укажите WORKTRACK_FTP_HOST, WORKTRACK_FTP_USER, WORKTRACK_FTP_PASS или WORKTRACK_UPLOAD_CMD в ~/.config/worktrack.env".into(),
                );
            }

            if !yes && !confirm_action("Отправить архив работодателю на FTP?")? {
                println!("Отправка отменена.");
                return Ok(());
            }

            archive::upload_archive(&archive_path, cfg.upload_cmd.as_deref(), &cfg.ftp)?;
        }
        Some(Commands::Update) => {
            handle_self_update()?;
        }
    }

    Ok(())
}

fn handle_self_update() -> Result<(), Box<dyn std::error::Error>> {
    let current_exe = std::env::current_exe()?;
    let url = "https://github.com/IVKLD/proof-of-work/releases/latest/download/worktrack-linux-x86_64";

    println!("=== Обновление worktrack (готовый бинарник) ===");
    println!("Текущий путь: {}", current_exe.display());
    println!("Скачивание последнего релиза: {url}");

    let pid = std::process::id();
    let tmp_path = current_exe.with_extension(format!("update.tmp.{pid}"));

    let success = Command::new("curl")
        .args(["-fsSL", url, "-o"])
        .arg(&tmp_path)
        .status()
        .is_ok_and(|s| s.success())
        || Command::new("wget")
            .args(["-qO"])
            .arg(&tmp_path)
            .arg(url)
            .status()
            .is_ok_and(|s| s.success());

    if !success {
        let _ = fs::remove_file(&tmp_path);
        return Err("Не удалось скачать бинарник через curl/wget. Проверьте сеть или наличие релиза.".into());
    }

    let file_bytes = fs::read(&tmp_path)?;
    if file_bytes.len() < 4 || &file_bytes[..4] != b"\x7fELF" {
        let _ = fs::remove_file(&tmp_path);
        return Err("Скачанный файл повреждён или не является бинарником Linux ELF.".into());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&tmp_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&tmp_path, perms)?;
    }

    fs::rename(&tmp_path, &current_exe)?;

    println!("Готово! Программа успешно обновлена до последней версии.");
    if get_running_pid().is_some() {
        println!("Внимание: сейчас запущен демон трекера. Чтобы применить обновление, выполните:");
        println!("    worktrack stop && worktrack start");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shift_state_persistence() {
        let temp_dir = std::env::temp_dir().join("test_worktrack_shift_state");
        let _ = fs::create_dir_all(&temp_dir);

        let cfg = Config::default();
        let today = "2026-11-03";

        let state = ShiftState {
            date: today.to_string(),
            first_start: "09:00:00".to_string(),
            current_session_start: Some("13:00:00".to_string()),
            total_worked_secs: 7200,
        };

        save_shift_state(&temp_dir, &state);

        let (first_start, worked_secs) = load_shift_state(&temp_dir, today, &cfg);
        assert_eq!(first_start, Some("09:00:00".to_string()));
        assert_eq!(worked_secs, 7200);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
