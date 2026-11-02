use serde::Deserialize;
use std::io::Write;
use std::process::{Command, Stdio};

struct ClipboardBackup {
    mime_type: Option<String>,
    data: Vec<u8>,
    had_copyq: bool,
}

impl ClipboardBackup {
    fn capture() -> Self {
        let had_copyq = Command::new("copyq")
            .arg("disable")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        let mime = Command::new("wl-paste")
            .arg("--list-types")
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    String::from_utf8(out.stdout).ok().and_then(|s| {
                        s.lines()
                            .map(|l| l.trim().to_string())
                            .find(|l| !l.is_empty() && !l.starts_with("image/"))
                            .or_else(|| s.lines().next().map(|l| l.trim().to_string()))
                    })
                } else {
                    None
                }
            });

        let data = if let Some(m) = &mime {
            Command::new("wl-paste")
                .args(["--no-newline", "--type", m])
                .output()
                .ok()
                .map(|out| out.stdout)
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        Self {
            mime_type: mime,
            data,
            had_copyq,
        }
    }

    fn restore(self) {
        // Give Niri compositor 200ms to complete its async clipboard write before we restore
        std::thread::sleep(std::time::Duration::from_millis(200));

        if let Some(m) = self.mime_type {
            if !self.data.is_empty() {
                if let Ok(mut child) = Command::new("wl-copy")
                    .args(["--type", &m])
                    .stdin(Stdio::piped())
                    .spawn()
                {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(&self.data);
                    }
                    let _ = child.wait();
                }
            } else {
                let _ = Command::new("wl-copy").arg("--clear").status();
            }
        } else {
            let _ = Command::new("wl-copy").arg("--clear").status();
        }

        if self.had_copyq {
            let _ = Command::new("copyq").arg("enable").status();
            let _ = Command::new("copyq").args(["select", "0"]).status();
        }
    }
}

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
    let backup = ClipboardBackup::capture();
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
    backup.restore();

    match status {
        Ok(s) if s.success() => Ok(()),
        _ => {
            // Fallback to screen capture if no window is focused
            screenshot_screen(path)
        }
    }
}

pub fn screenshot_screen(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let backup = ClipboardBackup::capture();
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
    backup.restore();

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
