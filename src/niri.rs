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
    pub pos_in_scrolling_layout: Option<[i64; 2]>,
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
    pub workspace_id: Option<u64>,
    #[serde(default)]
    pub is_floating: bool,
    #[serde(default)]
    #[allow(dead_code)]
    pub is_focused: bool,
    pub layout: Option<WindowLayout>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct Workspace {
    pub id: u64,
    pub idx: Option<u64>,
    pub name: Option<String>,
    pub output: Option<String>,
    pub is_active: bool,
    pub is_focused: bool,
    pub active_window_id: Option<u64>,
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

#[allow(dead_code)]
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

pub fn get_all_workspaces() -> Result<Vec<Workspace>, Box<dyn std::error::Error>> {
    let output = Command::new("niri")
        .args(["msg", "-j", "workspaces"])
        .output()?;
    if !output.status.success() {
        return Err("niri msg workspaces failed".into());
    }

    let list: Vec<Workspace> = serde_json::from_slice(&output.stdout)?;
    Ok(list)
}

pub fn find_workspace(
    workspaces: &[Workspace],
    query: &str,
    preferred_output: Option<&str>,
) -> Option<Workspace> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }

    // 1. Try matching workspace name (case-insensitive)
    if let Some(ws) = workspaces
        .iter()
        .find(|w| w.name.as_deref().map(|n| n.eq_ignore_ascii_case(q)).unwrap_or(false))
    {
        return Some(ws.clone());
    }

    // 2. Try matching explicit "id:<number>"
    if let Some(id) = q
        .strip_prefix("id:")
        .and_then(|s| s.trim().parse::<u64>().ok())
    {
        return workspaces.iter().find(|w| w.id == id).cloned();
    }

    // 3. Try matching numeric index (idx)
    if let Ok(idx) = q.parse::<u64>() {
        // If preferred output specified, try matching idx on that output first
        if let Some(pref_out) = preferred_output {
            let found = workspaces
                .iter()
                .find(|w| w.idx == Some(idx) && w.output.as_deref() == Some(pref_out));
            if let Some(ws) = found {
                return Some(ws.clone());
            }
        }
        // Then try matching idx on any focused or active workspace
        if let Some(ws) = workspaces
            .iter()
            .find(|w| w.idx == Some(idx) && (w.is_focused || w.is_active))
        {
            return Some(ws.clone());
        }
        // Fallback to first matching idx
        if let Some(ws) = workspaces.iter().find(|w| w.idx == Some(idx)) {
            return Some(ws.clone());
        }
    }

    None
}

pub fn focus_workspace(reference: &str) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("niri")
        .args(["msg", "action", "focus-workspace", reference])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("niri msg action focus-workspace {reference} failed").into())
    }
}

pub fn focus_workspace_previous() -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("niri")
        .args(["msg", "action", "focus-workspace-previous"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err("niri msg action focus-workspace-previous failed".into())
    }
}

pub fn focus_window(id: u64) -> Result<(), Box<dyn std::error::Error>> {
    let id_str = id.to_string();
    let status = Command::new("niri")
        .args(["msg", "action", "focus-window", "--id", &id_str])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("niri msg action focus-window --id {id} failed").into())
    }
}

pub fn focus_monitor(output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("niri")
        .args(["msg", "action", "focus-monitor", output])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("niri msg action focus-monitor {output} failed").into())
    }
}

