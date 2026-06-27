use serde::Deserialize;
use std::path::PathBuf;

/// Top-level configuration loaded from ~/.migu/config.toml
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub keys: KeyConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
}

#[derive(Debug, Deserialize)]
pub struct KeyConfig {
    /// Global leader key applied to bindings that use ${leader}
    #[serde(default = "default_leader")]
    pub leader: String,
    /// Toggle sort mode binding
    #[serde(default = "default_toggle_sort")]
    pub toggle_sort: String,
    /// Number jump mode binding
    #[serde(default = "default_toggle_numbers")]
    pub toggle_numbers: String,
    /// Help toggle binding
    #[serde(default = "default_toggle_help")]
    pub toggle_help: String,
    /// Set display limit binding
    #[serde(default = "default_set_limit")]
    pub set_limit: String,
}

impl Default for KeyConfig {
    fn default() -> Self {
        KeyConfig {
            leader: default_leader(),
            toggle_sort: default_toggle_sort(),
            toggle_numbers: default_toggle_numbers(),
            toggle_help: default_toggle_help(),
            set_limit: default_set_limit(),
        }
    }
}

fn default_leader() -> String {
    "Alt".to_string()
}
fn default_toggle_sort() -> String {
    "${leader} + s".to_string()
}
fn default_toggle_numbers() -> String {
    "${leader} + n".to_string()
}
fn default_toggle_help() -> String {
    "${leader} + h".to_string()
}
fn default_set_limit() -> String {
    "${leader} + l".to_string()
}

#[derive(Debug, Default, Deserialize)]
pub struct DatabaseConfig {
    /// Custom database file path (default: ~/.migu/history.db)
    #[serde(default)]
    pub path: Option<String>,
    /// Max entries to keep; older ones are purged probabilistically.
    /// If not set, no limit. Env var MIGU_MAX_ENTRIES takes precedence.
    #[serde(default)]
    pub max_entries: Option<i64>,
}

/// Resolve a binding string like "Ctrl + l" or "${leader} + s" into (KeyModifiers, char).
/// The leader placeholder is expanded first, then the key and modifiers are extracted.
pub fn resolve_binding(binding: &str, leader: &str) -> (crossterm::event::KeyModifiers, char) {
    use crossterm::event::KeyModifiers;

    let resolved = binding.replace("${leader}", leader);
    let parts: Vec<&str> = resolved.split('+').map(|s| s.trim()).collect();

    let (mod_part, key_part) = if parts.len() == 1 {
        (String::new(), parts[0].to_string())
    } else {
        let key = parts[parts.len() - 1].to_string();
        let mods = parts[..parts.len() - 1].join("+");
        (mods, key)
    };

    let key_char = key_part.chars().next().unwrap_or(' ');

    let mut m = KeyModifiers::NONE;
    for part in mod_part.split('+') {
        match part.trim().to_lowercase().as_str() {
            "alt" => m |= KeyModifiers::ALT,
            "ctrl" | "control" => m |= KeyModifiers::CONTROL,
            "shift" => m |= KeyModifiers::SHIFT,
            _ => {}
        }
    }
    (m, key_char)
}

/// Resolved key bindings: (KeyModifiers, char) pairs ready for use in TUI.
pub struct ResolvedKeys {
    pub toggle_sort: (crossterm::event::KeyModifiers, char),
    pub toggle_numbers: (crossterm::event::KeyModifiers, char),
    pub toggle_help: (crossterm::event::KeyModifiers, char),
    pub set_limit: (crossterm::event::KeyModifiers, char),
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
    let base = dirs::home_dir().unwrap_or(PathBuf::from("."));
    base.join(".migu").join("config.toml")
}
