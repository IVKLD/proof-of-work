use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub dir: PathBuf,
    pub interval_secs: u64,
    pub output: Option<String>,
    pub workspace: Option<String>,
    pub scale: Option<f32>,
    pub upload_cmd: Option<String>,
    pub blocked_apps: Vec<String>,
}

fn expand_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if let Ok(home) = std::env::var("HOME") {
        if trimmed == "~" || trimmed == "$HOME" {
            return PathBuf::from(home);
        }
        if let Some(rest) = trimmed.strip_prefix("~/") {
            return PathBuf::from(home).join(rest);
        }
        if let Some(rest) = trimmed.strip_prefix("$HOME/") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(trimmed)
}

impl Default for Config {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Self {
            dir: PathBuf::from(home).join("worklog"),
            interval_secs: 60,
            output: None,
            workspace: None,
            scale: None,
            upload_cmd: None,
            blocked_apps: Vec::new(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let mut cfg = Self::default();

        // 1. Try reading ~/.config/worktrack.env
        if let Ok(home) = std::env::var("HOME") {
            let env_path = PathBuf::from(home).join(".config/worktrack.env");
            if let Ok(content) = fs::read_to_string(&env_path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    if let Some((k, v)) = trimmed.split_once('=') {
                        let key = k.trim().trim_start_matches("export ").trim();
                        let val = v.trim().trim_matches('"').trim_matches('\'').trim();
                        match key {
                            "WORKTRACK_DIR" => cfg.dir = expand_path(val),
                            "WORKTRACK_INTERVAL" => {
                                if let Ok(n) = val.parse() {
                                    cfg.interval_secs = n;
                                }
                            }
                            "WORKTRACK_OUTPUT" if !val.is_empty() => {
                                cfg.output = Some(val.to_string());
                            }
                            "WORKTRACK_WORKSPACE" if !val.is_empty() => {
                                cfg.workspace = Some(val.to_string());
                            }
                            "WORKTRACK_SCALE" => {
                                if let Ok(s) = val.parse::<f32>() {
                                    cfg.scale = Some(s);
                                }
                            }
                            "WORKTRACK_UPLOAD_CMD" => cfg.upload_cmd = Some(val.to_string()),
                            "WORKTRACK_BLOCKED_APPS" => {
                                cfg.blocked_apps = val
                                    .split(',')
                                    .map(|s| s.trim().to_lowercase())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // 2. Override with ambient env vars if set
        if let Ok(val) = std::env::var("WORKTRACK_DIR") {
            cfg.dir = expand_path(&val);
        }
        if let Some(n) = std::env::var("WORKTRACK_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            cfg.interval_secs = n;
        }
        if let Ok(val) = std::env::var("WORKTRACK_OUTPUT") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                cfg.output = Some(trimmed.to_string());
            }
        }
        if let Ok(val) = std::env::var("WORKTRACK_WORKSPACE") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                cfg.workspace = Some(trimmed.to_string());
            }
        }
        if let Some(s) = std::env::var("WORKTRACK_SCALE")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
        {
            cfg.scale = Some(s);
        }
        if let Ok(val) = std::env::var("WORKTRACK_UPLOAD_CMD") {
            cfg.upload_cmd = Some(val);
        }
        if let Ok(val) = std::env::var("WORKTRACK_BLOCKED_APPS") {
            cfg.blocked_apps = val
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
        }

        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.interval_secs, 60);
        assert!(cfg.output.is_none());
        assert!(cfg.workspace.is_none());
    }
}
