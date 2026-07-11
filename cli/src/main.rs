mod tui;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use nodewipe_core::{
    annotate_workspace_roots, delete, group_by_workspace, load_config, load_ignore_patterns, restore, scan,
    ArtifactKind, DeleteMode, Entry, ScanOptions,
};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

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

    /// Directory to scan, given directly (ncdu-style): `nodewipe /`,
    /// `nodewipe ~`, `nodewipe ../some-project`. Takes priority over --root
    /// if both are given.
    #[arg(value_name = "PATH")]
    path: Option<PathBuf>,

    /// Root directory to scan. Prefer the positional form above for
    /// everyday use; this flag mainly exists for scripts that build up
    /// arguments explicitly.
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,

    /// Emit machine-readable JSON instead of a human table.
    /// This is the headless mode requested in npkill#188 — safe for scripts/CI.
    #[arg(long, global = true)]
    json: bool,

    /// Comma-separated artifact types to skip, in addition to any set via
    /// default_exclude_types in the config file. Everything is scanned by
    /// default. Valid values: node_modules, venv, pycache, pytest_cache,
    /// mypy_cache, ruff_cache, rust_target, maven_target, gradle_build,
    /// next_cache, turbo_cache, dist.
    #[arg(long, global = true, value_delimiter = ',')]
    exclude_types: Vec<String>,

    /// Skip loading .nodewipeignore files (both the global ~/.nodewipeignore
    /// and any in the scan root).
    #[arg(long, global = true)]
    no_ignore_file: bool,
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

        /// Only show artifacts whose last modification is older than this,
        /// e.g. `30d`, `2w`, `6m`, `1y`. Surfaces likely-abandoned projects
        /// rather than active ones you'd still want to keep intact.
        #[arg(long)]
        older_than: Option<String>,
    },
    /// Delete one or more artifact directories.
    Delete {
        /// Path(s) to delete. Must be a recognized artifact directory
        /// (node_modules, venv, __pycache__, target, build, dist, ...).
        paths: Vec<PathBuf>,

        /// How to delete: move to OS trash (recoverable), archive to .tar.gz
        /// first, or permanently remove. Falls back to config file's
        /// default_delete_mode, then Trash, if not given.
        #[arg(long, value_enum)]
        mode: Option<DeleteModeArg>,

        /// Skip the confirmation prompt (required for non-interactive/CI use).
        #[arg(long)]
        yes: bool,

        /// Show what would be deleted without deleting anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Restore a `.tar.gz` backup created by `delete --mode archive` back
    /// into its original location.
    Restore {
        /// Path to the `-backup.tar.gz` file to restore.
        archive: PathBuf,
    },
    /// List every supported artifact type and its slug (for --exclude-types).
    Types,
    /// Launch the desktop GUI. Looks for a nodewipe-gui binary/AppImage
    /// installed alongside this CLI (e.g. via install.sh's CLI+GUI option,
    /// or a from-source `cargo build --release --workspace`).
    Gui,
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
    let config = load_config();
    let root = cli.path.clone().unwrap_or_else(|| cli.root.clone());

    // Merge CLI-provided exclude types with the config file's defaults —
    // both apply; the config sets a baseline, flags add to it per-invocation.
    let mut exclude_kinds = parse_exclude_types(&cli.exclude_types)?;
    if let Some(defaults) = &config.default_exclude_types {
        exclude_kinds.extend(parse_exclude_types(defaults)?);
    }
    exclude_kinds.sort_by_key(|k| k.slug());
    exclude_kinds.dedup_by_key(|k| k.slug());

    let ignore_patterns = if cli.no_ignore_file { Vec::new() } else { load_ignore_patterns(&root) };

    match cli.command {
        None => {
            // Default UX: interactive TUI when attached to a real terminal
            // (matches npkill's default behavior). Falls back to a headless
            // scan when piped/redirected or when --json is requested, so
            // `nodewipe > out.json` or `nodewipe --json` still work without
            // needing the explicit `scan` subcommand.
            if cli.json || !atty_stdout() {
                cmd_scan(&root, cli.json, 0, false, exclude_kinds, ignore_patterns, None)
            } else {
                tui::run(root)?;
                Ok(ExitCode::SUCCESS)
            }
        }
        Some(Command::Scan { min_mb, grouped, older_than }) => {
            cmd_scan(&root, cli.json, min_mb, grouped, exclude_kinds, ignore_patterns, older_than)
        }
        Some(Command::Delete { paths, mode, yes, dry_run }) => {
            let resolved_mode = resolve_delete_mode(mode, &config);
            cmd_delete(paths, resolved_mode, yes, dry_run, cli.json)
        }
        Some(Command::Restore { archive }) => cmd_restore(&archive, cli.json),
        Some(Command::Types) => cmd_types(),
        Some(Command::Gui) => cmd_gui(),
    }
}

