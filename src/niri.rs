use serde::Deserialize;
use std::collections::HashMap;
use std::process::Command;

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

/// Captures a screenshot purely via grim (Wayland zwlr_screencopy_manager_v1).
/// NEVER touches or uses the Wayland clipboard under any circumstance.
#[allow(clippy::collapsible_if)]
pub fn capture_screenshot(
    path: &str,
    target_output: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let grim = grim_bin();

    // 1. Try to crop to focused window geometry (no wallpapers, no background)
    if let (Some(win), Some(out)) = (get_focused_window(), get_focused_output()) {
        if let (Some(layout), Some(logical)) = (win.layout, out.logical) {
            if let Some(wsize) = layout.window_size {
                let (w, h) = (wsize[0], wsize[1]);
                let (x, y) = if win.is_floating {
                    if let Some(pos) = layout.tile_pos_in_workspace_view {
                        (logical.x + pos[0] as i64, logical.y + pos[1] as i64)
                    } else {
                        (logical.x, logical.y)
                    }
                } else if let Some(tsize) = layout.tile_size {
                    let gap_x = ((logical.width as f64 - tsize[0]) / 2.0).max(0.0) as i64;
                    let gap_y = ((logical.height as f64 - tsize[1]) / 2.0).max(0.0) as i64;
                    (logical.x + gap_x, logical.y + gap_y)
                } else {
                    (logical.x, logical.y)
                };

                let geom = format!("{x},{y} {w}x{h}");
                let status = Command::new(&grim)
                    .args(["-c", "-g", &geom, path])
                    .status();

                if let Ok(s) = status {
                    if s.success() {
                        return Ok(());
                    }
                }
            }
        }
    }

    // 2. Fallback to capturing the focused or targeted output via grim
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
