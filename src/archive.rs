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
                let path = entry.path();
                if let Ok(file_name) = entry.file_name().into_string() {
                    if file_name.contains(".tmp") {
                        continue;
                    }
                    let archive_name = format!("{day}/{file_name}");
                    if let Ok(mut f) = File::open(&path) {
                        let _ = tar.append_file(&archive_name, &mut f);
                    }
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
        let status = Command::new("bash").arg("-c").arg(&replaced_cmd).status()?;

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

    let raw_host = ftp.host.as_deref().ok_or("WORKTRACK_FTP_HOST не задан")?;
    let host = raw_host
        .trim()
        .trim_start_matches("ftps://")
        .trim_start_matches("ftp://")
        .trim_end_matches('/')
        .trim();
    let port = ftp.port;
    let proto = if ftp.secure { "ftps" } else { "ftp" };
    let remote_dir = ftp
        .remote_dir
        .as_deref()
        .unwrap_or("")
        .trim()
        .trim_matches('/')
        .to_string();

    let target_url = if remote_dir.is_empty() {
        format!("{proto}://{host}:{port}/{file_name}")
    } else {
        format!("{proto}://{host}:{port}/{remote_dir}/{file_name}")
    };

    println!("Выгрузка архива по FTP на {target_url}...");

    let mut cmd = Command::new("curl");
    cmd.arg("-T")
        .arg(archive_path)
        .arg("--ftp-create-dirs")
        .arg("--connect-timeout")
        .arg("30")
        .arg("--retry")
        .arg("3")
        .arg("--retry-delay")
        .arg("2")
        .arg("--speed-limit")
        .arg("1024")
        .arg("--speed-time")
        .arg("30")
        .arg("--max-time")
        .arg("900")
        .arg("--fail")
        .arg("--show-error");

    if let (Some(user), Some(pass)) = (ftp.user.as_deref(), ftp.pass.as_deref()) {
        cmd.arg("--user").arg(format!("{user}:{pass}"));
    } else if let Some(user) = ftp.user.as_deref() {
        cmd.arg("--user").arg(user);
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
