pub mod backup;
pub mod catalog;
pub mod config;
pub mod containers;
pub mod custom_rules;
pub mod daemon;
pub mod dedupe;
pub mod deleter;
pub mod diff;
pub mod doctor;
pub mod explain;
pub mod git;
pub mod history;
pub mod ignorefile;
pub mod interner;
pub mod model;
pub mod pin;
pub mod plan;
pub mod profile;
pub mod rebuild;
pub mod reclaim;
pub mod reflink;
pub mod scanner;
pub mod stale;
pub mod timeline;
pub mod tree;
pub mod workspace;

pub use backup::{BackupInfo, PruneReport, list_backups, prune_backups, restore_backup_archive};
pub use catalog::{
    CatalogEntry, EcosystemCategory, by_ecosystem, catalog, get_by_kind, get_by_name, get_by_slug,
};
pub use config::{
    Config, Preset, config_path, get_preset, init_default_config, list_presets, load_config,
    remove_preset, save_config, save_preset, set_config_path_override,
};
pub use containers::{
    ContainerArtifact, ContainerCacheKind, ContainerCleaner, ContainerProviderKind,
    ContainerPruneReport, ContainerScanOptions, ContainerScanReport, DockerEngineAdapter,
    PodmanAdapter, discover_container_adapters, prune_container_caches, scan_container_caches,
};
pub use custom_rules::{
    CustomRule, generate_default_rules_template, global_rules_file, load_active_rules,
    load_global_rules, load_project_rules, load_rules_from_file, load_rules_from_str,
    save_rules_to_file, validate_custom_rule,
};
pub use daemon::{
    DaemonConfig, DaemonRunner, DaemonStatusInfo, DiskPressureAlert, DiskSpaceInfo,
    WorkspaceWatcher, check_daemon_status, evaluate_disk_pressure, get_disk_space,
    install_daemon_service, is_pid_alive, read_pid, remove_pid, terminate_process,
    uninstall_daemon_service, write_pid,
};
pub use dedupe::{DedupeOptions, DedupeStats, dedupe_paths};
pub use deleter::{DeleteMode, DeleteResult, DiscoveredArchive, delete, find_archives, restore};
pub use diff::{
    ScanDiff, ScanSnapshot, ScanSnapshotEntry, SizeDrift, compute_scan_diff, default_snapshot_file,
    load_snapshot, save_snapshot,
};
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
pub use plan::{
    CleanupPlan, PlanExecutionReport, PlanItem, apply_cleanup_plan, create_cleanup_plan,
    load_plan_from_file, save_plan_to_file,
};
pub use profile::ProfileKind;
pub use rebuild::{RebuildPlan, resolve_rebuild_plan};
pub use reclaim::{ReclaimCandidate, ReclaimPlan, format_bytes, parse_size_to_bytes, plan_reclaim};
pub use reflink::{
    CloneRangeChunk, CowBackend, CowCapabilities, ReflinkDirStats, align_down_to_cluster,
    align_up_to_cluster, plan_aligned_clone_chunks, probe_cow_capabilities, reflink_batch,
    reflink_dir, reflink_file_strict, supports_reflink,
};
pub use scanner::{classify_path, classify_path_with_custom, scan};
pub use stale::{StaleProject, find_stale_projects};
pub use timeline::{ProjectTimeline, TimelineVerdict, format_timeline_text, inspect_timeline};
pub use tree::{ArtifactTree, TreeNode, inspect_artifact_tree};
pub use workspace::{
    CacheKind, CachePruneItem, CachePruneOptions, CachePruneReport, CachePruneStatus,
    WorkspaceAnalysis, WorkspaceCacheSummary, WorkspaceManifest, WorkspacePackage,
    analyze_workspace, annotate_workspace_roots, group_by_workspace, parse_workspace_manifest,
    prune_monorepo_caches,
};
