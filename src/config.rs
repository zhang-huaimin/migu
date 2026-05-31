use serde::Deserialize;
use std::path::PathBuf;

/// Top-level configuration loaded from ~/.migu/config.toml
#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub keys: KeyConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            keys: KeyConfig::default(),
            database: DatabaseConfig::default(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct KeyConfig {
    /// Modifier key: "Alt", "Ctrl", "Ctrl+Shift", or "" for none
    #[serde(default = "default_modifier")]
    pub modifier: String,
    /// Character key for toggling sort mode (default: s → Alt+s)
    #[serde(default = "default_toggle_sort")]
    pub toggle_sort: char,
    /// Character key for entering number mode (default: n → Alt+n)
    #[serde(default = "default_toggle_numbers")]
    pub toggle_numbers: char,
    /// Character key for toggling help (default: h → Alt+h)
    #[serde(default = "default_toggle_help")]
    pub toggle_help: char,
    /// Character key for setting limit (default: l → Alt+l)
    #[serde(default = "default_set_limit")]
    pub set_limit: char,
}

impl Default for KeyConfig {
    fn default() -> Self {
        KeyConfig {
            modifier: default_modifier(),
            toggle_sort: default_toggle_sort(),
            toggle_numbers: default_toggle_numbers(),
            toggle_help: default_toggle_help(),
            set_limit: default_set_limit(),
        }
    }
}

fn default_modifier() -> String {
    "Alt".to_string()
}
fn default_toggle_sort() -> char {
    's'
}
fn default_toggle_numbers() -> char {
    'n'
}
fn default_toggle_help() -> char {
    'h'
}
fn default_set_limit() -> char {
    'l'
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    /// Custom database path. If not set, defaults to ~/.migu/history.db
    #[serde(default)]
    pub path: Option<String>,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        DatabaseConfig { path: None }
    }
}

/// Load config from ~/.migu/config.toml, falling back to defaults.
pub fn load() -> Config {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

fn config_path() -> PathBuf {
    let base = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join(".migu").join("config.toml")
}

/// Parse a modifier string (e.g. "Alt", "Ctrl", "Ctrl+Shift") into KeyModifiers.
pub fn parse_modifier(s: &str) -> crossterm::event::KeyModifiers {
    use crossterm::event::KeyModifiers;
    let s = s.trim();
    if s.is_empty() {
        return KeyModifiers::NONE;
    }
    let mut m = KeyModifiers::NONE;
    for part in s.split('+') {
        match part.trim().to_lowercase().as_str() {
            "alt" => m |= KeyModifiers::ALT,
            "ctrl" | "control" => m |= KeyModifiers::CONTROL,
            "shift" => m |= KeyModifiers::SHIFT,
            _ => {}
        }
    }
    m
}
