use std::collections::HashSet;
use std::process::Command;
use regex::Regex;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Workspace {
    pub id: u64,
    pub is_active: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Window {
    pub id: u64,
    pub title: Option<String>,
    pub app_id: Option<String>,
    pub workspace_id: Option<u64>,
}

pub fn get_active_workspaces() -> Result<HashSet<u64>, Box<dyn std::error::Error>> {
    let output = Command::new("niri")
        .args(["msg", "-j", "workspaces"])
        .output()?;
    if !output.status.success() {
        return Err("niri msg workspaces failed".into());
    }

    let workspaces: Vec<Workspace> = serde_json::from_slice(&output.stdout)?;
    let active_ids = workspaces
        .into_iter()
        .filter(|w| w.is_active)
        .map(|w| w.id)
        .collect();
    Ok(active_ids)
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

pub fn get_matching_windows(
    app_regex: &Regex,
    only_active_workspaces: bool,
) -> Result<Vec<Window>, Box<dyn std::error::Error>> {
    let active_workspaces = if only_active_workspaces {
        get_active_workspaces()?
    } else {
        HashSet::new()
    };

    let windows = get_all_windows()?;
    let matched = windows
        .into_iter()
        .filter(|w| {
            if let Some(app_id) = &w.app_id {
                if !app_regex.is_match(app_id) {
                    return false;
                }
            } else {
                return false;
            }

            if only_active_workspaces {
                if let Some(wid) = w.workspace_id {
                    active_workspaces.contains(&wid)
                } else {
                    false
                }
            } else {
                true
            }
        })
        .collect();

    Ok(matched)
}

pub fn screenshot_window(id: u64, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("niri")
        .args([
            "msg",
            "action",
            "screenshot-window",
            "--id",
            &id.to_string(),
            "--write-to-disk",
            "true",
            "--path",
            path,
        ])
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("failed to screenshot window id {id}").into())
    }
}
