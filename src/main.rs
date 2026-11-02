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

fn capture_screen(day_dir: &Path, output: Option<&str>) {
    let now_stamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let time_str = Local::now().format("%H-%M-%S").to_string();

    let file_name = format!("{time_str}.png");
    let file_path = day_dir.join(&file_name);
    let file_str = file_path.to_string_lossy().to_string();

    if let Some(out) = output {
        let _ = Command::new("niri")
            .args(["msg", "action", "focus-monitor", out])
            .status();
    }

    let res = niri::screenshot_focused_window(&file_str).is_ok();

    if res {
        // Wait up to 1500ms for compositor to finish writing PNG to disk
        let mut stamped = false;
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(50));
            if fs::metadata(&file_path).is_ok_and(|m| m.len() > 100)
                && stamper::stamp_screenshot(&file_path, &now_stamp).is_ok()
            {
                stamped = true;
                break;
            }
        }
        if !stamped {
            append_log(day_dir, &format!("WARN: could not stamp {file_name}"));
        }
    } else {
        append_log(day_dir, "ERROR: failed to take screen screenshot");
    }
}

fn run_daemon(cfg: Config) {
    let pid = std::process::id() as i32;
    let _ = fs::write(pid_file_path(), pid.to_string());

    let running = Arc::new(AtomicBool::new(true));
    let paused = Arc::new(AtomicBool::new(pause_file_path().exists()));

    // Hook signals
    let r_sig = running.clone();
    let _ = ctrlc::set_handler(move || {
        r_sig.store(false, Ordering::SeqCst);
    });

    // Start Wayland indicator thread
    let ind_running = running.clone();
    let ind_paused = paused.clone();
    let ind_thread = std::thread::spawn(move || {
        indicator::run_indicator(ind_paused, ind_running);
    });

    let initial_day = Local::now().format("%Y-%m-%d").to_string();
    let initial_dir = cfg.dir.join(&initial_day);
    let _ = fs::create_dir_all(&initial_dir);
    append_log(&initial_dir, "START");

    let mut last_capture = std::time::Instant::now()
        .checked_sub(Duration::from_secs(cfg.interval_secs))
        .unwrap_or_else(std::time::Instant::now);

    let mut prev_paused = paused.load(Ordering::Relaxed);

    while running.load(Ordering::SeqCst) {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let day_dir = cfg.dir.join(&today);
        let _ = fs::create_dir_all(&day_dir);

        let is_paused = pause_file_path().exists();
        paused.store(is_paused, Ordering::Relaxed);

        if prev_paused != is_paused {
            prev_paused = is_paused;
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

        if !is_paused && last_capture.elapsed() >= Duration::from_secs(cfg.interval_secs) {
            last_capture = std::time::Instant::now();
            capture_screen(&day_dir, cfg.output.as_deref());
        }

        std::thread::sleep(Duration::from_millis(250));
    }

    let today = Local::now().format("%Y-%m-%d").to_string();
    let day_dir = cfg.dir.join(&today);
    append_log(&day_dir, "STOP");

    let _ = fs::remove_file(pid_file_path());
    let _ = fs::remove_file(pause_file_path());
    let _ = ind_thread.join();
}

fn print_status(cfg: &Config) {
    let pid = get_running_pid();
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
                "\nДоступные команды: worktrack <start|shot|pause|resume|status|windows|stop|pack|upload|completions>"
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
            capture_screen(&day_dir, cfg.output.as_deref());
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
