pub mod config;
pub mod dedupe;
pub mod deleter;
pub mod doctor;
pub mod explain;
pub mod git;
pub mod history;
pub mod ignorefile;
pub mod interner;
pub mod model;
pub mod pin;
pub mod profile;
pub mod rebuild;
pub mod reclaim;
pub mod reflink;
pub mod scanner;
pub mod stale;
pub mod workspace;

pub use config::{
    Config, Preset, config_path, get_preset, init_default_config, list_presets, load_config,
    remove_preset, save_config, save_preset,
};
pub use dedupe::{DedupeOptions, DedupeStats, dedupe_paths};
pub use deleter::{DeleteMode, DeleteResult, DiscoveredArchive, delete, find_archives, restore};
pub use explain::{Explanation, RiskLevel, SourceStaleness, explain_entry, explain_path};
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
pub use model::{
    ArtifactKind, Entry, GitStatus, PackageManager, ScanOptions, WorkspaceGroup, icons,
};
pub use pin::{
    PinnedProject, add_pinned_project, is_path_pinned, is_path_pinned_with_list,
    list_pinned_projects, load_pinned_projects, remove_pinned_project, save_pinned_projects,
    toggle_pinned_project,
};
pub use profile::ProfileKind;
pub use rebuild::{RebuildPlan, resolve_rebuild_plan};
pub use reclaim::{ReclaimCandidate, ReclaimPlan, format_bytes, parse_size_to_bytes, plan_reclaim};
pub use reflink::{ReflinkDirStats, reflink_dir, supports_reflink};
pub use scanner::{classify_path, scan};
pub use stale::{StaleProject, find_stale_projects};
pub use workspace::{annotate_workspace_roots, group_by_workspace};
