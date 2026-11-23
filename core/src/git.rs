use crate::model::{Entry, GitStatus};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Finds the root directory of the Git repository enclosing `path`, if any.
/// Walks up the directory hierarchy starting from `path` (or its parent if `path` is an artifact).
pub fn find_git_repo_root(path: &Path) -> Option<PathBuf> {
    let mut current = path.parent()?;

    loop {
        let git_entry = current.join(".git");
        if git_entry.exists() {
            return Some(current.to_path_buf());
        }
        match current.parent() {
            Some(p) => current = p,
            None => break,
        }
    }
    None
}

/// Resolves the actual `.git` directory for a repository.
/// In standard repos, this is `repo_root.join(".git")`.
/// In git worktrees or submodules, `.git` is a file containing `gitdir: <path>`.
pub fn resolve_git_dir(repo_root: &Path) -> Option<PathBuf> {
    let git_path = repo_root.join(".git");
    if git_path.is_dir() {
        Some(git_path)
    } else if git_path.is_file() {
        let content = fs::read_to_string(&git_path).ok()?;
        for line in content.lines() {
            if let Some(rest) = line.trim().strip_prefix("gitdir:") {
                let target = PathBuf::from(rest.trim());
                let resolved = if target.is_absolute() {
                    target
                } else {
                    repo_root.join(target)
                };
                if resolved.is_dir() && resolved.join("HEAD").exists() {
                    return Some(resolved);
                }
            }
        }
        None
    } else {
        None
    }
}

/// Sanitizes a branch name extracted from .git/HEAD to prevent path traversal attacks.
/// Rejects empty names, null bytes, absolute paths, or paths containing parent directory components.
pub fn sanitize_branch_name(branch: &str) -> Option<String> {
    let trimmed = branch.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return None;
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return None;
    }
    for component in path.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return None;
        }
    }
    Some(trimmed.to_string())
}

/// Inspects git status for a specific repository root.
pub fn inspect_git_repo(repo_root: &Path) -> Option<GitStatus> {
    let git_dir = resolve_git_dir(repo_root)?;
    let head_path = git_dir.join("HEAD");
    let head_content = fs::read_to_string(&head_path).ok()?;
    let head_trimmed = head_content.trim();

    let branch = if let Some(ref_path) = head_trimmed.strip_prefix("ref: refs/heads/") {
        sanitize_branch_name(ref_path)
    } else if let Some(ref_path) = head_trimmed.strip_prefix("ref: ") {
        sanitize_branch_name(ref_path)
    } else if !head_trimmed.is_empty() {
        // Detached HEAD (commit hash)
        let short_hash = &head_trimmed[..7.min(head_trimmed.len())];
        if short_hash.chars().all(|c| c.is_ascii_hexdigit()) {
            Some(format!("detached ({short_hash})"))
        } else {
            None
        }
    } else {
        None
    };

    let last_commit_time = read_last_commit_time(&git_dir, branch.as_deref())
        .or_else(|| read_commit_time_from_cmd(repo_root));

    let is_dirty = check_is_dirty(repo_root);

    let is_merged = check_is_merged(repo_root, branch.as_deref());

    Some(GitStatus {
        branch,
        is_dirty,
        is_merged,
        last_commit_time,
    })
}

fn read_last_commit_time(git_dir: &Path, branch: Option<&str>) -> Option<u64> {
    // 1. Try reading the branch's reflog
    if let Some(b) = branch
        && let Some(safe_b) = sanitize_branch_name(b)
    {
        let branch_log = git_dir.join("logs").join("refs").join("heads").join(&safe_b);
        if let Some(ts) = parse_last_reflog_timestamp(&branch_log) {
            return Some(ts);
        }
    }

    // 2. Try reading HEAD's reflog
    let head_log = git_dir.join("logs").join("HEAD");
    if let Some(ts) = parse_last_reflog_timestamp(&head_log) {
        return Some(ts);
    }

    // 3. Fallback to mtime of branch ref file or HEAD
    if let Some(b) = branch
        && let Some(safe_b) = sanitize_branch_name(b)
    {
        let ref_file = git_dir.join("refs").join("heads").join(&safe_b);
        if let Ok(meta) = fs::metadata(&ref_file)
            && let Ok(mtime) = meta.modified()
            && let Ok(dur) = mtime.duration_since(std::time::SystemTime::UNIX_EPOCH)
        {
            return Some(dur.as_secs());
        }
    }

    None
}

fn parse_last_reflog_timestamp(log_path: &Path) -> Option<u64> {
    let content = fs::read_to_string(log_path).ok()?;
    let last_line = content.lines().rfind(|l| !l.trim().is_empty())?;
    // Git reflog line format:
    // <old-hash> <new-hash> <committer> <<email>> <timestamp> <tz>\t<message>
    let header = last_line.split('\t').next()?;
    let parts: Vec<&str> = header.split_whitespace().collect();
    if parts.len() >= 2 {
        let ts_str = parts[parts.len() - 2];
        if let Ok(ts) = ts_str.parse::<u64>() {
            return Some(ts);
        }
    }
    None
}

