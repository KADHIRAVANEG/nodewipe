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

fn add_ignore_file_to_builder(builder: &mut GitignoreBuilder, file: &Path, root: &Path) {
    if let Ok(content) = std::fs::read_to_string(file) {
        let root_canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let p = Path::new(trimmed);
            if p.is_absolute() {
                let p_canonical = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
                if let Ok(rel) = p_canonical.strip_prefix(&root_canonical) {
                    let rel_str = rel.to_string_lossy();
                    if rel_str.is_empty() {
                        let _ = builder.add_line(None, "*");
                    } else {
                        let _ = builder.add_line(None, &format!("/{rel_str}"));
                        let _ = builder.add_line(None, &format!("/{rel_str}/**"));
                    }
                } else if root_canonical.starts_with(&p_canonical) {
                    // The entire root directory falls within this ignored path
                    let _ = builder.add_line(None, "*");
                }
            } else {
                let _ = builder.add_line(None, trimmed);
            }
        }
    }
}

pub fn load_patterns(root: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(root);

    if let Some(home) = dirs::home_dir() {
        let global_ignore = home.join(".nodewipeignore");
        if global_ignore.exists() {
            add_ignore_file_to_builder(&mut builder, &global_ignore, root);
        }
    }

    let local_ignore = root.join(".nodewipeignore");
    if local_ignore.exists() {
        add_ignore_file_to_builder(&mut builder, &local_ignore, root);
    }

    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// Loads patterns from a specific ignore file into a Gitignore matcher rooted at `root`.
pub fn load_patterns_from_file(root: &Path, file: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(root);
    add_ignore_file_to_builder(&mut builder, file, root);
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// Adds a path to the specified ignore file safely.
/// Returns an error if the path points to root or is invalid.
pub fn add_ignored_pattern_to_file(ignore_file: &Path, target: &Path) -> Result<PathBuf, String> {
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

    if let Some(parent) = ignore_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let existing = std::fs::read_to_string(ignore_file).unwrap_or_default();
    let pattern_str = canonical.display().to_string();

    if existing.lines().any(|l| l.trim() == pattern_str) {
        return Ok(ignore_file.to_path_buf());
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ignore_file)
        .map_err(|e| format!("Failed to open {}: {e}", ignore_file.display()))?;

    writeln!(file, "{pattern_str}")
        .map_err(|e| format!("Failed to write to {}: {e}", ignore_file.display()))?;

    Ok(ignore_file.to_path_buf())
}

/// Adds a path to the global ~/.nodewipeignore file safely.
/// Returns an error if the path points to root or is invalid.
pub fn add_ignored_pattern(target: &Path) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Cannot find user home directory".to_string())?;
    let global_ignore = home.join(".nodewipeignore");
    add_ignored_pattern_to_file(&global_ignore, target)
}

/// Lists all patterns from a specific ignore file.
pub fn list_ignored_patterns_from_file(ignore_file: &Path) -> Vec<IgnoredPattern> {
    let mut results = Vec::new();
    if let Ok(content) = std::fs::read_to_string(ignore_file) {
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
    results
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_absolute_path_ignore_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let scan_root = tmp.path().join("workspace");
        let project_dir = scan_root.join("my-project");
        let node_modules = project_dir.join("node_modules");
        std::fs::create_dir_all(&node_modules).unwrap();

        let ignore_file = tmp.path().join("test_ignore");

        // Add absolute path to ignore file
        let added = add_ignored_pattern_to_file(&ignore_file, &node_modules);
        assert!(added.is_ok());

        // Load patterns with root = scan_root
        let matcher = load_patterns_from_file(&scan_root, &ignore_file);

        // Verify that node_modules is ignored
        assert!(
            is_ignored(&node_modules, &matcher),
            "absolute path should be properly matched by load_patterns"
        );

        // Subdirectory of ignored node_modules should also be ignored
        let subpkg = node_modules.join("react");
        assert!(is_ignored(&subpkg, &matcher));

        // Unrelated directory should not be ignored
        let other_dir = project_dir.join("src");
        assert!(!is_ignored(&other_dir, &matcher));

        // Remove pattern
        let removed =
            remove_ignored_pattern_from_file(&ignore_file, &node_modules.to_string_lossy());
        assert!(removed.is_ok());
        assert!(removed.unwrap());

        let matcher_after = load_patterns_from_file(&scan_root, &ignore_file);
        assert!(!is_ignored(&node_modules, &matcher_after));
    }

    #[test]
    fn test_relative_pattern_ignore() {
        let tmp = TempDir::new().unwrap();
        let scan_root = tmp.path().join("workspace");
        let project_dir = scan_root.join("my-project");
        let target_dir = project_dir.join("target");
        std::fs::create_dir_all(&target_dir).unwrap();

        let ignore_file = scan_root.join(".nodewipeignore");
        std::fs::write(&ignore_file, "target\n").unwrap();

        let matcher = load_patterns_from_file(&scan_root, &ignore_file);
        assert!(is_ignored(&target_dir, &matcher));
    }
}
