use serde::Deserialize;
use std::collections::HashMap;
use std::process::Command;

#[derive(Debug, Deserialize, Clone)]
pub struct LogicalOutput {
    pub width: u64,
    pub height: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OutputInfo {
    pub name: String,
    pub make: Option<String>,
    pub model: Option<String>,
    pub logical: Option<LogicalOutput>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Window {
    pub id: u64,
    pub title: Option<String>,
    pub app_id: Option<String>,
    pub workspace_id: Option<u64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Workspace {
    pub id: u64,
    pub idx: Option<u64>,
    pub name: Option<String>,
    pub output: Option<String>,
    pub is_active: bool,
    pub is_focused: bool,
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

#[derive(Debug, Deserialize)]
struct FocusedWindowInfo {
    id: u64,
}

pub fn get_focused_window_id() -> Option<u64> {
    let output = Command::new("niri")
        .args(["msg", "-j", "focused-window"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice::<FocusedWindowInfo>(&output.stdout)
        .ok()
        .map(|w| w.id)
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

pub struct AnimationGuard {
    active: bool,
}

impl AnimationGuard {
    pub fn disable_workspace_animation() -> Self {
        // Read user's niri config and load temporary config with workspace-switch { off }
        if let Ok(home) = std::env::var("HOME") {
            let config_path = std::path::PathBuf::from(home).join(".config/niri/config.kdl");
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                let temp_path = std::env::temp_dir().join("worktrack-niri-noanim.kdl");
                let modified = if content.contains("workspace-switch { off }")
                    || content.contains("workspace-switch {off}")
                {
                    content
                } else if content.contains("animations {") {
                    content.replacen(
                        "animations {",
                        "animations {\n\tworkspace-switch { off }",
                        1,
                    )
                } else {
                    format!("animations {{\n\tworkspace-switch {{ off }}\n}}\n{content}")
                };

                if std::fs::write(&temp_path, modified).is_ok() {
                    let path_str = temp_path.to_string_lossy().to_string();
                    let status = Command::new("niri")
                        .args(["msg", "action", "load-config-file", "--path", &path_str])
                        .status();
                    if status.is_ok_and(|s| s.success()) {
                        return Self { active: true };
                    }
                }
            }
        }
        Self { active: false }
    }
}

impl Drop for AnimationGuard {
    fn drop(&mut self) {
        if self.active {
            // Restore user's original config
            let _ = Command::new("niri")
                .args(["msg", "action", "load-config-file"])
                .status();
            let temp_path = std::env::temp_dir().join("worktrack-niri-noanim.kdl");
            let _ = std::fs::remove_file(temp_path);
        }
    }
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

    let all_windows = match get_all_windows() {
        Ok(w) => w,
        Err(_) => return Vec::new(),
    };

    let has_blocked = all_windows
        .iter()
        .any(|w| w.workspace_id == Some(target_ws_id) && is_window_blocked(w, blocked_apps));

    if !has_blocked {
        return Vec::new();
    }

    // A forbidden application is detected on the captured workspace:
    // Black out the entire screen to guarantee privacy without guessing partial scroll offsets.
    let outputs = get_all_outputs().unwrap_or_default();
    if let Some(logical) = outputs
        .into_iter()
        .find(|o| o.name == target_output)
        .and_then(|o| o.logical)
    {
        return vec![[0, 0, logical.width as u32, logical.height as u32]];
    }
    vec![[0, 0, 3840, 2160]]
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
            },
            Workspace {
                id: 2,
                idx: Some(2),
                name: Some("main".to_string()),
                output: Some("DP-1".to_string()),
                is_active: true,
                is_focused: true,
            },
            Workspace {
                id: 3,
                idx: Some(3),
                name: None,
                output: Some("DP-1".to_string()),
                is_active: false,
                is_focused: false,
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
