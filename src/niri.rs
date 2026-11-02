use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize, Clone)]
pub struct Window {
    pub id: u64,
    pub title: Option<String>,
    pub app_id: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OutputInfo {
    pub name: String,
    pub make: Option<String>,
    pub model: Option<String>,
}

pub fn get_all_outputs() -> Result<Vec<OutputInfo>, Box<dyn std::error::Error>> {
    let output = Command::new("niri")
        .args(["msg", "-j", "outputs"])
        .output()?;
    if !output.status.success() {
        return Err("niri msg outputs failed".into());
    }

    let map: std::collections::HashMap<String, OutputInfo> = serde_json::from_slice(&output.stdout)?;
    let mut list: Vec<OutputInfo> = map.into_values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(list)
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

pub fn screenshot_focused_window(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("niri")
        .args([
            "msg",
            "action",
            "screenshot-window",
            "--show-pointer",
            "true",
            "--write-to-disk",
            "true",
            "--path",
            path,
        ])
        .status();

    match status {
        Ok(s) if s.success() => Ok(()),
        _ => {
            // Fallback to screen capture if no window is focused
            screenshot_screen(path)
        }
    }
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
