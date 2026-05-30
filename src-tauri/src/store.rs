//! File-backed persistence for servers and settings.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::parser::Server;
use crate::singbox::Mode;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    pub mode: Mode,
    pub bypass_ru: bool,
    pub active_server: Option<String>,
    /// Accent: a named preset ("amber", "teal", …) or a hex color ("#rrggbb").
    #[serde(default = "default_accent")]
    pub accent: String,
    /// Theme: "dark" | "light" | "system".
    #[serde(default = "default_theme")]
    pub theme: String,
}

fn default_accent() -> String {
    "amber".to_string()
}
fn default_theme() -> String {
    "dark".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            mode: Mode::SystemProxy,
            bypass_ru: true,
            active_server: None,
            accent: default_accent(),
            theme: default_theme(),
        }
    }
}

fn base_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME не задана".to_string())?;
    let dir = PathBuf::from(home).join("Library/Application Support/coffeeNetwork");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn servers_file() -> Result<PathBuf, String> {
    Ok(base_dir()?.join("servers.json"))
}

fn settings_file() -> Result<PathBuf, String> {
    Ok(base_dir()?.join("settings.json"))
}

pub fn load_servers() -> Vec<Server> {
    let Ok(path) = servers_file() else { return vec![] };
    let Ok(text) = fs::read_to_string(path) else { return vec![] };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_servers(servers: &[Server]) -> Result<(), String> {
    let path = servers_file()?;
    let text = serde_json::to_string_pretty(servers).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| e.to_string())
}

pub fn load_settings() -> Settings {
    let Ok(path) = settings_file() else { return Settings::default() };
    let Ok(text) = fs::read_to_string(path) else { return Settings::default() };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_settings(settings: &Settings) -> Result<(), String> {
    let path = settings_file()?;
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| e.to_string())
}
