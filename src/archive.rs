use crate::config::FtpConfig;
use flate2::Compression;
use flate2::write::GzEncoder;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn pack_day(root: &Path, day: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let day_dir = root.join(day);
    if !day_dir.exists() {
        return Err(format!("Директория {day_dir:?} не существует").into());
    }

    let archive_path = root.join(format!("{day}.tar.gz"));
    let tmp_archive = root.join(format!("{day}.tar.gz.tmp"));

    let pack_res = (|| -> Result<(), Box<dyn std::error::Error>> {
        let tar_gz = File::create(&tmp_archive)?;
        let enc = GzEncoder::new(tar_gz, Compression::default());
        let mut tar = tar::Builder::new(enc);

        tar.append_dir(day, &day_dir)?;

        if let Ok(entries) = std::fs::read_dir(&day_dir) {
            for entry in entries.flatten() {
                let Ok(file_name) = entry.file_name().into_string() else { continue };
                if file_name.contains(".tmp") {
                    let _ = std::fs::remove_file(entry.path());
                    continue;
                }
                if let Ok(mut f) = File::open(entry.path()) {
                    let _ = tar.append_file(format!("{day}/{file_name}"), &mut f);
                }
            }
        }

        tar.finish()?;
        std::fs::rename(&tmp_archive, &archive_path)?;
        Ok(())
    })();

    if let Err(e) = pack_res {
        let _ = std::fs::remove_file(&tmp_archive);
        return Err(e);
    }

    Ok(archive_path)
}

pub fn upload_archive(
    archive_path: &Path,
    upload_cmd: Option<&str>,
    ftp: &FtpConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(cmd_template) = upload_cmd.filter(|c| !c.trim().is_empty()) {
        let full_path = archive_path.to_string_lossy();
        let replaced_cmd = cmd_template.replace("{file}", &full_path);

        println!("Выполняю команду выгрузки: {replaced_cmd}");
        let status = Command::new("bash").args(["-c", &replaced_cmd]).status()?;

        if status.success() {
            println!("Архив успешно выгружен!");
            return Ok(());
        } else {
            return Err(format!(
                "Команда выгрузки завершилась с ошибкой: {:?}",
                status.code()
            )
            .into());
        }
    }

    if ftp.is_configured() {
        return upload_via_ftp(archive_path, ftp);
    }

    println!(
        "Выгрузка не настроена. Задайте FTP (WORKTRACK_FTP_HOST, WORKTRACK_FTP_USER, WORKTRACK_FTP_PASS) или WORKTRACK_UPLOAD_CMD в ~/.config/worktrack.env.\nАрхив сохранён локально: {:?}",
        archive_path
    );
    Ok(())
}

pub fn upload_via_ftp(
    archive_path: &Path,
    ftp: &FtpConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let file_name = archive_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Некорректное имя архива")?;

    let target_url = ftp
        .target_url(Some(file_name))
        .ok_or("WORKTRACK_FTP_HOST не задан")?;

    println!("Выгрузка архива по FTP на {target_url}...");

    let mut cmd = Command::new("curl");
    cmd.arg("-T")
        .arg(archive_path)
        .args([
            "--ftp-create-dirs",
            "--connect-timeout", "30",
            "--retry", "3",
            "--retry-delay", "2",
            "--speed-limit", "1024",
            "--speed-time", "30",
            "--max-time", "900",
            "--fail",
            "--show-error",
        ]);

    if let Some(user) = &ftp.user {
        let creds = match &ftp.pass {
            Some(pass) => format!("{user}:{pass}"),
            None => user.clone(),
        };
        cmd.args(["--user", &creds]);
    }

    if ftp.secure {
        cmd.arg("--ssl-reqd");
    }

    cmd.arg(&target_url);

    let status = cmd.status()?;
    if status.success() {
        println!("Архив успешно выгружен на FTP-сервер ({target_url})!");
        Ok(())
    } else {
        Err(format!(
            "FTP выгрузка завершилась с ошибкой (curl exit code: {:?})",
            status.code()
        )
        .into())
    }
}
