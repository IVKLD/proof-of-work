use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub dir: PathBuf,
    pub interval_secs: u64,
    pub apps_regex: String,
    pub upload_cmd: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Self {
            dir: PathBuf::from(home).join("worklog"),
            interval_secs: 60,
            apps_regex: "^(kitty|firefox|code|Code|discord|vesktop|org\\.telegram\\.desktop)$"
                .to_string(),
            upload_cmd: None,
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
                            "WORKTRACK_DIR" => cfg.dir = PathBuf::from(val),
                            "WORKTRACK_INTERVAL" => {
                                if let Ok(n) = val.parse() {
                                    cfg.interval_secs = n;
                                }
                            }
                            "WORKTRACK_APPS" => cfg.apps_regex = val.to_string(),
                            "WORKTRACK_UPLOAD_CMD" => cfg.upload_cmd = Some(val.to_string()),
                            _ => {}
                        }
                    }
                }
            }
        }

        // 2. Override with ambient env vars if set
        if let Ok(val) = std::env::var("WORKTRACK_DIR") {
            cfg.dir = PathBuf::from(val);
        }
        if let Some(n) = std::env::var("WORKTRACK_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            cfg.interval_secs = n;
        }
        if let Ok(val) = std::env::var("WORKTRACK_APPS") {
            cfg.apps_regex = val;
        }
        if let Ok(val) = std::env::var("WORKTRACK_UPLOAD_CMD") {
            cfg.upload_cmd = Some(val);
        }

        cfg
    }
}
