use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;

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
