use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum IgnoreSource {
    Global,
    Local(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IgnoredPattern {
    pub pattern: String,
    pub source: IgnoreSource,
}

pub fn load_patterns(root: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(root);

    if let Some(home) = dirs::home_dir() {
        let global_ignore = home.join(".nodewipeignore");
        if global_ignore.exists() {
            let _ = builder.add(global_ignore);
        }
    }

    let local_ignore = root.join(".nodewipeignore");
    if local_ignore.exists() {
        let _ = builder.add(local_ignore);
    }

    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// Adds a path to the global ~/.nodewipeignore file safely.
/// Returns an error if the path points to root or is invalid.
pub fn add_ignored_pattern(target: &Path) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(target).unwrap_or_else(|_| {
        if target.is_relative() {
            std::env::current_dir()
                .map(|cwd| cwd.join(target))
                .unwrap_or_else(|_| target.to_path_buf())
        } else {
            target.to_path_buf()
        }
    });

    let s = canonical.to_string_lossy();
    if s == "/" || s == "\\" || canonical.components().count() <= 1 {
        return Err("Refusing to ignore root filesystem directory".to_string());
    }

    let home = dirs::home_dir().ok_or_else(|| "Cannot find user home directory".to_string())?;
    let global_ignore = home.join(".nodewipeignore");

    let existing = std::fs::read_to_string(&global_ignore).unwrap_or_default();
    let pattern_str = canonical.display().to_string();

    if existing.lines().any(|l| l.trim() == pattern_str) {
        return Ok(global_ignore);
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&global_ignore)
        .map_err(|e| format!("Failed to open {}: {e}", global_ignore.display()))?;

    writeln!(file, "{pattern_str}")
        .map_err(|e| format!("Failed to write to {}: {e}", global_ignore.display()))?;

    Ok(global_ignore)
}

/// Lists all patterns from ~/.nodewipeignore and ./nodewipeignore.
pub fn list_ignored_patterns(root: &Path) -> Vec<IgnoredPattern> {
    let mut results = Vec::new();

    if let Some(home) = dirs::home_dir() {
        let global_ignore = home.join(".nodewipeignore");
        if let Ok(content) = std::fs::read_to_string(&global_ignore) {
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    results.push(IgnoredPattern {
                        pattern: trimmed.to_string(),
                        source: IgnoreSource::Global,
                    });
                }
            }
        }
    }

    let local_ignore = root.join(".nodewipeignore");
    if let Ok(content) = std::fs::read_to_string(&local_ignore) {
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                results.push(IgnoredPattern {
                    pattern: trimmed.to_string(),
                    source: IgnoreSource::Local(local_ignore.clone()),
                });
            }
        }
    }

    results
}

/// Removes a pattern from a specific ignore file.
pub fn remove_ignored_pattern_from_file(ignore_file: &Path, pattern: &str) -> Result<bool, String> {
    if !ignore_file.exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(ignore_file)
        .map_err(|e| format!("Failed to read {}: {e}", ignore_file.display()))?;

    let mut found = false;
    let mut new_lines = Vec::new();
    for line in content.lines() {
        if line.trim() == pattern.trim() {
            found = true;
        } else {
            new_lines.push(line);
        }
    }

    if found {
        let mut new_content = new_lines.join("\n");
        if !new_content.is_empty() {
            new_content.push('\n');
        }
        std::fs::write(ignore_file, new_content)
            .map_err(|e| format!("Failed to write {}: {e}", ignore_file.display()))?;
    }

    Ok(found)
}

/// Removes a pattern from either the local `./.nodewipeignore` under `root` or the global `~/.nodewipeignore`.
pub fn remove_ignored_pattern_from_root(root: &Path, pattern: &str) -> Result<bool, String> {
    let mut removed = false;

    let local_ignore = root.join(".nodewipeignore");
    if local_ignore.exists() && remove_ignored_pattern_from_file(&local_ignore, pattern)? {
        removed = true;
    }

    if let Some(home) = dirs::home_dir() {
        let global_ignore = home.join(".nodewipeignore");
        if global_ignore.exists() && remove_ignored_pattern_from_file(&global_ignore, pattern)? {
            removed = true;
        }
    }

    Ok(removed)
}

/// Removes a pattern from ~/.nodewipeignore or the current directory's .nodewipeignore.
pub fn remove_ignored_pattern(pattern: &str) -> Result<bool, String> {
    remove_ignored_pattern_from_root(Path::new("."), pattern)
}

pub fn is_ignored(path: &Path, ignore: &Gitignore) -> bool {
    if ignore.is_empty() {
        return false;
    }

    let root = ignore.path();

    // The ignore crate panics with "path is expected to be under the root"
    // if `path` is absolute and does not begin with `ignore.path()`.
    // If the path is not under the matcher's root, it cannot be ignored by it.
    if path.has_root() && (!root.has_root() || !path.starts_with(root)) {
        return false;
    }

    // matched_path_or_any_parents tells us if the path or its ancestors are ignored
    let is_dir = true; // nodewipe scanner only passes directories to this function
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        matches!(
            ignore.matched_path_or_any_parents(path, is_dir),
            ignore::Match::Ignore(_)
        )
    }))
    .unwrap_or(false)
}
