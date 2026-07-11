use std::path::{Path, PathBuf};

/// Loads ignore patterns from `.nodewipeignore` in the scan root, plus a
/// global `~/.nodewipeignore` if present. Root-specific patterns and global
/// patterns are simply concatenated — both apply.
///
/// Format: one pattern per line. Blank lines and lines starting with `#`
/// are ignored. A pattern matches if it equals any path component (e.g.
/// `legacy-project` skips anything under a directory with that exact name)
/// or if it's a path suffix (e.g. `apps/old-site` skips that specific
/// nested path). This is intentionally simple — no glob syntax — since the
/// common case is "skip this one directory" or "skip this one project",
/// not complex pattern matching.
pub fn load_patterns(root: &Path) -> Vec<String> {
    let mut patterns = Vec::new();

    if let Some(home) = dirs::home_dir() {
        patterns.extend(read_ignore_file(&home.join(".nodewipeignore")));
    }
    patterns.extend(read_ignore_file(&root.join(".nodewipeignore")));

    patterns
}

fn read_ignore_file(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|content| {
            content
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.trim_end_matches('/').to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// True if `path` should be skipped per `patterns`. A pattern matches if
/// any ancestor path ends with it (component-wise), so both a bare
/// directory name and a multi-segment relative path work as expected.
pub fn is_ignored(path: &Path, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let path_str = path.to_string_lossy();
    patterns.iter().any(|pattern| {
        let pattern_path = PathBuf::from(pattern);
        let pattern_components: Vec<_> = pattern_path.components().collect();

        if pattern_components.len() == 1 {
            // Bare name: match any path component exactly.
            path.components().any(|c| c.as_os_str() == pattern_components[0].as_os_str())
        } else {
            // Multi-segment: match as a path suffix.
            path_str.replace('\\', "/").ends_with(&pattern.replace('\\', "/"))
        }
    })
}
