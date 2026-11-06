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
    /// dotnet_bin, dotnet_obj.
    #[arg(long, global = true, value_delimiter = ',', value_name = "TYPES")]
    pub exclude_types: Vec<String>,

    /// Skip loading .nodewipeignore files.
    #[arg(long, global = true)]
    pub no_ignore_file: bool,
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
        #[arg(long, default_value_t = 0)]
        min_mb: u64,

        /// Group output by monorepo/workspace root instead of a flat list.
        #[arg(long)]
        grouped: bool,

        /// Only show artifacts whose last modification is older than this,
        /// e.g. `30d`, `2w`, `6m`, `1y`.
        #[arg(long, value_name = "AGE")]
        older_than: Option<String>,

        /// Primary sort order for results.
        /// `size` (default) shows biggest first; `age` shows oldest
        /// (most-abandoned) first, surfacing forgotten projects at the top.
        #[arg(long, value_enum, default_value_t = SortKey::Size)]
        sort: SortKey,

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

    /// Render or install the manual page for nodewipe.
    ///
    /// By default, views the man page or writes to stdout. Pass --install to
    /// automatically copy it to ~/.local/share/man/man1/nodewipe.1.
    Man {
        /// Install man page to user man directory (~/.local/share/man/man1/).
        #[arg(long)]
        install: bool,
    },
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

#[derive(Clone, Copy, ValueEnum)]
pub enum SortKey {
    /// Sort by size, largest first (default).
    Size,
    /// Sort by last-modified date, oldest first.
    /// Surfaces the most abandoned artifacts at the top of the list.
    Age,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PresetArg {
    /// JavaScript/TypeScript/Frontend ecosystems (node_modules, next, turbo, dist).
    Node,
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
}