pub fn grim_bin() -> String {
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

pub fn is_window_blocked(win: &Window, blocked_apps: &[String]) -> bool {
    if blocked_apps.is_empty() {
        return false;
    }
    let app_id = win.app_id.as_deref().unwrap_or("").to_lowercase();
    let title = win.title.as_deref().unwrap_or("").to_lowercase();

    for blocked in blocked_apps {
        let b = blocked.trim().to_lowercase();
        if b.is_empty() {
            continue;
        }
        if app_id.contains(&b) || title.contains(&b) {
            return true;
        }
    }
    false
}

pub fn get_blocked_window_rects_on_output(
    target_output: &str,
    blocked_apps: &[String],
) -> Vec<[u32; 4]> {
    if blocked_apps.is_empty() {
        return Vec::new();
    }

    let workspaces = match get_all_workspaces() {
        Ok(w) => w,
        Err(_) => return Vec::new(),
    };
    let active_ws = match workspaces
        .into_iter()
        .find(|w| w.output.as_deref() == Some(target_output) && w.is_active)
    {
        Some(w) => w,
        None => return Vec::new(),
    };

    get_blocked_window_rects_for_workspace(active_ws.id, target_output, blocked_apps)
}

pub fn get_blocked_window_rects_for_workspace(
    target_ws_id: u64,
    target_output: &str,
    blocked_apps: &[String],
) -> Vec<[u32; 4]> {
    if blocked_apps.is_empty() {
        return Vec::new();
    }

    let outputs = match get_all_outputs() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let out_info = match outputs.into_iter().find(|o| o.name == target_output) {
        Some(o) => o,
        None => return Vec::new(),
    };
    let logical = match out_info.logical {
        Some(l) => l,
        None => return Vec::new(),
    };
    let mon_w = logical.width as f64;
    let mon_h = logical.height as f64;

    let workspaces = match get_all_workspaces() {
        Ok(w) => w,
        Err(_) => return Vec::new(),
    };
    let target_ws = match workspaces.into_iter().find(|w| w.id == target_ws_id) {
        Some(w) => w,
        None => return Vec::new(),
    };

    let all_windows = match get_all_windows() {
        Ok(w) => w,
        Err(_) => return Vec::new(),
    };

    let ws_windows: Vec<Window> = all_windows
        .into_iter()
        .filter(|w| w.workspace_id == Some(target_ws.id))
        .collect();

    let gap = 8.0f64;
    let mut blocked_rects = Vec::new();

    // 1. Check floating windows
    for win in ws_windows.iter().filter(|w| w.is_floating) {
        if !is_window_blocked(win, blocked_apps) {
            continue;
        }
        if let Some(ref layout) = win.layout {
            let (fx, fy) = if let Some(pos) = layout.tile_pos_in_workspace_view {
                (pos[0], pos[1])
            } else {
                (gap, gap)
            };
            let (w, h) = if let Some(size) = layout.window_size {
                (size[0] as f64, size[1] as f64)
            } else {
                (800.0, 600.0)
            };

            let x0 = fx.max(0.0) as u32;
            let y0 = fy.max(0.0) as u32;
            let x1 = (fx + w).min(mon_w).max(0.0) as u32;
            let y1 = (fy + h).min(mon_h).max(0.0) as u32;
            if x1 > x0 && y1 > y0 {
                blocked_rects.push([x0, y0, x1 - x0, y1 - y0]);
            }
        }
    }

    // 2. Check tiled windows in scrolling layout
    let tiled_windows: Vec<&Window> = ws_windows.iter().filter(|w| !w.is_floating).collect();
    if tiled_windows.is_empty() {
        return blocked_rects;
    }

    // Group by column
    let mut cols: HashMap<i64, Vec<&Window>> = HashMap::new();
    for win in &tiled_windows {
        let col = win
            .layout
            .as_ref()
            .and_then(|l| l.pos_in_scrolling_layout)
            .map(|p| p[0])
            .unwrap_or(1);
        cols.entry(col).or_default().push(win);
    }

    let mut sorted_cols: Vec<i64> = cols.keys().copied().collect();
    sorted_cols.sort_unstable();

    // Calculate column X positions and widths
    let mut col_x: HashMap<i64, f64> = HashMap::new();
    let mut col_w: HashMap<i64, f64> = HashMap::new();
    let mut cur_x = gap;

    for &c in &sorted_cols {
        let w_list = &cols[&c];
        let cw = w_list
            .iter()
            .filter_map(|w| w.layout.as_ref()?.tile_size.map(|s| s[0]))
            .fold(mon_w - 2.0 * gap, f64::max);
        col_x.insert(c, cur_x);
        col_w.insert(c, cw);
        cur_x += cw + gap;
    }

    // Find active column
    let mut active_col = sorted_cols.first().copied().unwrap_or(1);
    if let Some(act_id) = target_ws.active_window_id {
        for (&c, w_list) in &cols {
            if w_list.iter().any(|w| w.id == act_id) {
                active_col = c;
                break;
            }
        }
    }

    // Compute scroll_x (minimal scroll to keep active column on screen)
    let mut scroll_x = 0.0f64;
    if let (Some(&c_left), Some(&cw)) = (col_x.get(&active_col), col_w.get(&active_col)) {
        let c_right = c_left + cw;
        if c_right > mon_w - gap {
            scroll_x = c_right - (mon_w - gap);
        }
    }

    // Check each window
    for &c in &sorted_cols {
        let scr_x = col_x[&c] - scroll_x;
        let mut w_list = cols[&c].clone();
        w_list.sort_by_key(|w| {
            w.layout
                .as_ref()
                .and_then(|l| l.pos_in_scrolling_layout)
                .map(|p| p[1])
                .unwrap_or(1)
        });

        let mut cur_y = gap;
        for w in w_list {
            let (win_w, win_h) = if let Some(ref l) = w.layout {
                let ts = l.tile_size.unwrap_or([col_w[&c], mon_h - 2.0 * gap]);
                (ts[0], ts[1])
            } else {
                (col_w[&c], mon_h - 2.0 * gap)
            };

            let is_blocked = is_window_blocked(w, blocked_apps);
            if is_blocked {
                let x0 = scr_x.max(0.0) as u32;
                let y0 = cur_y.max(0.0) as u32;
                let x1 = (scr_x + win_w).min(mon_w).max(0.0) as u32;
                let y1 = (cur_y + win_h).min(mon_h).max(0.0) as u32;

                if x1 > x0 && y1 > y0 {
                    blocked_rects.push([x0, y0, x1 - x0, y1 - y0]);
                }
            }

            cur_y += win_h + gap;
        }
    }

    blocked_rects
}

/// Captures a screenshot of the specified output (or the focused output if None).
/// Uses grim (Wayland zwlr_screencopy_manager_v1) to capture the output directly.
/// Returns the name of the captured output.
/// NEVER touches or modifies the Wayland clipboard or CopyQ history.
pub fn capture_screenshot(
    path: &str,
    target_output: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let grim = grim_bin();
    let out_name = target_output
        .map(|s| s.to_string())
        .or_else(|| get_focused_output().map(|o| o.name))
        .ok_or("No active output found")?;

    let mut cmd = Command::new(&grim);
    cmd.arg("-c");
    cmd.args(["-o", &out_name]);
    cmd.arg(path);

    let status = cmd.status()?;
    if status.success() {
        Ok(out_name)
    } else {
        Err(format!("grim failed to capture output {out_name}").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_workspace() {
        let workspaces = vec![
            Workspace {
                id: 1,
                idx: Some(1),
                name: Some("demonstrations".to_string()),
                output: Some("DP-2".to_string()),
                is_active: false,
                is_focused: false,
                active_window_id: None,
            },
            Workspace {
                id: 2,
                idx: Some(2),
                name: Some("main".to_string()),
                output: Some("DP-1".to_string()),
                is_active: true,
                is_focused: true,
                active_window_id: None,
            },
            Workspace {
                id: 3,
                idx: Some(3),
                name: None,
                output: Some("DP-1".to_string()),
                is_active: false,
                is_focused: false,
                active_window_id: None,
            },
        ];

        // Search by name (case-insensitive)
        let found = find_workspace(&workspaces, "demonstrations", None);
        assert_eq!(found.unwrap().id, 1);

        let found = find_workspace(&workspaces, "MAIN", None);
        assert_eq!(found.unwrap().id, 2);

        // Search by idx
        let found = find_workspace(&workspaces, "3", None);
        assert_eq!(found.unwrap().id, 3);

        // Search by id
        let found = find_workspace(&workspaces, "id:1", None);
        assert_eq!(found.unwrap().id, 1);

        // Not found
        let found = find_workspace(&workspaces, "nonexistent", None);
        assert!(found.is_none());
    }
}
