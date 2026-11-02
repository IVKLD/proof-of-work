use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use flate2::write::GzEncoder;
use flate2::Compression;

pub fn pack_day(root: &Path, day: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let day_dir = root.join(day);
    if !day_dir.exists() {
        return Err(format!("Директория {day_dir:?} не существует").into());
    }

    let archive_path = root.join(format!("{day}.tar.gz"));
    let tar_gz = File::create(&archive_path)?;
    let enc = GzEncoder::new(tar_gz, Compression::default());
    let mut tar = tar::Builder::new(enc);

    tar.append_dir_all(day, &day_dir)?;
    tar.finish()?;

    Ok(archive_path)
}

pub fn upload_archive(archive_path: &Path, upload_cmd: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let cmd_template = match upload_cmd {
        Some(c) if !c.trim().is_empty() => c,
        _ => {
            println!(
                "Выгрузка не настроена (задайте WORKTRACK_UPLOAD_CMD в ~/.config/worktrack.env). Архив: {:?}",
                archive_path
            );
            return Ok(());
        }
    };

    let full_path = archive_path.to_string_lossy();
    let replaced_cmd = cmd_template.replace("{file}", &full_path);

    println!("Выполняю команду выгрузки: {replaced_cmd}");
    let status = Command::new("bash")
        .arg("-c")
        .arg(&replaced_cmd)
        .status()?;

    if status.success() {
        println!("Архив успешно выгружен!");
        Ok(())
    } else {
        Err(format!("Команда выгрузки завершилась с ошибкой: {:?}", status.code()).into())
    }
}
