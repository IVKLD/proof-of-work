use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize, Clone)]
pub struct Window {
    pub id: u64,
    pub title: Option<String>,
    pub app_id: Option<String>,
}

pub fn get_all_windows() -> Result<Vec<Window>, Box<dyn std::error::Error>> {
    let output = Command::new("niri")
        .args(["msg", "-j", "windows"])
        .output()?;
    if !output.status.success() {
        return Err("niri msg windows failed".into());
    }

    let windows: Vec<Window> = serde_json::from_slice(&output.stdout)?;
    Ok(windows)
}

pub fn screenshot_screen(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("niri")
        .args([
            "msg",
            "action",
            "screenshot-screen",
            "--write-to-disk",
            "true",
            "--path",
            path,
        ])
        .status();

    match status {
        Ok(s) if s.success() => Ok(()),
        _ => {
            // Fallback to grim if niri command fails
            let g_status = Command::new("grim").arg(path).status()?;
            if g_status.success() {
                Ok(())
            } else {
                Err("failed to take screen screenshot with niri or grim".into())
            }
        }
    }
}
