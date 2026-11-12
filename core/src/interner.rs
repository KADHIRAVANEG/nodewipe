use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// A thread-safe string interner that deduplicates directory path prefixes
/// across scans containing thousands of paths, minimizing heap fragmentation
/// and keeping memory overhead low.
#[derive(Debug, Default)]
pub struct PathInterner {
    pool: RwLock<HashSet<Arc<str>>>,
}

impl PathInterner {
    pub fn new() -> Self {
        Self {
            pool: RwLock::new(HashSet::new()),
        }
    }

    /// Interns a string slice, returning an `Arc<str>`. If already present,
    /// returns the existing instance without allocating a new string.
    pub fn intern(&self, s: &str) -> Arc<str> {
        if let Ok(read) = self.pool.read()
            && let Some(existing) = read.get(s)
        {
            return existing.clone();
        }
        if let Ok(mut write) = self.pool.write() {
            if let Some(existing) = write.get(s) {
                return existing.clone();
            }
            let arc: Arc<str> = Arc::from(s);
            write.insert(arc.clone());
            arc
        } else {
            Arc::from(s)
        }
    }

    /// Returns the number of unique strings currently interned.
    pub fn len(&self) -> usize {
        self.pool.read().map(|p| p.len()).unwrap_or(0)
    }

    /// Returns true if no strings are interned.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Interns a path's parent directory prefix, sharing identical parent
    /// paths across all children.
    pub fn intern_path(&self, path: &Path) -> PathBuf {
        if let Some(parent) = path.parent()
            && let Some(parent_str) = parent.to_str()
        {
            let interned_parent = self.intern(parent_str);
            if let Some(file_name) = path.file_name() {
                return Path::new(&*interned_parent).join(file_name);
            }
        }
        path.to_path_buf()
    }
}
