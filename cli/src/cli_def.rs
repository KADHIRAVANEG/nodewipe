/// Shared CLI definition — imported by both `main.rs` (for runtime parsing)
/// and `build.rs` (for compile-time completion/man-page generation).
///
/// Keep this file free of any runtime-only imports (`nodewipe_core`, `tui`,
/// etc.) so `build.rs` can include it without pulling in the whole crate.
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

pub const BANNER: &str = r#"
 _   _           _    __        ___            
| \ | | ___   __| | __\ \      / (_)_ __   ___ 
|  \| |/ _ \ / _` |/ _ \ \ /\ / /| | '_ \ / _ \
| |\  | (_) | (_| |  __/\ V  V / | | |_) |  __/
|_| \_|\___/ \__,_|\___| \_/\_/  |_| .__/ \___|
                                   |_|         
"#;

#[derive(Parser)]
#[command(
    name = "nodewipe",
    version,
    about = "Find and reclaim disk space from stray dev-dependency and build-artifact directories",
    long_about = concat!(
        "nodewipe scans your filesystem for disposable build artifacts and dev-dependency\n",
        "directories (node_modules, Python venvs, Rust target/, Java build output, Go caches,\n",
        "and more) and helps you delete them safely.\n\n",
        "Run without arguments to launch the interactive TUI. Use subcommands for\n",
        "scriptable/headless operation."
    ),
    before_help = BANNER,
    after_help = "Full documentation: https://github.com/KADHIRAVANEG/nodewipe"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Directory to scan, e.g. `nodewipe ~` or `nodewipe /srv`.
    /// Takes priority over --root if both are given.
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Root directory to scan (prefer the positional form above).
    #[arg(long, global = true, default_value = ".")]
    pub root: PathBuf,

    /// Emit machine-readable JSON instead of a human table.
    /// Safe for scripts and CI pipelines.
    #[arg(long, global = true)]
    pub json: bool,

    /// Comma-separated artifact types to skip.
    /// Valid values: node_modules, venv, pycache, pytest_cache, mypy_cache,
    /// ruff_cache, rust_target, maven_target, gradle_build, next_cache,
    /// turbo_cache, dist, go_mod_cache, go_build_cache, php_vendor,
    /// dotnet_bin, dotnet_obj, hf_cache, ollama_models, torch_cache, triton_cache,
    /// derived_data, swiftpm_build, cocoapods, gradle_cache, android_build_cache,
    /// playwright_browsers, cypress_cache, astro_cache, sveltekit_cache,
    /// nuxt_cache, docusaurus_cache, vite_cache, cmake_build, zig_cache,
    /// elixir_build, meson_build, pnpm_ignored.
    #[arg(long, global = true, value_delimiter = ',', value_name = "TYPES")]
    pub exclude_types: Vec<String>,

    /// Comma-separated artifact types to target exclusively (inverse of --exclude-types).
    /// When specified, only these artifact types will be scanned. Aliased as `--target`.
    #[arg(
        long,
        global = true,
        value_delimiter = ',',
        value_name = "TYPES",
        alias = "target"
    )]
    pub types: Vec<String>,

    /// Alias for --types.
    #[arg(long, global = true, value_delimiter = ',', value_name = "TYPES")]
    pub include_types: Vec<String>,

    /// Skip loading .nodewipeignore files.
    #[arg(long, global = true)]
    pub no_ignore_file: bool,

    /// Force single-threaded directory traversal (sequential fallback).
    #[arg(long, global = true)]
    pub serial: bool,

    /// Skip Git repository status inspection (branch, dirty status, merged check).
    #[arg(long, global = true)]
    pub no_git: bool,

    /// Cleanup profile to filter artifacts by cleanup intent (old-dependencies, build-outputs, large-models, safe-sweep).
    #[arg(long, global = true, value_enum)]
    pub profile: Option<ProfileArg>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Scan for disposable artifact directories (default if no subcommand given).
    ///
    /// Walks the target directory tree and lists every recognised build artifact
    /// or dev-dependency directory, with its size and last-modified age.
    Scan {
        /// Directory to scan, e.g. `nodewipe scan ~`.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,

        /// Only show entries at least this many megabytes in size.
        /// Defaults to config.default_min_mb (or 0 if unset).
        #[arg(long, value_name = "MB")]
        min_mb: Option<u64>,

        /// Group output by monorepo/workspace root instead of a flat list.
        #[arg(long)]
        grouped: bool,

        /// Only show artifacts whose last modification is older than this,
        /// e.g. `30d`, `2w`, `6m`, `1y`.
        #[arg(long, value_name = "AGE")]
        older_than: Option<String>,

        /// Primary sort order for results.
        /// `size` shows biggest first; `age` shows oldest (most-abandoned) first.
        /// Defaults to config.default_sort (or `size` if unset).
        #[arg(long, value_enum)]
        sort: Option<SortKey>,

        /// Flag entire projects as likely-abandoned when their source files
        /// haven't been touched in this many days.
        #[arg(long, value_name = "DAYS")]
        stale_days: Option<u64>,

        /// Preset to filter artifacts by development ecosystem.
        #[arg(long, value_enum)]
        preset: Option<PresetArg>,

        /// Export scan results to a file (JSON or CSV based on extension or content).
        #[arg(long, value_name = "FILE")]
        export: Option<PathBuf>,
    },

    /// Delete one or more artifact directories.
    ///
    /// Requires --yes for non-interactive use. Use --dry-run to preview
    /// what would be deleted without making any changes.
    Delete {
        /// Paths to delete. Must be recognised artifact directories.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// How to delete: trash (default, recoverable), archive (tar.gz backup),
        /// or permanent (unrecoverable).
        #[arg(long, value_enum)]
        mode: Option<DeleteModeArg>,

        /// Skip the confirmation prompt (required for non-interactive/CI use).
        #[arg(long)]
        yes: bool,

        /// Show what would be deleted without deleting anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// Restore a .tar.gz backup created by `delete --mode archive`.
    Restore {
        /// Path to the -backup.tar.gz file to restore.
        #[arg(value_name = "ARCHIVE")]
        archive: PathBuf,
    },

    /// List every supported artifact type and its slug (for --exclude-types).
    Types,

    /// Launch the desktop GUI.
    ///
    /// Looks for a nodewipe-gui binary or AppImage installed alongside this CLI.
    Gui,

    /// Scan your system and show a breakdown of disk usage by ecosystem.
    ///
    /// Reports how many Rust target/ dirs, Python venvs, node_modules, etc.
    /// exist, how much space each ecosystem consumes in total, and which
    /// artifact in each category is the oldest (most likely abandoned).
    ///
    /// Example: nodewipe doctor ~
    Doctor {
        /// Directory to scan, e.g. `nodewipe doctor ~`.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,

        /// Threshold size to trigger alert/notification, e.g. "20GB", "500MB", "10G".
        #[arg(long, value_name = "SIZE")]
        threshold: Option<String>,

        /// Send a desktop notification if reclaimable disk space exceeds the threshold.
        #[arg(long)]
        notify: bool,

        /// Install background service scheduler (systemd user timer on Linux, LaunchAgent on macOS).
        #[arg(long)]
        install_service: bool,

        /// Uninstall background service scheduler.
        #[arg(long)]
        uninstall_service: bool,
    },

    /// Show a dashboard of how much disk space you have freed over time.
    ///
    /// Reads ~/.local/share/nodewipe/history.json, which is written every
    /// time you run `nodewipe delete`. Shows a per-ecosystem breakdown and
    /// a bar chart of the last N days.
    Stats {
        /// How many past days to include in the bar chart.
        #[arg(long, default_value_t = 30)]
        days: u64,

        /// Unused positional (accepted for UX consistency with other subcommands).
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },

    /// Generate shell completions for the given shell.
    ///
    /// Example: nodewipe completions bash > ~/.local/share/bash-completion/completions/nodewipe
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Reinstall dependencies or rebuild a project using its detected toolchain.
    ///
    /// Detects package manager / compiler (e.g. `npm install`, `pnpm install`,
    /// `cargo build`, `python3 -m venv .venv`, `mvn package`, `gradle build`) and
    /// runs it in the target directory.
    Rebuild {
        /// Path to the artifact or project directory.
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,
    },

    /// Deduplicate identical files across project directories using hardlinks or reflinks.
    ///
    /// Finds duplicate dependencies and assets across multiple projects and replaces
    /// them with Copy-on-Write reflinks or filesystem hardlinks, reclaiming storage
    /// without breaking any projects.
    Dedupe {
        /// Target directories to scan and deduplicate (defaults to current directory).
        #[arg(value_name = "PATHS")]
        paths: Vec<PathBuf>,

        /// Preview how much disk space would be reclaimed without modifying files.
        #[arg(long)]
        dry_run: bool,

        /// Minimum file size in bytes to consider for deduplication (default: 64).
        #[arg(long, default_value_t = 64)]
        min_size: u64,

        /// Allow hardlinks when Copy-on-Write (reflink) is unsupported by the filesystem.
        /// WARNING: Hardlinks cause subsequent in-place edits to affect all linked files.
        #[arg(long)]
        allow_hardlinks: bool,
    },

    /// Manage ignored directory patterns (.nodewipeignore).
    Ignore {
        #[command(subcommand)]
        command: IgnoreCommand,
    },

    /// Render or install the manual page for nodewipe.
    ///
    /// By default, views the man page or writes to stdout. Pass --install to
    /// automatically copy it to ~/.local/share/man/man1/nodewipe.1.
    Man {
        /// Install man page to user man directory (~/.local/share/man/man1/).
        #[arg(long)]
        install: bool,
    },

    /// View, locate, or initialize nodewipe configuration settings.
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },

    /// Check for and install the latest release of nodewipe.
    Update {
        /// Check for newer versions without downloading or installing.
        #[arg(long)]
        check: bool,

        /// Automatically proceed with installation without prompting.
        #[arg(short, long)]
        yes: bool,

        /// Force installation even if already on the latest version.
        #[arg(long)]
        force: bool,
    },

    /// Explain an artifact directory: why it matched, risk level, staleness, and rebuild recipe.
    Explain {
        /// Path to the artifact or project directory to inspect.
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,
    },

    /// Manage pinned projects to protect them from cleanup and deletion.
    Pin {
        #[command(subcommand)]
        command: Option<PinCommand>,

        /// Path to pin directly without typing 'add' (e.g. `nodewipe pin .`).
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },

    /// Target a specific amount of disk space to reclaim (e.g. `nodewipe reclaim 10GB`).
    ///
    /// Scans the target directory, ranks disposable artifacts by risk, staleness,
    /// and size, and proposes an optimal deletion set with an interactive confirmation preview.
    Reclaim {
        /// Amount of disk space to reclaim (e.g. "10GB", "500MB", "2G", "1.5GB").
        #[arg(value_name = "TARGET")]
        target: String,

        /// Directory to search for reclaimable artifacts (defaults to current directory).
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,

        /// How to delete: trash (default, recoverable), archive (tar.gz backup), or permanent.
        #[arg(long, value_enum)]
        mode: Option<DeleteModeArg>,

        /// Skip confirmation prompt and execute immediately.
        #[arg(short, long)]
        yes: bool,

        /// Preview the proposed cleanup plan without deleting anything.
        #[arg(long)]
        dry_run: bool,

        /// Prefer local compiler rebuilds (Low risk) over re-downloading dependencies.
        #[arg(long, default_value_t = true)]
        prefer_local_rebuild: bool,
    },

    /// Manage and run repeatable scan workflows / presets.
    Preset {
        #[command(subcommand)]
        command: Option<PresetCommand>,

        /// Name of a preset to run directly (e.g. `nodewipe preset work`).
        #[arg(value_name = "NAME")]
        name: Option<String>,
    },

    /// Inspect the internal storage layout and largest subdirectories of an artifact directory.
    Tree {
        /// Path to the artifact directory (e.g. `target/`, `node_modules/`).
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,

        /// Maximum directory depth to inspect.
        #[arg(long, default_value_t = 2)]
        depth: usize,

        /// Maximum number of largest children to display per directory.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },

    /// Compare source modification, git commit, and artifact build timestamps to evaluate project staleness.
    Timeline {
        /// Path to the project or artifact directory.
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,
    },

    /// Analyze monorepo workspaces and report member packages, artifact footprints, and shared dependencies.
    Workspace {
        /// Path to the monorepo root or child directory.
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum PresetCommand {
    /// Save a new repeatable scan workflow preset.
    Save {
        /// Unique name for the preset.
        #[arg(value_name = "NAME")]
        name: String,

        /// Scan path for this preset.
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,

        /// Cleanup profile to associate with this preset.
        #[arg(long, value_enum)]
        profile: Option<ProfileArg>,

        /// Comma-separated types to exclude.
        #[arg(long, value_delimiter = ',', value_name = "TYPES")]
        exclude_types: Vec<String>,

        /// Minimum directory size in megabytes.
        #[arg(long, value_name = "MB")]
        min_mb: Option<u64>,

        /// Age filter (e.g. "30d", "2m", "1y").
        #[arg(long, value_name = "AGE")]
        older_than: Option<String>,

        /// Sort order (size or age).
        #[arg(long, value_enum)]
        sort: Option<SortKey>,

        /// Optional description for this preset.
        #[arg(short, long)]
        description: Option<String>,
    },
    /// List all saved workflow presets.
    List,
    /// Run a saved workflow preset by name.
    Run {
        /// Name of the preset to run.
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Remove a saved workflow preset.
    Remove {
        /// Name of the preset to delete.
        #[arg(value_name = "NAME")]
        name: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ProfileArg {
    /// Stale dependency directories (>60d) from inactive projects.
    OldDependencies,
    /// Compiler, bundler, and build outputs that rebuild locally.
    BuildOutputs,
    /// Large AI/ML model weights, checkpoints, and compiler caches.
    LargeModels,
    /// Zero-risk local artifacts with no network download requirements.
    SafeSweep,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum PinCommand {
    /// Pin a project directory to protect it from deletion.
    Add {
        /// Path to the project directory.
        #[arg(value_name = "PATH")]
        path: PathBuf,

        /// Optional reason or note for pinning this project.
        #[arg(short, long)]
        reason: Option<String>,
    },
    /// List all currently pinned projects.
    List,
    /// Unpin a project directory to allow cleanup.
    Remove {
        /// Path to the project directory to unpin.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ConfigAction {
    /// Print the path to the configuration file (~/.config/nodewipe/config.toml).
    Path,
    /// Display current active configuration settings.
    Show,
    /// Initialize a default config.toml file if it does not already exist.
    Init,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum DeleteModeArg {
    /// Move to the OS trash (recoverable via your file manager).
    Trash,
    /// Create a .tar.gz backup, then delete.
    Archive,
    /// Delete immediately with no backup. Cannot be undone.
    Permanent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum SortKey {
    /// Sort by size, largest first (default).
    Size,
    /// Sort by last-modified date, oldest first.
    /// Surfaces the most abandoned artifacts at the top of the list.
    Age,
    /// Sort alphabetically by Git branch name.
    Git,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PresetArg {
    /// JavaScript/TypeScript/Frontend ecosystems (node_modules, next, turbo, dist, astro, svelte, nuxt, vite, playwright, cypress).
    Node,
    /// Web frontend ecosystems (alias for node).
    Web,
    /// Frontend framework ecosystems (alias for node).
    Frontend,
    /// Python ecosystems (venv, __pycache__, pytest, mypy, ruff).
    Python,
    /// Rust/Cargo ecosystem (target).
    Rust,
    /// Java/JVM ecosystems (Maven target, Gradle build).
    Java,
    /// Go ecosystem (mod cache, build cache).
    Go,
    /// .NET ecosystem (bin, obj).
    Dotnet,
    /// AI & Machine Learning ecosystems (Hugging Face, Ollama, PyTorch, Triton).
    Ai,
    /// AI & Machine Learning ecosystems (alias for AI).
    Ml,
    /// Mobile & Apple development ecosystems (Xcode DerivedData, SwiftPM, CocoaPods, Gradle cache, Android cache).
    Mobile,
    /// Apple development ecosystem (alias for mobile).
    Apple,
    /// iOS development ecosystem (alias for mobile).
    Ios,
    /// Android development ecosystem (alias for mobile).
    Android,
    /// Systems and Native programming ecosystems (CMake, Ninja, Zig, Elixir, Meson, Rust).
    Systems,
    /// Native compiled ecosystems (alias for systems).
    Native,
    /// C/C++ compilation ecosystems (alias for systems).
    Cpp,
    /// C compilation ecosystems (alias for systems).
    C,
}

#[derive(Subcommand, Clone, Debug)]
pub enum IgnoreCommand {
    /// Add a directory path to ~/.nodewipeignore.
    Add {
        /// Directory path to ignore.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    /// List all currently configured ignore patterns.
    List,
    /// Remove an ignored pattern from ~/.nodewipeignore.
    Remove {
        /// Pattern to remove.
        #[arg(value_name = "PATTERN")]
        pattern: String,
    },
}