fn read_commit_time_from_cmd(repo_root: &Path) -> Option<u64> {
    let out = Command::new("git")
        .args(["log", "-1", "--format=%ct"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if out.status.success() {
        let text = std::str::from_utf8(&out.stdout).ok()?.trim();
        text.parse::<u64>().ok()
    } else {
        None
    }
}

fn check_is_dirty(repo_root: &Path) -> bool {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let trimmed = line.trim_start();
                if trimmed.is_empty() {
                    continue;
                }
                // If the only untracked item is a known disposable dev-artifact directory
                // (e.g. `node_modules/`, `target/`), the project source code itself is not dirty.
                if let Some(path_str) = trimmed.strip_prefix("?? ") {
                    let clean_name = path_str.trim().trim_end_matches('/');
                    if is_known_artifact_name(clean_name) {
                        continue;
                    }
                }
                return true;
            }
            false
        }
        _ => false,
    }
}

fn is_known_artifact_name(name: &str) -> bool {
    matches!(
        name,
        "node_modules"
            | "venv"
            | ".venv"
            | "__pycache__"
            | ".pytest_cache"
            | ".mypy_cache"
            | ".ruff_cache"
            | "target"
            | "build"
            | ".next"
            | ".turbo"
            | "dist"
            | "vendor"
            | "bin"
            | "obj"
            | ".astro"
            | ".svelte-kit"
            | ".nuxt"
            | ".docusaurus"
            | ".vite"
            | ".zig-cache"
            | "zig-out"
            | "_build"
            | "builddir"
            | "Pods"
            | ".build"
    )
}

fn check_is_merged(repo_root: &Path, branch: Option<&str>) -> bool {
    let branch = match branch {
        Some(b) => b,
        None => return false,
    };

    // Trunk/mainline branches are not treated as merged feature branches.
    if matches!(
        branch,
        "main" | "master" | "trunk" | "develop" | "development"
    ) || branch.starts_with("detached (")
    {
        return false;
    }

    let candidates = [
        "main",
        "master",
        "origin/main",
        "origin/master",
        "develop",
        "origin/develop",
        "trunk",
    ];

    for &target in &candidates {
        if branch == target {
            continue;
        }
        let out = Command::new("git")
            .args(["merge-base", "--is-ancestor", "HEAD", target])
            .current_dir(repo_root)
            .output();

        if let Ok(res) = out
            && res.status.success()
        {
            return true;
        }
    }

    false
}

/// Annotates entries with their enclosing Git repository status in parallel.
/// Batches by repository root so each Git repository is inspected at most once.
pub fn annotate_git_status(entries: &mut [Entry]) {
    let mut repo_to_indices: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (idx, entry) in entries.iter().enumerate() {
        if let Some(repo_root) = find_git_repo_root(&entry.path) {
            repo_to_indices.entry(repo_root).or_default().push(idx);
        }
    }

    if repo_to_indices.is_empty() {
        return;
    }

    let repo_statuses: Vec<(PathBuf, GitStatus)> = repo_to_indices
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .into_par_iter()
        .filter_map(|repo_root| inspect_git_repo(&repo_root).map(|status| (repo_root, status)))
        .collect();

    for (repo_root, status) in repo_statuses {
        if let Some(indices) = repo_to_indices.get(&repo_root) {
            for &idx in indices {
                entries[idx].git_status = Some(status.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_sanitize_branch_name() {
        assert_eq!(sanitize_branch_name("main"), Some("main".to_string()));
        assert_eq!(
            sanitize_branch_name("feature/my-cool-branch"),
            Some("feature/my-cool-branch".to_string())
        );
        assert_eq!(
            sanitize_branch_name("release/1.0.0"),
            Some("release/1.0.0".to_string())
        );

        // Path traversal attempts must be rejected
        assert_eq!(sanitize_branch_name("../../../../etc/shadow"), None);
        assert_eq!(sanitize_branch_name("/etc/passwd"), None);
        assert_eq!(sanitize_branch_name("feature/../../escape"), None);
        assert_eq!(sanitize_branch_name("branch\0with_null"), None);
        assert_eq!(sanitize_branch_name("   "), None);
        assert_eq!(sanitize_branch_name(""), None);
    }

    #[test]
    fn test_resolve_git_dir_traversal_protection() {
        let tmp = tempdir().unwrap();
        let repo_root = tmp.path();

        // Malicious .git file pointing to a non-git file
        let git_file = repo_root.join(".git");
        fs::write(&git_file, "gitdir: /etc/shadow\n").unwrap();

        let resolved = resolve_git_dir(repo_root);
        assert_eq!(resolved, None);
    }

    #[test]
    fn test_inspect_git_repo_with_malicious_head() {
        let tmp = tempdir().unwrap();
        let repo_root = tmp.path();
        let git_dir = repo_root.join(".git");
        fs::create_dir_all(&git_dir).unwrap();

        // Malicious HEAD attempting path traversal
        fs::write(
            git_dir.join("HEAD"),
            "ref: refs/heads/../../../../etc/shadow\n",
        )
        .unwrap();

        let status = inspect_git_repo(repo_root).unwrap();
        assert_eq!(status.branch, None);
    }
}
