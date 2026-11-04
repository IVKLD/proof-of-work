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
}

fn runtime_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(dir)
    } else {
        PathBuf::from("/tmp")
    }
}

fn pid_file_path() -> PathBuf {
    runtime_dir().join("worktrack.pid")
}

fn pause_file_path() -> PathBuf {
    runtime_dir().join("worktrack.paused")
}

fn reset_file_path() -> PathBuf {
    runtime_dir().join("worktrack.reset")
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
    let path = pid_file_path();
    let content = fs::read_to_string(&path).ok()?;
    let pid = content.trim().parse::<i32>().ok()?;
    if is_process_running(pid) {
        Some(pid)
    } else {
        None
    }
}

fn append_log(day_dir: &Path, msg: &str) {
    let log_path = day_dir.join("events.log");
    let now = Local::now().format("%H:%M:%S");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = writeln!(f, "{now} {msg}");
    }
}

fn capture_screen(
    day_dir: &Path,
    cfg: &Config,
    target_output_holder: Option<&Arc<std::sync::RwLock<Option<String>>>>,
) {
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
    let target_ws = cfg
        .workspace
        .as_deref()
        .and_then(|q| niri::find_workspace(&all_workspaces, q, cfg.output.as_deref()));

    let (
        target_output,
        target_ws_id,
        needs_restore,
        prev_win_id,
        prev_active_ws,
        prev_focused_output,
    ) = if let Some(ref ws) = target_ws {
        let all_outputs = niri::get_all_outputs().unwrap_or_default();
        let out_name = ws
            .output
            .clone()
            .or_else(|| cfg.output.clone())
            .filter(|o| all_outputs.iter().any(|ao| ao.name == *o))
            .or_else(|| niri::get_focused_output().map(|o| o.name))
            .or_else(|| all_outputs.first().map(|o| o.name.clone()))
            .unwrap_or_else(|| "DP-1".to_string());

        if ws.is_active {
            // Workspace is already visible on its output: no switching needed!
            (out_name, Some(ws.id), false, None, None, None)
        } else {
            // Workspace is inactive: switch to it temporarily
            let prev_win = niri::get_focused_window_id();
            let prev_active = all_workspaces
                .iter()
                .find(|w| w.output.as_deref() == Some(&out_name) && w.is_active)
                .cloned();
            let prev_focused_out = niri::get_focused_output().map(|o| o.name);

            // Switch to workspace
            let _ = niri::focus_monitor(&out_name);
            if let Some(ref name) = ws.name {
                let _ = niri::focus_workspace(name);
            } else if let Some(idx) = ws.idx {
                let _ = niri::focus_workspace(&idx.to_string());
            }
            // Settle time for workspace transition to complete before screenshot
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
        let all_outputs = niri::get_all_outputs().unwrap_or_default();
        let out_name = cfg
            .output
            .clone()
            .filter(|o| all_outputs.iter().any(|ao| ao.name == *o))
            .or_else(|| niri::get_focused_output().map(|o| o.name))
            .or_else(|| all_outputs.first().map(|o| o.name.clone()))
            .unwrap_or_else(|| "DP-1".to_string());
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
        let mut restored = false;
        if let Some(win_id) = prev_win_id
            && niri::focus_window(win_id).is_ok()
        {
            restored = true;
        }
        if !restored {
            if let Some(prev_ws) = prev_active_ws {
                if let Some(ref name) = prev_ws.name {
                    let _ = niri::focus_workspace(name);
                } else if let Some(idx) = prev_ws.idx {
                    if let Some(ref out) = prev_ws.output {
                        let _ = niri::focus_monitor(out);
                    }
                    let _ = niri::focus_workspace(&idx.to_string());
                } else {
                    let _ = niri::focus_workspace_previous();
                }
                if let Some(ref prev_out) = prev_focused_output {
                    let _ = niri::focus_monitor(prev_out);
                }
            } else {
                let _ = niri::focus_workspace_previous();
            }
        }
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
        }
        Err(err) => {
            append_log(day_dir, &format!("ERROR: screenshot failed: {err}"));
            let _ = fs::remove_file(&file_path);
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

fn shift_state_path(day_dir: &Path) -> PathBuf {
    day_dir.join("shift.json")
}

fn get_first_screenshot_time(dir: &Path, today: &str) -> Option<String> {
    let day_dir = dir.join(today);
    let mut shots: Vec<String> = fs::read_dir(day_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "png" || ext == "jpg" || ext == "jpeg")
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    shots.sort();
    for shot in shots {
        if let Some(stem) = Path::new(&shot).file_stem().and_then(|s| s.to_str()) {
            let clean_stem = stem.split('_').next().unwrap_or(stem);
            let parts: Vec<&str> = clean_stem.split('-').collect();
            if parts.len() == 3
                && parts[0].chars().all(|c| c.is_ascii_digit())
                && parts[1].chars().all(|c| c.is_ascii_digit())
                && parts[2].chars().all(|c| c.is_ascii_digit())
            {
                return Some(format!("{}:{}:{}", parts[0], parts[1], parts[2]));
            }
        }
    }
    None
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
    let lock_path = runtime_dir().join("worktrack.lock");
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
    let _ = fs::write(pid_file_path(), pid.to_string());

    let running = Arc::new(AtomicBool::new(true));
    let paused = Arc::new(AtomicBool::new(pause_file_path().exists()));

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
        let reset_file = reset_file_path();
        if reset_file.exists() {
            let _ = fs::remove_file(&reset_file);
            prior_worked_secs = 0;
            session_worked_secs = 0;
            let now_s = Local::now().format("%H:%M:%S").to_string();
            first_start_time = cfg.start_time.clone().unwrap_or_else(|| now_s.clone());
            session_start_time = now_s.clone();
            current_shift = ShiftState {
                date: current_day.clone(),
                first_start: first_start_time.clone(),
                current_session_start: Some(now_s.clone()),
                total_worked_secs: 0,
            };
            save_shift_state(&cfg.dir.join(&current_day), &current_shift);
            work_remaining.store(daily_goal_secs as i64, Ordering::Relaxed);
            let stats_path = runtime_dir().join("worktrack.stats");
            let _ = fs::write(
                stats_path,
                format!("0\n0\n{daily_goal_secs}\n{daily_goal_secs}\n{first_start_time}\n{now_s}"),
            );
            append_log(&cfg.dir.join(&current_day), "RESET");
        }

        let is_paused = pause_file_path().exists();
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
            first_start_time = cfg.start_time.clone().unwrap_or_else(|| now_s.clone());
            session_start_time = now_s.clone();
            current_shift = ShiftState {
                date: today.clone(),
                first_start: first_start_time.clone(),
                current_session_start: Some(now_s),
                total_worked_secs: 0,
            };
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

            let stats_path = runtime_dir().join("worktrack.stats");
            let _ = fs::write(
                stats_path,
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
                    capture_screen(&day_dir, &cap_cfg, Some(&cap_target));
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

    let _ = fs::remove_file(runtime_dir().join("worktrack.stats"));
    let _ = fs::remove_file(pid_file_path());
    let _ = fs::remove_file(pause_file_path());
    let _ = fs::remove_file(reset_file_path());
    let _ = ind_thread.join();
}

fn format_duration_hm(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h} ч {m:02} мин")
    } else if m > 0 {
        format!("{m} мин")
    } else {
        format!("{s} сек")
    }
}

fn format_duration_hms(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
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

fn get_today_worked_secs(cfg: &Config) -> (u64, usize) {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let day_dir = cfg.dir.join(&today);
    let count = count_screenshots_today(&cfg.dir);

    let is_running = get_running_pid().is_some();
    if is_running {
        let stats_path = runtime_dir().join("worktrack.stats");
        if let Ok(content) = fs::read_to_string(&stats_path) {
            let lines: Vec<&str> = content.lines().collect();
            if lines.len() >= 2 {
                let prior = lines[0].parse::<u64>().unwrap_or(0);
                let session = lines[1].parse::<u64>().unwrap_or(0);
                return (prior + session, count);
            }
        }
    }
    let (_, prior) = load_shift_state(&day_dir, &today, cfg);
    (prior, count)
}

fn print_status(cfg: &Config) {
    let pid = get_running_pid();
    let is_running = pid.is_some();
    match pid {
        Some(p) => {
            let paused = pause_file_path().exists();
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
        if is_running {
            let stats_path = runtime_dir().join("worktrack.stats");
            if let Ok(content) = fs::read_to_string(&stats_path) {
                let lines: Vec<&str> = content.lines().collect();
                if lines.len() >= 4 {
                    let prior = lines[0].parse::<u64>().unwrap_or(0);
                    let session = lines[1].parse::<u64>().unwrap_or(0);
                    let _goal = lines[2].parse::<u64>().unwrap_or(daily_goal_secs);
                    let rem = lines[3].parse::<i64>().unwrap_or(0);
                    let first_s = lines.get(4).filter(|s| !s.is_empty()).map(|s| s.to_string());
                    let sess_s = lines.get(5).filter(|s| !s.is_empty()).map(|s| s.to_string());
                    (prior, session, prior + session, rem, first_s, sess_s)
                } else {
                    let (first_s, prior) = load_shift_state(&day_dir, &today, cfg);
                    let rem = (daily_goal_secs as i64) - (prior as i64);
                    (prior, 0, prior, rem, first_s, None)
                }
            } else {
                let (first_s, prior) = load_shift_state(&day_dir, &today, cfg);
                let rem = (daily_goal_secs as i64) - (prior as i64);
                (prior, 0, prior, rem, first_s, None)
            }
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
    let workspaces = match niri::get_all_workspaces() {
        Ok(w) => w,
        Err(e) => {
            println!("Ошибка получения рабочих пространств niri: {e}");
            return;
        }
    };
    let windows = niri::get_all_windows().unwrap_or_default();

    println!("Рабочие пространства niri (workspaces):");
    println!("{}", "-".repeat(70));
    for ws in &workspaces {
        let is_active_str = if ws.is_active { " [активен]" } else { "" };
        let is_focused_str = if ws.is_focused { " [фокус]" } else { "" };
        let name_str = ws.name.as_deref().unwrap_or("<без имени>");
        let out_str = ws.output.as_deref().unwrap_or("?");
        let idx_str = ws
            .idx
            .map(|i| i.to_string())
            .unwrap_or_else(|| "?".to_string());

        let ws_wins: Vec<&niri::Window> = windows
            .iter()
            .filter(|w| w.workspace_id == Some(ws.id))
            .collect();
        let win_count = ws_wins.len();

        let is_cfg_match = cfg
            .workspace
            .as_deref()
            .is_some_and(|q| {
                let name_match = ws
                    .name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(q));
                let idx_match = ws.idx.is_some_and(|i| q == i.to_string());
                let id_match = q == format!("id:{}", ws.id);
                name_match || idx_match || id_match
            });

        let match_tag = if is_cfg_match {
            " <== WORKTRACK_WORKSPACE"
        } else {
            ""
        };

        println!(
            "  * id: {:<2} idx: {:<2} output: {:<5} name: {:<15} окон: {:<2}{}{}{}",
            ws.id, idx_str, out_str, name_str, win_count, is_active_str, is_focused_str, match_tag
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
        let matched = niri::find_workspace(&workspaces, target, cfg.output.as_deref());
        match matched {
            Some(w) => {
                let name = w.name.as_deref().unwrap_or("<без имени>");
                let out = w.output.as_deref().unwrap_or("?");
                println!(
                    "Настроен воркспейс: WORKTRACK_WORKSPACE=\"{target}\" (найден: \"{name}\" на {out})"
                );
            }
            None => {
                println!(
                    "Настроен воркспейс: WORKTRACK_WORKSPACE=\"{target}\" (ВНИМАНИЕ: не найден в niri!)"
                );
            }
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
    let day_dir = dir.join(today);
    if let Ok(entries) = fs::read_dir(day_dir) {
        entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "png" || ext == "jpg" || ext == "jpeg")
            })
            .count()
    } else {
        0
    }
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

            let _ = fs::remove_file(pause_file_path());

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
            let _ = fs::write(pid_file_path(), child.id().to_string());
            println!("Трекер запущен (PID {})", child.id());
        }

        Some(Commands::Shot) => {
            let today = Local::now().format("%Y-%m-%d").to_string();
            let day_dir = cfg.dir.join(&today);
            let _ = fs::create_dir_all(&day_dir);
            capture_screen(&day_dir, &cfg, None);
            let count = count_screenshots_today(&cfg.dir);
            println!("Снимок сохранён в {day_dir:?} (всего за день: {count})");
        }

        Some(Commands::Daemon) => {
            run_daemon(cfg);
        }

        Some(Commands::Pause) => {
            if get_running_pid().is_none() {
                println!("Трекер не запущен");
                return Ok(());
            }
            let _ = fs::File::create(pause_file_path());
            println!("Трекер поставлен на паузу (индикатор жёлтый)");
        }

        Some(Commands::Resume) => {
            if get_running_pid().is_none() {
                println!("Трекер не запущен");
                return Ok(());
            }
            let _ = fs::remove_file(pause_file_path());
            println!("Трекер снят с паузы (индикатор красный)");
        }

        Some(Commands::Reset { yes }) => {
            let today = Local::now().format("%Y-%m-%d").to_string();
            let day_dir = cfg.dir.join(&today);
            let _ = fs::create_dir_all(&day_dir);

            if !yes {
                println!("Внимание: сброс рабочей смены за сегодня ({today}).");
                println!("Накопленное время и таймер рабочего дня будут обнулены.");
                print!("Вы уверены, что хотите сбросить смену? [y/N]: ");
                std::io::stdout().flush()?;
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                let ans = input.trim().to_lowercase();
                if ans != "y" && ans != "yes" && ans != "д" && ans != "да" {
                    println!("Сброс отменён.");
                    return Ok(());
                }
            }

            if get_running_pid().is_some() {
                let reset_file = reset_file_path();
                let _ = fs::File::create(&reset_file);
                for _ in 0..10 {
                    if !reset_file.exists() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                println!("Рабочая смена успешно сброшена! Счётчик обнулён.");
            } else {
                let shift = ShiftState {
                    date: today.clone(),
                    first_start: cfg
                        .start_time
                        .clone()
                        .unwrap_or_else(|| Local::now().format("%H:%M:%S").to_string()),
                    current_session_start: None,
                    total_worked_secs: 0,
                };
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

            let _ = fs::remove_file(pause_file_path());
            let _ = fs::remove_file(pid_file_path());
            let _ = fs::remove_file(reset_file_path());

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
                    let size_str = fs::metadata(&archive_path)
                        .map(|m| format!(" ({})", format_file_size(m.len())))
                        .unwrap_or_default();
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
            let size_str = fs::metadata(&archive_path)
                .map(|m| format!(" ({})", format_file_size(m.len())))
                .unwrap_or_default();
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

            let destination_info = if let Some(ref raw_host) = cfg.ftp.host {
                let host = raw_host
                    .trim()
                    .trim_start_matches("ftps://")
                    .trim_start_matches("ftp://")
                    .trim_end_matches('/')
                    .trim();
                let proto = if cfg.ftp.secure { "ftps" } else { "ftp" };
                let port = cfg.ftp.port;
                let user_prefix = cfg
                    .ftp
                    .user
                    .as_deref()
                    .map(|u| format!("{u}@"))
                    .unwrap_or_default();
                let rdir = cfg.ftp.remote_dir.as_deref().unwrap_or("").trim().trim_matches('/');
                if rdir.is_empty() {
                    format!("{proto}://{user_prefix}{host}:{port}/")
                } else {
                    format!("{proto}://{user_prefix}{host}:{port}/{rdir}/")
                }
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

            if !yes {
                print!("Отправить архив работодателю на FTP? [y/N]: ");
                std::io::stdout().flush()?;
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                let ans = input.trim().to_lowercase();
                if ans != "y" && ans != "yes" && ans != "д" && ans != "да" {
                    println!("Отправка отменена.");
                    return Ok(());
                }
            }

            archive::upload_archive(&archive_path, cfg.upload_cmd.as_deref(), &cfg.ftp)?;
        }
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
