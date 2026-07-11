use serde::Deserialize;
use std::path::PathBuf;

/// User-level defaults loaded from `~/.config/nodewipe/config.toml` (or the
/// platform equivalent — `dirs::config_dir()` resolves to the right place
/// on Linux/macOS/Windows). Every field is optional; anything unset falls
/// back to nodewipe's built-in defaults. CLI flags always take priority
/// over the config file when both are given.
///
/// Example file:
/// ```toml
/// default_delete_mode = "archive"
/// default_exclude_types = ["venv", "rust_target"]
/// ```
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    pub default_delete_mode: Option<String>,
    pub default_exclude_types: Option<Vec<String>>,
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("nodewipe").join("config.toml"))
}

/// Loads the config file if present. Missing file or parse errors both
/// fall back to `Config::default()` (all-None) rather than failing the
/// whole program — a broken config shouldn't block scanning/deleting.
pub fn load_config() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Config::default();
    };
    toml::from_str(&content).unwrap_or_else(|e| {
        eprintln!("warning: failed to parse {}: {e} (using defaults)", path.display());
        Config::default()
    })
}
