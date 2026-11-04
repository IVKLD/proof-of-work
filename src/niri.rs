use serde::Deserialize;
use std::collections::HashMap;
use std::process::Command;

#[derive(Debug, Deserialize, Clone)]
pub struct LogicalOutput {
    pub width: u64,
    pub height: u64,
    pub scale: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OutputInfo {
    pub name: String,
    pub make: Option<String>,
    pub model: Option<String>,
    pub logical: Option<LogicalOutput>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WindowLayout {
    pub pos_in_scrolling_layout: Option<[u64; 2]>,
    pub tile_size: Option<[f64; 2]>,
    pub window_size: Option<[u64; 2]>,
}

impl WindowLayout {
    pub fn size(&self) -> Option<[f64; 2]> {
        self.tile_size.or_else(|| self.window_size.map(|s| [s[0] as f64, s[1] as f64]))
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Window {
    pub id: u64,
    pub title: Option<String>,
    pub app_id: Option<String>,
    pub workspace_id: Option<u64>,
    pub _is_floating: Option<bool>,
    pub layout: Option<WindowLayout>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Workspace {
    pub id: u64,
    pub idx: Option<u64>,
    pub name: Option<String>,
    pub output: Option<String>,
    pub is_active: bool,
    pub is_focused: bool,
    pub active_window_id: Option<u64>,
}

impl Workspace {
    pub fn focus(&self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(ref name) = self.name {
            focus_workspace(name)
        } else if let Some(idx) = self.idx {
            focus_workspace(&idx.to_string())
        } else {
            focus_workspace_previous()
        }
    }
}

fn niri_msg_json<T: for<'de> Deserialize<'de>>(subcmd: &str) -> Result<T, Box<dyn std::error::Error>> {
    let output = Command::new("niri").args(["msg", "-j", subcmd]).output()?;
    if !output.status.success() {
        return Err(format!("niri msg -j {subcmd} failed").into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn niri_action(args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("niri")
        .args(["msg", "action"])
        .args(args)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("niri msg action {} failed", args.join(" ")).into())
    }
}

pub fn get_focused_output() -> Option<OutputInfo> {
    niri_msg_json("focused-output").ok()
}

#[derive(Debug, Deserialize)]
struct FocusedWindowInfo {
    id: u64,
}

pub fn get_focused_window_id() -> Option<u64> {
    niri_msg_json::<FocusedWindowInfo>("focused-window")
        .ok()
        .map(|w| w.id)
}

pub fn get_all_outputs() -> Result<Vec<OutputInfo>, Box<dyn std::error::Error>> {
    let map: HashMap<String, OutputInfo> = niri_msg_json("outputs")?;
    let mut list: Vec<OutputInfo> = map.into_values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(list)
}

pub fn get_all_windows() -> Result<Vec<Window>, Box<dyn std::error::Error>> {
    niri_msg_json("windows")
}

pub fn get_all_workspaces() -> Result<Vec<Workspace>, Box<dyn std::error::Error>> {
    niri_msg_json("workspaces")
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

    // 1. Try matching workspace name on preferred output first
    if let Some(pref_out) = preferred_output
        && let Some(ws) = workspaces.iter().find(|w| {
            w.name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(q))
                && w.output.as_deref() == Some(pref_out)
        })
    {
        return Some(ws.clone());
    }

    // 2. Try matching workspace name on any output (case-insensitive)
    if let Some(ws) = workspaces
        .iter()
        .find(|w| w.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(q)))
    {
        return Some(ws.clone());
    }

    // 3. Try matching explicit "id:<number>"
    if let Some(id) = q
        .strip_prefix("id:")
        .and_then(|s| s.trim().parse::<u64>().ok())
    {
        return workspaces.iter().find(|w| w.id == id).cloned();
    }

    // 4. Try matching numeric index (idx)
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
    niri_action(&["focus-workspace", reference])
}

pub fn focus_workspace_previous() -> Result<(), Box<dyn std::error::Error>> {
    niri_action(&["focus-workspace-previous"])
}

pub fn focus_window(id: u64) -> Result<(), Box<dyn std::error::Error>> {
    let id_str = id.to_string();
    niri_action(&["focus-window", "--id", &id_str])
}

pub fn focus_monitor(output: &str) -> Result<(), Box<dyn std::error::Error>> {
    niri_action(&["focus-monitor", output])
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
        // 1. Direct match on app_id (standard for Wayland applications)
        if !app_id.is_empty() && app_id.contains(&b) {
            return true;
        }
        // 2. If app_id is unknown or empty, match on title substring
        if app_id.is_empty() && title.contains(&b) {
            return true;
        }
        // 3. Match title prefix/equality to avoid false positives (e.g. browser searching for "telegram")
        if title == b || title.starts_with(&format!("{b} - ")) || title.starts_with(&format!("{b}: ")) {
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

    let Some(active_ws) = get_all_workspaces()
        .unwrap_or_default()
        .into_iter()
        .find(|w| w.output.as_deref() == Some(target_output) && w.is_active)
    else {
        return Vec::new();
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

    let Ok(all_windows) = get_all_windows() else {
        return Vec::new();
    };

    let ws_windows: Vec<&Window> = all_windows
        .iter()
        .filter(|w| w.workspace_id == Some(target_ws_id))
        .collect();

    let has_blocked = ws_windows.iter().any(|w| is_window_blocked(w, blocked_apps));
    if !has_blocked {
        return Vec::new();
    }

    let outputs = get_all_outputs().unwrap_or_default();
    let (screen_w, screen_h, out_scale) = outputs
        .into_iter()
        .find(|o| o.name == target_output)
        .and_then(|o| o.logical)
        .map(|l| (l.width as i64, l.height as i64, l.scale.unwrap_or(1.0)))
        .unwrap_or((3840, 2160, 1.0));

    let all_workspaces = get_all_workspaces().unwrap_or_default();
    let active_win_id = all_workspaces
        .iter()
        .find(|w| w.id == target_ws_id)
        .and_then(|w| w.active_window_id);

    // Group windows by column
    let mut cols: std::collections::BTreeMap<u64, Vec<&Window>> = std::collections::BTreeMap::new();
    for w in &ws_windows {
        if let Some(ref l) = w.layout
            && let Some(pos) = l.pos_in_scrolling_layout
        {
            cols.entry(pos[0]).or_default().push(w);
        }
    }

    // Sort rows within each column
    for rows in cols.values_mut() {
        rows.sort_by_key(|w| {
            w.layout
                .as_ref()
                .and_then(|l| l.pos_in_scrolling_layout)
                .map(|p| p[1])
                .unwrap_or(0)
        });
    }

    const GAP: i64 = 8;
    let mut col_x_map: std::collections::HashMap<u64, (i64, i64)> = std::collections::HashMap::new();
    let mut cur_x = GAP;

    for (&col_idx, rows) in &cols {
        let col_w = rows
            .iter()
            .filter_map(|w| w.layout.as_ref())
            .filter_map(|l| l.size())
            .map(|s| s[0].round() as i64)
            .max()
            .unwrap_or(1200);

        col_x_map.insert(col_idx, (cur_x, col_w));
        cur_x += col_w + GAP;
    }

    // Determine scrolling offset based on active window
    let mut scroll_offset = 0i64;
    if let Some(act_id) = active_win_id
        && let Some(act_win) = ws_windows.iter().find(|w| w.id == act_id)
        && let Some(ref l) = act_win.layout
        && let Some(pos) = l.pos_in_scrolling_layout
        && let Some(&(col_start, col_w)) = col_x_map.get(&pos[0])
    {
        let col_end = col_start + col_w;
        if col_end > screen_w - GAP {
            scroll_offset = col_end - (screen_w - GAP);
        }
        if col_start - scroll_offset < GAP {
            scroll_offset = col_start - GAP;
        }
    }

    // Calculate bounding rect for each blocked window
    let mut blocked_rects = Vec::new();
    for w in &ws_windows {
        if !is_window_blocked(w, blocked_apps) {
            continue;
        }

        if let Some(ref l) = w.layout
            && let Some(pos) = l.pos_in_scrolling_layout
            && let Some(&(col_start, col_w)) = col_x_map.get(&pos[0])
        {
            let win_w = l
                .size()
                .map(|s| s[0].round() as i64)
                .unwrap_or(col_w);

            // Compute vertical position in column
            let rows = cols.get(&pos[0]).cloned().unwrap_or_default();
            let mut cur_y = GAP;
            let mut win_h = screen_h - 2 * GAP;

            for row_win in rows {
                let r_h = row_win
                    .layout
                    .as_ref()
                    .and_then(|rl| rl.size())
                    .map(|s| s[1].round() as i64)
                    .unwrap_or(screen_h - 2 * GAP);

                        if row_win.id == w.id {
                            win_h = r_h;
                            break;
                        }
                        cur_y += r_h + GAP;
                    }

                    let win_x = col_start - scroll_offset;
                    let win_y = cur_y;

                    // Intersect with screen viewport
                    let x1 = win_x.max(0);
                    let y1 = win_y.max(0);
                    let x2 = (win_x + win_w).min(screen_w);
                    let y2 = (win_y + win_h).min(screen_h);

                    if x2 > x1 && y2 > y1 {
                        let rx = ((x1 as f64) * out_scale).round() as u32;
                        let ry = ((y1 as f64) * out_scale).round() as u32;
                        let rw = (((x2 - x1) as f64) * out_scale).round() as u32;
                        let rh = (((y2 - y1) as f64) * out_scale).round() as u32;
                        blocked_rects.push([rx, ry, rw, rh]);
                    }
        }
    }

    // Check if any blocked window exists whose layout coordinates could not be determined (e.g. unmanaged floating window)
    let any_blocked_unmapped = ws_windows.iter().any(|w| {
        is_window_blocked(w, blocked_apps)
            && w.layout
                .as_ref()
                .and_then(|l| l.pos_in_scrolling_layout)
                .is_none()
    });

    // Fallback: only if a blocked window exists on this workspace but could NOT be located in layout at all
    if any_blocked_unmapped {
        let full_w = ((screen_w as f64) * out_scale).round() as u32;
        let full_h = ((screen_h as f64) * out_scale).round() as u32;
        blocked_rects.push([0, 0, full_w, full_h]);
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
    jpeg_quality: u8,
) -> Result<String, Box<dyn std::error::Error>> {
    let out_name = target_output
        .map(|s| s.to_string())
        .or_else(|| get_focused_output().map(|o| o.name))
        .ok_or("No active output found")?;

    let mut cmd = Command::new("grim");
    cmd.args(["-c", "-o", &out_name]);
    let q_str;
    if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        q_str = jpeg_quality.to_string();
        cmd.args(["-t", "jpeg", "-q", &q_str]);
    }
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

    #[test]
    fn test_is_window_blocked() {
        let blocked_apps = vec!["telegram".to_string(), "keepassxc".to_string()];

        let win_telegram = Window {
            id: 1,
            title: Some("Telegram".to_string()),
            app_id: Some("org.telegram.desktop".to_string()),
            workspace_id: Some(1),
            _is_floating: None,
            layout: None,
        };
        assert!(is_window_blocked(&win_telegram, &blocked_apps));

        let win_browser_searching = Window {
            id: 2,
            title: Some("how to create a telegram bot - Google Chrome".to_string()),
            app_id: Some("google-chrome".to_string()),
            workspace_id: Some(1),
            _is_floating: None,
            layout: None,
        };
        assert!(!is_window_blocked(&win_browser_searching, &blocked_apps));

        let win_editor_file = Window {
            id: 3,
            title: Some("telegram_client.rs - nvim".to_string()),
            app_id: Some("kitty".to_string()),
            workspace_id: Some(1),
            _is_floating: None,
            layout: None,
        };
        assert!(!is_window_blocked(&win_editor_file, &blocked_apps));

        let win_keepass = Window {
            id: 4,
            title: Some("KeePassXC - Passwords".to_string()),
            app_id: Some("org.keepassxc.KeePassXC".to_string()),
            workspace_id: Some(1),
            _is_floating: None,
            layout: None,
        };
        assert!(is_window_blocked(&win_keepass, &blocked_apps));
    }
}
