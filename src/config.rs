use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_CHANNEL: &str = "default";
pub const DEFAULT_BASE: &str = "https://api1.xkji.com/api/v1";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub download_dir: Option<String>,
    #[serde(default)]
    pub channels: BTreeMap<String, String>,
}

impl Config {
    pub fn channel_base(&self, name: &str) -> Result<&str> {
        if name == DEFAULT_CHANNEL {
            return Ok(DEFAULT_BASE);
        }
        self.channels
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| anyhow::anyhow!("未知渠道：{name}，请先运行 qiuchenly channel add"))
    }
}

pub fn config_dir() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("无法确定用户主目录"))?
        .join(".config")
        .join("qiuchenly"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

pub fn load() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config::default());
    }
    let raw = fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn save(cfg: &Config) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(cfg)?)?;
    Ok(())
}

pub fn download_dir(cfg: &Config) -> Result<PathBuf> {
    if let Some(d) = &cfg.download_dir {
        return Ok(PathBuf::from(d));
    }
    Ok(dirs::download_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Downloads")
    }))
}
