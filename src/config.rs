use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FtpConfig {
    pub host: Option<String>,
    pub port: u16,
    pub user: Option<String>,
    pub pass: Option<String>,
    pub remote_dir: Option<String>,
    pub secure: bool,
}

impl Default for FtpConfig {
    fn default() -> Self {
        Self {
            host: None,
            port: 21,
            user: None,
            pass: None,
            remote_dir: None,
            secure: false,
        }
    }
}

impl FtpConfig {
    pub fn is_configured(&self) -> bool {
        self.host.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub dir: PathBuf,
    pub interval_secs: u64,
    pub daily_hours: f32,
    pub start_time: Option<String>,
    pub output: Option<String>,
    pub workspace: Option<String>,
    pub scale: Option<f32>,
    pub upload_cmd: Option<String>,
    pub blocked_apps: Vec<String>,
    pub image_format: String,
    pub jpeg_quality: u8,
    pub max_width: Option<u32>,
    pub ftp: FtpConfig,
}

fn expand_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if let Ok(home) = std::env::var("HOME") {
        if trimmed == "~" || trimmed == "$HOME" || trimmed == "${HOME}" {
            return PathBuf::from(home);
        }
        if let Some(rest) = trimmed.strip_prefix("~/") {
            return PathBuf::from(home).join(rest);
        }
        if let Some(rest) = trimmed.strip_prefix("$HOME/") {
            return PathBuf::from(home).join(rest);
        }
        if let Some(rest) = trimmed.strip_prefix("${HOME}/") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(trimmed)
}

pub fn extract_env_value(raw: &str) -> String {
    let s = raw.trim();
    if let Some(stripped) = s.strip_prefix('"')
        && let Some(end_idx) = stripped.find('"')
    {
        return stripped[..end_idx].to_string();
    } else if let Some(stripped) = s.strip_prefix('\'')
        && let Some(end_idx) = stripped.find('\'')
    {
        return stripped[..end_idx].to_string();
    }
    // Unquoted: take up to '#' if present
    if let Some((val, _comment)) = s.split_once('#') {
        val.trim().trim_matches('"').trim_matches('\'').trim().to_string()
    } else {
        s.trim_matches('"').trim_matches('\'').trim().to_string()
    }
}

impl Default for Config {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Self {
            dir: PathBuf::from(home).join("worklog"),
            interval_secs: 60,
            daily_hours: 8.0,
            start_time: None,
            output: None,
            workspace: None,
            scale: None,
            upload_cmd: None,
            blocked_apps: Vec::new(),
            image_format: "jpg".to_string(),
            jpeg_quality: 60,
            max_width: Some(1920),
            ftp: FtpConfig::default(),
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
                        let val_str = extract_env_value(v);
                        let val = val_str.as_str();
                        match key {
                            "WORKTRACK_DIR" => cfg.dir = expand_path(val),
                            "WORKTRACK_INTERVAL" => {
                                if let Ok(n) = val.parse::<u64>() {
                                    cfg.interval_secs = n.max(5);
                                }
                            }
                            "WORKTRACK_DAILY_HOURS" | "WORKTRACK_WORK_HOURS" => {
                                if let Ok(h) = val.parse::<f32>()
                                    && h.is_finite() && h > 0.0
                                {
                                    cfg.daily_hours = h.clamp(0.1, 24.0);
                                }
                            }
                            "WORKTRACK_START_TIME" if !val.is_empty() => {
                                cfg.start_time = Some(val.to_string());
                            }
                            "WORKTRACK_OUTPUT" if !val.is_empty() => {
                                cfg.output = Some(val.to_string());
                            }
                            "WORKTRACK_WORKSPACE" if !val.is_empty() => {
                                cfg.workspace = Some(val.to_string());
                            }
                            "WORKTRACK_SCALE" => {
                                if let Ok(s) = val.parse::<f32>()
                                    && s.is_finite() && s > 0.0
                                {
                                    cfg.scale = Some(s.clamp(0.5, 5.0));
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
                            "WORKTRACK_FORMAT" => {
                                cfg.image_format = val.to_lowercase();
                            }
                            "WORKTRACK_JPEG_QUALITY" | "WORKTRACK_QUALITY" => {
                                if let Ok(q) = val.parse::<u8>() {
                                    cfg.jpeg_quality = q.clamp(10, 100);
                                }
                            }
                            "WORKTRACK_MAX_WIDTH" => {
                                let lower = val.to_lowercase();
                                if lower == "0" || lower == "none" || lower == "off" || lower == "full" {
                                    cfg.max_width = None;
                                } else if let Ok(w) = val.parse::<u32>() {
                                    cfg.max_width = Some(w);
                                }
                            }
                            "WORKTRACK_FTP_HOST" if !val.is_empty() => {
                                cfg.ftp.host = Some(val.to_string());
                            }
                            "WORKTRACK_FTP_PORT" => {
                                if let Ok(p) = val.parse::<u16>() {
                                    cfg.ftp.port = p;
                                }
                            }
                            "WORKTRACK_FTP_USER" if !val.is_empty() => {
                                cfg.ftp.user = Some(val.to_string());
                            }
                            "WORKTRACK_FTP_PASS" | "WORKTRACK_FTP_PASSWORD" => {
                                cfg.ftp.pass = Some(val.to_string());
                            }
                            "WORKTRACK_FTP_DIR" | "WORKTRACK_FTP_PATH" if !val.is_empty() => {
                                cfg.ftp.remote_dir = Some(val.to_string());
                            }
                            "WORKTRACK_FTP_SECURE" | "WORKTRACK_FTPS" => {
                                cfg.ftp.secure = matches!(val.to_lowercase().as_str(), "true" | "1" | "yes");
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
            .and_then(|v| v.parse::<u64>().ok())
        {
            cfg.interval_secs = n.max(5);
        }
        if let Some(h) = std::env::var("WORKTRACK_DAILY_HOURS")
            .ok()
            .or_else(|| std::env::var("WORKTRACK_WORK_HOURS").ok())
            .and_then(|v| v.trim().parse::<f32>().ok())
            && h.is_finite() && h > 0.0
        {
            cfg.daily_hours = h.clamp(0.1, 24.0);
        }
        if let Ok(val) = std::env::var("WORKTRACK_START_TIME") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                cfg.start_time = Some(trimmed.to_string());
            }
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
            && s.is_finite() && s > 0.0
        {
            cfg.scale = Some(s.clamp(0.5, 5.0));
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
        if let Ok(val) = std::env::var("WORKTRACK_FORMAT") {
            cfg.image_format = val.trim().to_lowercase();
        }
        if let Some(q) = std::env::var("WORKTRACK_JPEG_QUALITY")
            .ok()
            .or_else(|| std::env::var("WORKTRACK_QUALITY").ok())
            .and_then(|v| v.trim().parse::<u8>().ok())
        {
            cfg.jpeg_quality = q.clamp(10, 100);
        }
        if let Ok(val) = std::env::var("WORKTRACK_MAX_WIDTH") {
            let lower = val.trim().to_lowercase();
            if lower == "0" || lower == "none" || lower == "off" || lower == "full" {
                cfg.max_width = None;
            } else if let Ok(w) = val.trim().parse::<u32>() {
                cfg.max_width = Some(w);
            }
        }
        if let Ok(val) = std::env::var("WORKTRACK_FTP_HOST") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                cfg.ftp.host = Some(trimmed.to_string());
            }
        }
        if let Some(p) = std::env::var("WORKTRACK_FTP_PORT")
            .ok()
            .and_then(|v| v.trim().parse::<u16>().ok())
        {
            cfg.ftp.port = p;
        }
        if let Ok(val) = std::env::var("WORKTRACK_FTP_USER") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                cfg.ftp.user = Some(trimmed.to_string());
            }
        }
        if let Ok(val) = std::env::var("WORKTRACK_FTP_PASS")
            .or_else(|_| std::env::var("WORKTRACK_FTP_PASSWORD"))
        {
            cfg.ftp.pass = Some(val);
        }
        if let Ok(val) = std::env::var("WORKTRACK_FTP_DIR")
            .or_else(|_| std::env::var("WORKTRACK_FTP_PATH"))
        {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                cfg.ftp.remote_dir = Some(trimmed.to_string());
            }
        }
        if let Ok(val) = std::env::var("WORKTRACK_FTP_SECURE")
            .or_else(|_| std::env::var("WORKTRACK_FTPS"))
        {
            cfg.ftp.secure = matches!(val.trim().to_lowercase().as_str(), "true" | "1" | "yes");
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
        assert_eq!(cfg.daily_hours, 8.0);
        assert_eq!(cfg.image_format, "jpg");
        assert_eq!(cfg.jpeg_quality, 60);
        assert_eq!(cfg.max_width, Some(1920));
        assert!(!cfg.ftp.is_configured());
        assert!(cfg.output.is_none());
        assert!(cfg.workspace.is_none());
    }

    #[test]
    fn test_extract_env_value() {
        assert_eq!(extract_env_value("60 # comment"), "60");
        assert_eq!(extract_env_value("\"DP-1\" # comment"), "DP-1");
        assert_eq!(extract_env_value("'secret#123' # inline"), "secret#123");
        assert_eq!(extract_env_value("\"pass#word\""), "pass#word");
        assert_eq!(extract_env_value("8.5"), "8.5");
        assert_eq!(extract_env_value("~/worklog # path"), "~/worklog");
    }
}
