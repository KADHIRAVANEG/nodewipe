mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use nodewipe_core::{annotate_workspace_roots, delete, group_by_workspace, scan, ArtifactKind, DeleteMode, Entry, ScanOptions};
use std::path::PathBuf;
use std::process::ExitCode;

const BANNER: &str = r#"
 _   _           _    __        ___            
| \ | | ___   __| | __\ \      / (_)_ __   ___ 
|  \| |/ _ \ / _` |/ _ \ \ /\ / /| | '_ \ / _ \
| |\  | (_) | (_| |  __/\ V  V / | | |_) |  __/
|_| \_|\___/ \__,_|\___| \_/\_/  |_| .__/ \___|
                                   |_|         
"#;

/// Comma-separated list of type slugs, e.g. "venv,pycache,rust_target".
/// Scanning covers every known kind by default; this is how you opt out.
const TYPE_HELP: &str = "node_modules, venv, pycache, pytest_cache, mypy_cache, \
ruff_cache, rust_target, maven_target, gradle_build, next_cache, turbo_cache, dist";

#[derive(Parser)]
#[command(
    name = "nodewipe",
    version,
    about = "Find and reclaim disk space from stray dev-dependency and build-artifact directories",
    before_help = BANNER
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Root directory to scan.
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,

    /// Emit machine-readable JSON instead of a human table.
    /// This is the headless mode requested in npkill#188 — safe for scripts/CI.
    #[arg(long, global = true)]
    json: bool,

    /// Comma-separated artifact types to skip. Everything is scanned by
    /// default. Valid values: node_modules, venv, pycache, pytest_cache,
    /// mypy_cache, ruff_cache, rust_target, maven_target, gradle_build,
    /// next_cache, turbo_cache, dist.
    #[arg(long, global = true, value_delimiter = ',')]
    exclude_types: Vec<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Scan for disposable artifact directories (default if no subcommand given).
    Scan {
        /// Only show entries at least this many megabytes in size.
        #[arg(long, default_value_t = 0)]
        min_mb: u64,

        /// Group output by monorepo/workspace root instead of a flat list.
        #[arg(long)]
        grouped: bool,
    },
    /// Delete one or more artifact directories.
    Delete {
        /// Path(s) to delete. Must be a recognized artifact directory
        /// (node_modules, venv, __pycache__, target, build, dist, ...).
        paths: Vec<PathBuf>,

        /// How to delete: move to OS trash (recoverable), archive to .tar.gz
        /// first, or permanently remove.
        #[arg(long, value_enum, default_value_t = DeleteModeArg::Trash)]
        mode: DeleteModeArg,

        /// Skip the confirmation prompt (required for non-interactive/CI use).
        #[arg(long)]
        yes: bool,

        /// Show what would be deleted without deleting anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// List every supported artifact type and its slug (for --exclude-types).
    Types,
}

#[derive(Clone, Copy, ValueEnum)]
enum DeleteModeArg {
    Trash,
    Archive,
    Permanent,
}

impl From<DeleteModeArg> for DeleteMode {
    fn from(v: DeleteModeArg) -> Self {
        match v {
            DeleteModeArg::Trash => DeleteMode::Trash,
            DeleteModeArg::Archive => DeleteMode::Archive,
            DeleteModeArg::Permanent => DeleteMode::Permanent,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let exclude_kinds = parse_exclude_types(&cli.exclude_types)?;

    match cli.command {
        None => {
            // Default UX: interactive TUI when attached to a real terminal
            // (matches npkill's default behavior). Falls back to a headless
            // scan when piped/redirected or when --json is requested, so
            // `nodewipe > out.json` or `nodewipe --json` still work without
            // needing the explicit `scan` subcommand.
            if cli.json || !atty_stdout() {
                cmd_scan(&cli.root, cli.json, 0, false, exclude_kinds)
            } else {
                tui::run(cli.root)?;
                Ok(ExitCode::SUCCESS)
            }
        }
        Some(Command::Scan { min_mb, grouped }) => cmd_scan(&cli.root, cli.json, min_mb, grouped, exclude_kinds),
        Some(Command::Delete { paths, mode, yes, dry_run }) => {
            cmd_delete(paths, mode.into(), yes, dry_run, cli.json)
        }
        Some(Command::Types) => cmd_types(),
    }
}

fn parse_exclude_types(raw: &[String]) -> Result<Vec<ArtifactKind>> {
    raw.iter()
        .map(|s| {
            ArtifactKind::from_slug(s.trim())
                .ok_or_else(|| anyhow::anyhow!("unknown artifact type '{s}'. Valid types: {TYPE_HELP}"))
        })
        .collect()
}

fn atty_stdout() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

fn cmd_types() -> Result<ExitCode> {
    println!("Supported artifact types:\n");
    for kind in ALL_KINDS {
        println!("  {:<14} {}", kind.slug(), kind.label());
    }
    Ok(ExitCode::SUCCESS)
}

const ALL_KINDS: &[ArtifactKind] = &[
    ArtifactKind::NodeModules,
    ArtifactKind::PythonVenv,
    ArtifactKind::PythonPycache,
    ArtifactKind::PythonPytestCache,
    ArtifactKind::PythonMypyCache,
    ArtifactKind::PythonRuffCache,
    ArtifactKind::RustTarget,
    ArtifactKind::JavaMavenTarget,
    ArtifactKind::JavaGradleBuild,
    ArtifactKind::NextCache,
    ArtifactKind::TurboCache,
    ArtifactKind::GenericDist,
];

fn cmd_scan(root: &PathBuf, json: bool, min_mb: u64, grouped: bool, exclude_kinds: Vec<ArtifactKind>) -> Result<ExitCode> {
    let opts = ScanOptions {
        root: root.clone(),
        exclude_kinds,
        ..Default::default()
    };

    let mut entries: Vec<Entry> = scan(&opts)?;
    annotate_workspace_roots(&mut entries, &opts.root);

    let min_bytes = min_mb * 1024 * 1024;
    entries.retain(|e| e.size_bytes >= min_bytes);
    entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));

    if grouped {
        let groups = group_by_workspace(entries);
        if json {
            println!("{}", serde_json::to_string_pretty(&groups.into_values().collect::<Vec<_>>())?);
        } else {
            for group in groups.values() {
                println!("\n{} ({})", group.root.display(), human_size(group.total_size_bytes));
                for e in &group.entries {
                    println!("  {}  {}  {}", human_size(e.size_bytes), kind_display(e), e.path.display());
                }
            }
        }
    } else if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        let total: u64 = entries.iter().map(|e| e.size_bytes).sum();
        for e in &entries {
            println!("{}  {}  {}", human_size(e.size_bytes), kind_display(e), e.path.display());
        }
        println!("\n{} artifacts found, {} reclaimable", entries.len(), human_size(total));
    }

    Ok(ExitCode::SUCCESS)
}

fn cmd_delete(
    paths: Vec<PathBuf>,
    mode: DeleteMode,
    yes: bool,
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    if paths.is_empty() {
        eprintln!("no paths given");
        return Ok(ExitCode::FAILURE);
    }

    if dry_run {
        for p in &paths {
            println!("would delete ({mode:?}): {}", p.display());
        }
        return Ok(ExitCode::SUCCESS);
    }

    if !yes {
        eprintln!("refusing to delete without --yes (or use --dry-run to preview)");
        return Ok(ExitCode::FAILURE);
    }

    let mut results = Vec::new();
    let mut had_error = false;

    for p in &paths {
        let size = dir_size_quick(p);
        match delete(p, mode, size) {
            Ok(r) => results.push(r),
            Err(e) => {
                had_error = true;
                eprintln!("failed to delete {}: {e:#}", p.display());
            }
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        for r in &results {
            print!("deleted {} ({:?})", r.path.display(), r.mode);
            if let Some(a) = &r.archive_path {
                print!(" -> archived at {}", a.display());
            }
            println!();
        }
    }

    Ok(if had_error { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

fn dir_size_quick(path: &PathBuf) -> u64 {
    walkdir_size(path)
}

fn walkdir_size(path: &PathBuf) -> u64 {
    use std::fs;
    fn walk(p: &std::path::Path, total: &mut u64) {
        if let Ok(entries) = fs::read_dir(p) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Ok(meta) = entry.metadata() {
                    if meta.is_dir() {
                        walk(&path, total);
                    } else {
                        *total += meta.len();
                    }
                }
            }
        }
    }
    let mut total = 0;
    walk(path, &mut total);
    total
}

fn kind_display(e: &Entry) -> String {
    match &e.package_manager {
        Some(pm) => format!("{:<14} ({pm:?})", e.kind.label()),
        None => e.kind.label().to_string(),
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.2} {}", UNITS[unit])
}