fn resolve_delete_mode(cli_mode: Option<DeleteModeArg>, config: &nodewipe_core::Config) -> DeleteMode {
    if let Some(m) = cli_mode {
        return m.into();
    }
    match config.default_delete_mode.as_deref() {
        Some("archive") => DeleteMode::Archive,
        Some("permanent") => DeleteMode::Permanent,
        Some("trash") | None => DeleteMode::Trash,
        Some(other) => {
            eprintln!("warning: unknown default_delete_mode '{other}' in config, using trash");
            DeleteMode::Trash
        }
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

/// Parses simple age strings like "30d", "2w", "6m", "1y". A bare number is
/// treated as days. Months/years are approximate (30/365 days) — precise
/// calendar math isn't the point here, "roughly how stale is this" is.
fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    let (num_part, unit) = match s.chars().last() {
        Some(c) if c.is_ascii_alphabetic() => (&s[..s.len() - 1], c.to_ascii_lowercase()),
        _ => (s, 'd'),
    };
    let n: u64 = num_part
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid duration '{s}', expected e.g. 30d, 2w, 6m, 1y"))?;

    let days = match unit {
        'd' => n,
        'w' => n * 7,
        'm' => n * 30,
        'y' => n * 365,
        _ => anyhow::bail!("invalid duration unit in '{s}', expected d/w/m/y"),
    };
    Ok(Duration::from_secs(days * 24 * 60 * 60))
}

fn atty_stdout() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

fn cmd_types() -> Result<ExitCode> {
    println!("Supported artifact types:\n");
    for kind in ArtifactKind::ALL {
        println!("  {:<14} {}", kind.slug(), kind.label());
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_scan(
    root: &PathBuf,
    json: bool,
    min_mb: u64,
    grouped: bool,
    exclude_kinds: Vec<ArtifactKind>,
    ignore_patterns: Vec<String>,
    older_than: Option<String>,
) -> Result<ExitCode> {
    let opts = ScanOptions {
        root: root.clone(),
        exclude_kinds,
        ignore_patterns,
        ..Default::default()
    };

    let mut entries: Vec<Entry> = scan(&opts)?;
    annotate_workspace_roots(&mut entries, &opts.root);

    let min_bytes = min_mb * 1024 * 1024;
    entries.retain(|e| e.size_bytes >= min_bytes);

    if let Some(older_than) = older_than {
        let threshold = parse_duration(&older_than)?;
        let now = SystemTime::now();
        entries.retain(|e| match e.last_modified {
            Some(t) => now.duration_since(t).map(|age| age >= threshold).unwrap_or(false),
            None => false, // unknown age: don't claim it's stale
        });
    }

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

    print_risk_warnings(&paths);

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

fn cmd_restore(archive: &PathBuf, json: bool) -> Result<ExitCode> {
    match restore(archive) {
        Ok(restored_path) => {
            if json {
                println!("{}", serde_json::json!({ "restored": restored_path }));
            } else {
                println!("Restored: {}", restored_path.display());
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => {
            eprintln!("failed to restore {}: {e:#}", archive.display());
            Ok(ExitCode::FAILURE)
        }
    }
}

fn print_risk_warnings(paths: &[PathBuf]) {
    for p in paths {
        if let Some(kind) = nodewipe_core::classify_path(p) {
            if let Some(note) = kind.risk_note() {
                eprintln!("⚠ WARNING [{}] {}\n  {}\n", kind.label(), p.display(), note);
            }
        }
    }
}

fn cmd_gui() -> Result<ExitCode> {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));

    if let Some(dir) = &exe_dir {
        let gui_bin_name = if cfg!(windows) { "nodewipe-gui.exe" } else { "nodewipe-gui" };
        let candidates = [dir.join(gui_bin_name), dir.join("nodewipe-gui.AppImage")];

        for candidate in candidates {
            if candidate.exists() {
                println!("Launching nodewipe GUI...");
                std::process::Command::new(&candidate)
                    .spawn()
                    .with_context(|| format!("failed to launch {}", candidate.display()))?;
                return Ok(ExitCode::SUCCESS);
            }
        }
    }

    // macOS: fall back to the installed .app bundle, if any.
    if cfg!(target_os = "macos") {
        let launched = std::process::Command::new("open").arg("-a").arg("nodewipe").status();
        if matches!(launched, Ok(status) if status.success()) {
            return Ok(ExitCode::SUCCESS);
        }
    }

    eprintln!("nodewipe GUI isn't installed alongside this CLI.");
    eprintln!();
    eprintln!("Install both with:");
    eprintln!("  curl -fsSL https://raw.githubusercontent.com/KADHIRAVANEG/nodewipe/main/scripts/install.sh | bash");
    eprintln!("  (choose option 2: CLI + GUI)");
    eprintln!();
    eprintln!("Or from source:");
    eprintln!("  cargo build --release --workspace   # builds nodewipe-gui alongside this CLI");
    Ok(ExitCode::FAILURE)
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
