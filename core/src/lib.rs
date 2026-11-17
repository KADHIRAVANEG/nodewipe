pub mod config;
pub mod dedupe;
pub mod deleter;
pub mod doctor;
pub mod git;
pub mod history;
pub mod ignorefile;
pub mod interner;
pub mod model;
pub mod rebuild;
pub mod reflink;
pub mod scanner;
pub mod stale;
pub mod workspace;

pub use config::{Config, config_path, init_default_config, load_config, save_config};
pub use dedupe::{DedupeOptions, DedupeStats, dedupe_paths};
pub use deleter::{DeleteMode, DeleteResult, DiscoveredArchive, delete, find_archives, restore};
pub use git::{annotate_git_status, find_git_repo_root, inspect_git_repo};
pub use history::{
    EcosystemStats, HistoryEntry, append as history_append, load as history_load,
    stats_by_ecosystem,
};
pub use ignore::gitignore::Gitignore;
pub use ignorefile::{
    IgnoreSource, IgnoredPattern, add_ignored_pattern, add_ignored_pattern_to_file,
    list_ignored_patterns, list_ignored_patterns_from_file, load_patterns as load_ignore_patterns,
    remove_ignored_pattern, remove_ignored_pattern_from_file, remove_ignored_pattern_from_root,
};
pub use interner::PathInterner;
pub use model::{ArtifactKind, Entry, GitStatus, PackageManager, ScanOptions, WorkspaceGroup};
pub use rebuild::{RebuildPlan, resolve_rebuild_plan};
pub use reflink::{ReflinkDirStats, reflink_dir, supports_reflink};
pub use scanner::{classify_path, scan};
pub use stale::{StaleProject, find_stale_projects};
pub use workspace::{annotate_workspace_roots, group_by_workspace};
