use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    pub yandex_token: String,
    #[serde(default)]
    pub soundcloud_client_id: String,
    #[serde(default)]
    pub discord_client_id: String,
    #[serde(default)]
    pub lastfm_api_key: String,
    #[serde(default)]
    pub liked_shuffle: bool,
    #[serde(default)]
    pub last_update_check: u64,
}

pub const DEFAULT_DISCORD_CLIENT_ID: &str = "1409612809859366932";

/// Epoch seconds (0 if unavailable).
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("music-player-tui")
}

fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

pub fn load() -> Config {
    let path = config_file();
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

pub fn save(config: &Config) {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = config_file();
    if let Ok(text) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(&path, text);
    }
}
