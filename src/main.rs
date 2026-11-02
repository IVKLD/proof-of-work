mod archive;
mod config;
mod indicator;
mod niri;
mod stamper;

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
    /// Остановить трекер, собрать архив за сегодня и выгрузить
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
    /// Выгрузить архив за сегодняшний день
    Upload,
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

fn is_process_running(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    unsafe { libc::kill(pid, 0) == 0 }
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
    hidden: Option<&Arc<AtomicBool>>,
    target_output_holder: Option<&Arc<std::sync::RwLock<Option<String>>>>,
) {
    let now_stamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let time_str = Local::now().format("%H-%M-%S").to_string();

    let file_name = format!("{time_str}.png");
    let file_path = day_dir.join(&file_name);
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
        let out_name = ws
            .output
            .clone()
            .or_else(|| cfg.output.clone())
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
        let out_name = cfg
            .output
            .clone()
            .or_else(|| niri::get_focused_output().map(|o| o.name))
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

    // 2. Perform capture (briefly hide indicator badge so it does not appear on screenshot)
    if let Some(h) = hidden {
        h.store(true, Ordering::SeqCst);
        let hide_file = runtime_dir().join("worktrack.hide");
        let _ = fs::File::create(&hide_file);
        std::thread::sleep(Duration::from_millis(50));
    }

    let res = niri::capture_screenshot(&file_str, Some(&target_output));

    if let Some(h) = hidden {
        h.store(false, Ordering::SeqCst);
        let hide_file = runtime_dir().join("worktrack.hide");
        let _ = fs::remove_file(&hide_file);
    }

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
        Ok(out_name) => {
            let blocked_rects = if let Some(ws_id) = target_ws_id {
                niri::get_blocked_window_rects_for_workspace(ws_id, &out_name, &cfg.blocked_apps)
            } else {
                niri::get_blocked_window_rects_on_output(&out_name, &cfg.blocked_apps)
            };
            if let Err(err) = stamper::stamp_screenshot(&file_path, &now_stamp, &blocked_rects) {
                append_log(day_dir, &format!("WARN: could not stamp {file_name}: {err}"));
            }
        }
        Err(err) => {
            append_log(day_dir, &format!("ERROR: screenshot failed: {err}"));
        }
    }
}

