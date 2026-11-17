use serde::{Deserialize, Serialize};
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
/// default_min_mb = 10
/// default_sort = "size"
/// ```
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub default_delete_mode: Option<String>,
    pub default_exclude_types: Option<Vec<String>>,
    pub default_min_mb: Option<u64>,
    pub default_sort: Option<String>,
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("nodewipe").join("config.toml"))
}

pub fn legacy_npkillrc_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".npkillrc"))
}

/// Loads the config file if present. If ~/.config/nodewipe/config.toml is missing,
/// checks for a legacy ~/.npkillrc file for smooth configuration migration.
pub fn load_config() -> Config {
    if let Some(path) = config_path()
        && path.exists()
        && let Ok(content) = std::fs::read_to_string(&path)
    {
        return toml::from_str(&content).unwrap_or_else(|e| {
            eprintln!(
                "warning: failed to parse {}: {e} (using defaults)",
                path.display()
            );
            Config::default()
        });
    }

    // Fall back to legacy .npkillrc if present for backwards compatibility
    if let Some(npkillrc) = legacy_npkillrc_path()
        && npkillrc.exists()
        && let Ok(content) = std::fs::read_to_string(&npkillrc)
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
    {
        let mut cfg = Config::default();
        if let Some(arr) = val.get("exclude").and_then(|v| v.as_array()) {
            let excludes: Vec<String> = arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
            if !excludes.is_empty() {
                cfg.default_exclude_types = Some(excludes);
            }
        }
        return cfg;
    }

    Config::default()
}

pub fn save_config(config: &Config) -> std::io::Result<PathBuf> {
    let path = config_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not determine config directory",
        )
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let toml_str = toml::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, toml_str)?;
    Ok(path)
}

pub fn init_default_config() -> std::io::Result<(PathBuf, bool)> {
    let path = config_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not determine config directory",
        )
    })?;
    if path.exists() {
        return Ok((path, false));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let template = r#"# nodewipe configuration file
# Location: ~/.config/nodewipe/config.toml (or OS equivalent)

# Default deletion mode: "trash" (safe, recoverable), "archive" (tar.gz backup), or "permanent"
default_delete_mode = "trash"

# Default artifact types to exclude from scanning
# default_exclude_types = ["rust_target", "generic_dist"]

# Minimum directory size (in megabytes) to show by default
default_min_mb = 0

# Default sorting order: "size" (biggest first) or "age" (oldest first)
default_sort = "size"
"#;
    std::fs::write(&path, template)?;
    Ok((path, true))
}
