use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PinnedProject {
    pub path: PathBuf,
    pub pinned_at_unix: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub fn pinned_file_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("nodewipe").join("pinned.json"))
}

pub fn load_pinned_projects() -> Vec<PinnedProject> {
    if let Some(path) = pinned_file_path() {
        load_pinned_projects_from_file(&path)
    } else {
        Vec::new()
    }
}

pub fn list_pinned_projects() -> Vec<PinnedProject> {
    load_pinned_projects()
}

pub fn load_pinned_projects_from_file(file_path: &Path) -> Vec<PinnedProject> {
    if !file_path.exists() {
        return Vec::new();
    }
    match fs::read_to_string(file_path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn save_pinned_projects(list: &[PinnedProject]) -> Result<()> {
    let path = pinned_file_path().context("Could not determine config directory for pinned projects")?;
    save_pinned_projects_to_file(&path, list)
}

pub fn save_pinned_projects_to_file(file_path: &Path, list: &[PinnedProject]) -> Result<()> {
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(list)?;
    fs::write(file_path, content)?;
    Ok(())
}

/// Checks if `target` or any of its parent directories is pinned using the default configuration file.
pub fn is_path_pinned(target: &Path) -> bool {
    let pinned = load_pinned_projects();
    is_path_pinned_with_list(target, &pinned)
}

/// Checks if `target` matches or is contained inside any pinned project in `pinned`.
pub fn is_path_pinned_with_list(target: &Path, pinned: &[PinnedProject]) -> bool {
    let target_canon = fs::canonicalize(target).unwrap_or_else(|_| {
        if target.is_relative() {
            std::env::current_dir()
                .map(|cwd| cwd.join(target))
                .unwrap_or_else(|_| target.to_path_buf())
        } else {
            target.to_path_buf()
        }
    });

    for p in pinned {
        let p_canon = fs::canonicalize(&p.path).unwrap_or_else(|_| p.path.clone());
        if target_canon == p_canon || target_canon.starts_with(&p_canon) {
            return true;
        }
    }
    false
}

pub fn add_pinned_project(target: &Path, reason: Option<String>) -> Result<PathBuf> {
    let file_path = pinned_file_path().context("Could not determine config directory for pinned projects")?;
    add_pinned_project_to_file(&file_path, target, reason)
}

pub fn add_pinned_project_to_file(file_path: &Path, target: &Path, reason: Option<String>) -> Result<PathBuf> {
    let canonical = fs::canonicalize(target).unwrap_or_else(|_| {
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
        anyhow::bail!("Refusing to pin root filesystem directory: {}", canonical.display());
    }

    let mut list = load_pinned_projects_from_file(file_path);
    let already_pinned = list.iter().any(|item| {
        fs::canonicalize(&item.path).unwrap_or_else(|_| item.path.clone()) == canonical
    });

    if !already_pinned {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        list.push(PinnedProject {
            path: canonical.clone(),
            pinned_at_unix: now,
            reason,
        });
        save_pinned_projects_to_file(file_path, &list)?;
    }

    Ok(canonical)
}

pub fn remove_pinned_project(target: &Path) -> Result<bool> {
    let file_path = pinned_file_path().context("Could not determine config directory for pinned projects")?;
    remove_pinned_project_from_file(&file_path, target)
}

pub fn remove_pinned_project_from_file(file_path: &Path, target: &Path) -> Result<bool> {
    let canonical = fs::canonicalize(target).unwrap_or_else(|_| {
        if target.is_relative() {
            std::env::current_dir()
                .map(|cwd| cwd.join(target))
                .unwrap_or_else(|_| target.to_path_buf())
        } else {
            target.to_path_buf()
        }
    });

    let mut list = load_pinned_projects_from_file(file_path);
    let initial_len = list.len();
    list.retain(|item| {
        let item_canon = fs::canonicalize(&item.path).unwrap_or_else(|_| item.path.clone());
        item_canon != canonical && !canonical.starts_with(&item_canon)
    });

    if list.len() < initial_len {
        save_pinned_projects_to_file(file_path, &list)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Toggles pin status on `target`. If pinned, removes it; if unpinned, pins it.
/// Returns (is_now_pinned, resolved_path).
pub fn toggle_pinned_project(target: &Path) -> Result<(bool, PathBuf)> {
    let file_path = pinned_file_path().context("Could not determine config directory for pinned projects")?;
    toggle_pinned_project_in_file(&file_path, target)
}

pub fn toggle_pinned_project_in_file(file_path: &Path, target: &Path) -> Result<(bool, PathBuf)> {
    let list = load_pinned_projects_from_file(file_path);
    if is_path_pinned_with_list(target, &list) {
        remove_pinned_project_from_file(file_path, target)?;
        let canonical = fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
        Ok((false, canonical))
    } else {
        let canonical = add_pinned_project_to_file(file_path, target, None)?;
        Ok((true, canonical))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_pin_and_unpin_lifecycle() {
        let tmp = tempdir().expect("tempdir");
        let pin_file = tmp.path().join("pinned.json");

        let project_dir = tmp.path().join("my-cool-project");
        let artifact_dir = project_dir.join("node_modules");
        fs::create_dir_all(&artifact_dir).expect("create dirs");

        let initial_list = load_pinned_projects_from_file(&pin_file);
        assert!(initial_list.is_empty());
        assert!(!is_path_pinned_with_list(&artifact_dir, &initial_list));

        // Pin the project root
        let pinned_path = add_pinned_project_to_file(&pin_file, &project_dir, Some("Client demo".into()))
            .expect("pin project");
        assert_eq!(pinned_path, fs::canonicalize(&project_dir).unwrap());

        let updated_list = load_pinned_projects_from_file(&pin_file);
        assert_eq!(updated_list.len(), 1);
        assert_eq!(updated_list[0].reason.as_deref(), Some("Client demo"));

        // Both project root and child artifact should be recognized as pinned!
        assert!(is_path_pinned_with_list(&project_dir, &updated_list));
        assert!(is_path_pinned_with_list(&artifact_dir, &updated_list));

        // Unpin
        let removed = remove_pinned_project_from_file(&pin_file, &project_dir).expect("remove");
        assert!(removed);
        let empty_list = load_pinned_projects_from_file(&pin_file);
        assert!(empty_list.is_empty());
        assert!(!is_path_pinned_with_list(&artifact_dir, &empty_list));
    }

    #[test]
    fn test_refuse_root_pin() {
        let tmp = tempdir().expect("tempdir");
        let pin_file = tmp.path().join("pinned.json");
        let root = Path::new("/");
        let res = add_pinned_project_to_file(&pin_file, root, None);
        assert!(res.is_err());
    }
}