fn run_daemon(cfg: Config) {
    let pid = std::process::id() as i32;
    let _ = fs::write(pid_file_path(), pid.to_string());

    let running = Arc::new(AtomicBool::new(true));
    let paused = Arc::new(AtomicBool::new(pause_file_path().exists()));
    let hidden = Arc::new(AtomicBool::new(false));

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

    let daily_goal_secs = (cfg.daily_hours * 3600.0).round() as u64;
    let initial_shots = count_screenshots_today(&cfg.dir);
    let mut prior_worked_secs = (initial_shots as u64) * cfg.interval_secs;
    let mut session_worked_secs: u64 = 0;

    let initial_rem_work = if prior_worked_secs >= daily_goal_secs {
        -((prior_worked_secs - daily_goal_secs) as i64)
    } else {
        (daily_goal_secs - prior_worked_secs) as i64
    };

    // Start Wayland indicator thread
    let ind_running = running.clone();
    let ind_paused = paused.clone();
    let ind_hidden = hidden.clone();
    let countdown = Arc::new(std::sync::atomic::AtomicI64::new(cfg.interval_secs as i64));
    let work_remaining = Arc::new(std::sync::atomic::AtomicI64::new(initial_rem_work));
    let ind_countdown = countdown.clone();
    let ind_work_rem = work_remaining.clone();
    let ind_scale = cfg.scale;
    let ind_target_out = target_output.clone();
    let ind_thread = std::thread::spawn(move || {
        indicator::run_indicator(
            ind_paused,
            ind_hidden,
            ind_running,
            ind_countdown,
            ind_work_rem,
            ind_scale,
            ind_target_out,
        );
    });

    let initial_day = Local::now().format("%Y-%m-%d").to_string();
    let initial_dir = cfg.dir.join(&initial_day);
    let _ = fs::create_dir_all(&initial_dir);
    append_log(&initial_dir, "START");

    let mut current_day = initial_day;
    let mut last_capture = std::time::Instant::now()
        .checked_sub(Duration::from_secs(cfg.interval_secs))
        .unwrap_or_else(std::time::Instant::now);

    let mut prev_paused = paused.load(Ordering::Relaxed);
    let capturing = Arc::new(AtomicBool::new(false));
    let mut last_second_tick = std::time::Instant::now();

    while running.load(Ordering::SeqCst) {
        let is_paused = pause_file_path().exists();
        paused.store(is_paused, Ordering::Relaxed);

        let today = Local::now().format("%Y-%m-%d").to_string();
        if today != current_day {
            current_day = today.clone();
            let day_dir = cfg.dir.join(&today);
            let _ = fs::create_dir_all(&day_dir);
            prior_worked_secs = 0;
            session_worked_secs = 0;
        }

        if prev_paused != is_paused {
            prev_paused = is_paused;
            let day_dir = cfg.dir.join(&today);
            let _ = fs::create_dir_all(&day_dir);
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
                session_worked_secs += elapsed_ticks;
            }

            let total_worked = prior_worked_secs + session_worked_secs;
            let rem_work = if total_worked >= daily_goal_secs {
                -((total_worked - daily_goal_secs) as i64)
            } else {
                (daily_goal_secs - total_worked) as i64
            };
            work_remaining.store(rem_work, Ordering::Relaxed);

            let stats_path = runtime_dir().join("worktrack.stats");
            let _ = fs::write(
                stats_path,
                format!("{prior_worked_secs}\n{session_worked_secs}\n{daily_goal_secs}\n{rem_work}"),
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
            let cap_hidden = hidden.clone();
            let cap_target = target_output.clone();
            let cap_flag = capturing.clone();

            std::thread::spawn(move || {
                capture_screen(&day_dir, &cap_cfg, Some(&cap_hidden), Some(&cap_target));
                cap_flag.store(false, Ordering::SeqCst);
            });
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    let today = Local::now().format("%Y-%m-%d").to_string();
    let day_dir = cfg.dir.join(&today);
    append_log(&day_dir, "STOP");

    let _ = fs::remove_file(runtime_dir().join("worktrack.stats"));
    let _ = fs::remove_file(pid_file_path());
    let _ = fs::remove_file(pause_file_path());
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
    println!("Скриншотов за сегодня: {count}");

    let daily_goal_secs = (cfg.daily_hours * 3600.0).round() as u64;

    let (prior_secs, session_secs, total_worked_secs, rem_secs) = if is_running {
        let stats_path = runtime_dir().join("worktrack.stats");
        if let Ok(content) = fs::read_to_string(&stats_path) {
            let lines: Vec<&str> = content.lines().collect();
            if lines.len() >= 4 {
                let prior = lines[0].parse::<u64>().unwrap_or(0);
                let session = lines[1].parse::<u64>().unwrap_or(0);
                let _goal = lines[2].parse::<u64>().unwrap_or(daily_goal_secs);
                let rem = lines[3].parse::<i64>().unwrap_or(0);
                (prior, session, prior + session, rem)
            } else {
                let prior = (count as u64) * cfg.interval_secs;
                let total = prior;
                let rem = (daily_goal_secs as i64) - (total as i64);
                (prior, 0, total, rem)
            }
        } else {
            let prior = (count as u64) * cfg.interval_secs;
            let total = prior;
            let rem = (daily_goal_secs as i64) - (total as i64);
            (prior, 0, total, rem)
        }
    } else {
        let prior = (count as u64) * cfg.interval_secs;
        let total = prior;
        let rem = (daily_goal_secs as i64) - (total as i64);
        (prior, 0, total, rem)
    };

    let goal_str = format_duration_hm(daily_goal_secs);
    let total_str = format_duration_hm(total_worked_secs);
    let percent = ((total_worked_secs as f64 / daily_goal_secs as f64) * 100.0).min(100.0);

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
        println!("Осталось отработать сегодня: {}", format_duration_hm(rem_secs as u64));
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
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "png"))
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
                "\nДоступные команды: worktrack <start|shot|pause|resume|status|workspaces|windows|stop|pack|upload|completions>"
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
            println!("Трекер запущен (PID {})", child.id());
        }

        Some(Commands::Shot) => {
            let today = Local::now().format("%Y-%m-%d").to_string();
            let day_dir = cfg.dir.join(&today);
            let _ = fs::create_dir_all(&day_dir);
            capture_screen(&day_dir, &cfg, None, None);
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

            let today = Local::now().format("%Y-%m-%d").to_string();
            match archive::pack_day(&cfg.dir, &today) {
                Ok(archive_path) => {
                    let count = count_screenshots_today(&cfg.dir);
                    println!("Архив готов: {archive_path:?} ({count} скриншотов)");
                    let _ = archive::upload_archive(&archive_path, cfg.upload_cmd.as_deref());
                }
                Err(err) => {
                    println!("Предупреждение: не удалось собрать архив: {err}");
                }
            }
        }

        Some(Commands::Pack) => {
            let today = Local::now().format("%Y-%m-%d").to_string();
            let archive_path = archive::pack_day(&cfg.dir, &today)?;
            let count = count_screenshots_today(&cfg.dir);
            println!("Архив собран: {archive_path:?} ({count} скриншотов)");
            let _ = archive::upload_archive(&archive_path, cfg.upload_cmd.as_deref());
        }

        Some(Commands::Upload) => {
            let today = Local::now().format("%Y-%m-%d").to_string();
            let archive_path = cfg.dir.join(format!("{today}.tar.gz"));
            if !archive_path.exists() {
                return Err(format!(
                    "Архив {archive_path:?} не найден. Сначала выполните `worktrack pack`"
                )
                .into());
            }
            archive::upload_archive(&archive_path, cfg.upload_cmd.as_deref())?;
        }
    }

    Ok(())
}
