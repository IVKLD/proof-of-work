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

    pub fn target_url(&self, file_name: Option<&str>) -> Option<String> {
        let raw_host = self.host.as_deref()?;
        let host = raw_host
            .trim()
            .trim_start_matches("ftps://")
            .trim_start_matches("ftp://")
            .trim_end_matches('/')
            .trim();
        let proto = if self.secure { "ftps" } else { "ftp" };
        let port = self.port;
        let user_prefix = self.user.as_deref().map(|u| format!("{u}@")).unwrap_or_default();
        let rdir = self.remote_dir.as_deref().unwrap_or("").trim().trim_matches('/');
        let base = if rdir.is_empty() {
            format!("{proto}://{user_prefix}{host}:{port}/")
        } else {
            format!("{proto}://{user_prefix}{host}:{port}/{rdir}/")
        };
        Some(match file_name {
            Some(f) => format!("{base}{f}"),
            None => base,
        })
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
    let trimmed = raw.trim().trim_matches(['"', '\'']).trim();
    if let Ok(home) = std::env::var("HOME") {
        if matches!(trimmed, "~" | "$HOME" | "${HOME}") {
            return PathBuf::from(home);
        }
        for prefix in ["~/", "$HOME/", "${HOME}/"] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                return PathBuf::from(&home).join(rest);
            }
        }
    }
    PathBuf::from(trimmed)
}

pub fn extract_env_value(raw: &str) -> String {
    let s = raw.trim();
    for quote in ['"', '\''] {
        if let Some(stripped) = s.strip_prefix(quote)
            && let Some(end_idx) = stripped.find(quote)
        {
            return stripped[..end_idx].to_string();
        }
    }
    s.split_once('#')
        .map_or(s, |(val, _)| val)
        .trim()
        .trim_matches(['"', '\''])
        .to_string()
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
    fn apply_key_value(&mut self, key: &str, val: &str) {
        let trimmed = val.trim();
        match key {
            "WORKTRACK_DIR" => self.dir = expand_path(trimmed),
            "WORKTRACK_INTERVAL" => {
                if let Ok(n) = trimmed.parse::<u64>() {
                    self.interval_secs = n.max(5);
                }
            }
            "WORKTRACK_DAILY_HOURS" | "WORKTRACK_WORK_HOURS" => {
                if let Ok(h) = trimmed.parse::<f32>()
                    && h.is_finite() && h > 0.0
                {
                    self.daily_hours = h.clamp(0.1, 24.0);
                }
            }
            "WORKTRACK_START_TIME" if !trimmed.is_empty() => {
                self.start_time = Some(trimmed.to_string());
            }
            "WORKTRACK_OUTPUT" if !trimmed.is_empty() => {
                self.output = Some(trimmed.to_string());
            }
            "WORKTRACK_WORKSPACE" if !trimmed.is_empty() => {
                self.workspace = Some(trimmed.to_string());
            }
            "WORKTRACK_SCALE" => {
                if let Ok(s) = trimmed.parse::<f32>()
                    && s.is_finite() && s > 0.0
                {
                    self.scale = Some(s.clamp(0.5, 5.0));
                }
            }
            "WORKTRACK_UPLOAD_CMD" => self.upload_cmd = Some(trimmed.to_string()),
            "WORKTRACK_BLOCKED_APPS" => {
                self.blocked_apps = trimmed
                    .split(',')
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "WORKTRACK_FORMAT" => {
                self.image_format = trimmed.to_lowercase();
            }
            "WORKTRACK_JPEG_QUALITY" | "WORKTRACK_QUALITY" => {
                if let Ok(q) = trimmed.parse::<u8>() {
                    self.jpeg_quality = q.clamp(10, 100);
                }
            }
            "WORKTRACK_MAX_WIDTH" => {
                let lower = trimmed.to_lowercase();
                if lower == "0" || lower == "none" || lower == "off" || lower == "full" {
                    self.max_width = None;
                } else if let Ok(w) = trimmed.parse::<u32>() {
                    self.max_width = Some(w);
                }
            }
            "WORKTRACK_FTP_HOST" if !trimmed.is_empty() => {
                self.ftp.host = Some(trimmed.to_string());
            }
            "WORKTRACK_FTP_PORT" => {
                if let Ok(p) = trimmed.parse::<u16>() {
                    self.ftp.port = p;
                }
            }
            "WORKTRACK_FTP_USER" if !trimmed.is_empty() => {
                self.ftp.user = Some(trimmed.to_string());
            }
            "WORKTRACK_FTP_PASS" | "WORKTRACK_FTP_PASSWORD" => {
                self.ftp.pass = Some(trimmed.to_string());
            }
            "WORKTRACK_FTP_DIR" | "WORKTRACK_FTP_PATH" if !trimmed.is_empty() => {
                self.ftp.remote_dir = Some(trimmed.to_string());
            }
            "WORKTRACK_FTP_SECURE" | "WORKTRACK_FTPS" => {
                self.ftp.secure = matches!(trimmed.to_lowercase().as_str(), "true" | "1" | "yes");
            }
            _ => {}
        }
    }

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
                        let val = extract_env_value(v);
                        cfg.apply_key_value(key, &val);
                    }
                }
            }
        }

        // 2. Override with ambient env vars
        for (k, v) in std::env::vars() {
            if k.starts_with("WORKTRACK_") {
                cfg.apply_key_value(&k, &v);
            }
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
