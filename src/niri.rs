use serde::Deserialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct LogicalOutput {
    pub x: i64,
    pub y: i64,
    pub width: u64,
    pub height: u64,
    pub scale: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct OutputInfo {
    pub name: String,
    pub make: Option<String>,
    pub model: Option<String>,
    pub logical: Option<LogicalOutput>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct WindowLayout {
    pub tile_size: Option<[f64; 2]>,
    pub window_size: Option<[u64; 2]>,
    pub tile_pos_in_workspace_view: Option<[f64; 2]>,
    pub window_offset_in_tile: Option<[f64; 2]>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct FocusedWindow {
    pub id: u64,
    pub title: Option<String>,
    pub app_id: Option<String>,
    pub workspace_id: Option<u64>,
    pub is_floating: bool,
    pub layout: Option<WindowLayout>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Window {
    pub id: u64,
    pub title: Option<String>,
    pub app_id: Option<String>,
}

pub fn get_focused_output() -> Option<OutputInfo> {
    let output = Command::new("niri")
        .args(["msg", "-j", "focused-output"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

pub fn get_focused_window() -> Option<FocusedWindow> {
    let output = Command::new("niri")
        .args(["msg", "-j", "focused-window"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

pub fn get_all_outputs() -> Result<Vec<OutputInfo>, Box<dyn std::error::Error>> {
    let output = Command::new("niri")
        .args(["msg", "-j", "outputs"])
        .output()?;
    if !output.status.success() {
        return Err("niri msg outputs failed".into());
    }

    let map: HashMap<String, OutputInfo> = serde_json::from_slice(&output.stdout)?;
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

fn grim_bin() -> String {
    if Command::new("grim").arg("-h").output().is_ok() {
        return "grim".to_string();
    }
    if let Ok(entries) = std::fs::read_dir("/nix/store") {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.contains("-grim-") {
                let bin = path.join("bin/grim");
                if bin.is_file() {
                    return bin.to_string_lossy().to_string();
                }
            }
        }
    }
    "grim".to_string()
}

#[allow(clippy::collapsible_if)]
fn restore_clipboard(prev_mime: Option<String>, prev_data: Option<Vec<u8>>) {
    // 1. Poll copyq for up to 250ms to see if niri placed an image in row 0.
    // If found, delete it from history and reselect previous item.
    // CopyQ is NEVER disabled, user copy operations remain 100% active.
    for _ in 0..12 {
        std::thread::sleep(Duration::from_millis(20));
        let out = Command::new("copyq")
            .args([
                "eval",
                "if (read('image/png', 0).length > 0) { remove(0); select(0); 'CLEANED'; } else { 'WAIT'; }",
            ])
            .output();
        if let Ok(o) = out {
            if String::from_utf8_lossy(&o.stdout).trim() == "CLEANED" {
                break;
            }
        }
    }

    // 2. If wl-paste still has image/png (e.g. copyq not running or not used),
    // restore the user's prior clipboard selection or clear it.
    let current_mime = Command::new("wl-paste")
        .arg("-l")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();

    if current_mime.contains("image/png") {
        if let (Some(mime), Some(data)) = (prev_mime, prev_data) {
            if let Ok(mut child) = Command::new("wl-copy")
                .args(["--type", &mime])
                .stdin(Stdio::piped())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(&data);
                }
                let _ = child.wait();
            }
        } else {
            let _ = Command::new("wl-copy").arg("--clear").status();
        }
    }
}

/// Captures a screenshot of the current work.
///
/// First attempts window surface capture via niri action screenshot-window to guarantee:
/// - 0% wallpaper (mpvpaper / desktop background)
/// - 0% desktop gaps
/// - 0% layer-shell indicators
///
/// Immediately cleans up clipboard and copyq so user's clipboard is NEVER overwritten or disabled.
#[allow(clippy::collapsible_if)]
pub fn capture_screenshot(
    path: &str,
    target_output: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Prefer focused window surface capture (no wallpaper, no desktop gaps, no indicator)
    if let Some(win) = get_focused_window() {
        let prev_mime = Command::new("wl-paste")
            .arg("-l")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.lines().next().unwrap_or("").to_string())
            .filter(|s| !s.is_empty());

        let prev_data = if let Some(ref mime) = prev_mime {
            Command::new("wl-paste")
                .args(["--type", mime, "--no-newline"])
                .output()
                .ok()
                .filter(|o| o.status.success() && !o.stdout.is_empty())
                .map(|o| o.stdout)
        } else {
            None
        };

        let status = Command::new("niri")
            .args([
                "msg",
                "action",
                "screenshot-window",
                "--id",
                &win.id.to_string(),
                "--write-to-disk",
                "true",
                "--path",
                path,
            ])
            .status();

        restore_clipboard(prev_mime, prev_data);

        if let Ok(s) = status {
            if s.success() {
                let p = Path::new(path);
                for _ in 0..25 {
                    if p.is_file() && std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false) {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                if p.is_file() {
                    return Ok(());
                }
            }
        }
    }

    // 2. Fallback to capturing the focused or targeted output via grim
    let grim = grim_bin();
    let out_name = target_output
        .map(|s| s.to_string())
        .or_else(|| get_focused_output().map(|o| o.name));

    let mut cmd = Command::new(&grim);
    cmd.arg("-c");
    if let Some(name) = &out_name {
        cmd.args(["-o", name]);
    }
    cmd.arg(path);

    let status = cmd.status()?;
    if status.success() {
        Ok(())
    } else {
        Err("grim failed to capture screenshot".into())
    }
}
