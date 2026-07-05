use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use npkill_core::{annotate_workspace_roots, delete, group_by_workspace, scan, DeleteMode, Entry, ScanOptions};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "npkill-rs", version, about = "Find and reclaim disk space from stray node_modules directories")]
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
}

#[derive(Subcommand)]
enum Command {
    /// Scan for node_modules directories (default if no subcommand given).
    Scan {
        /// Only show entries at least this many megabytes in size.
        #[arg(long, default_value_t = 0)]
        min_mb: u64,

        /// Group output by monorepo/workspace root instead of a flat list.
        #[arg(long)]
        grouped: bool,
    },
    /// Delete one or more node_modules directories.
    Delete {
        /// Path(s) to delete. Must be `node_modules` directories.
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
    match cli.command.unwrap_or(Command::Scan { min_mb: 0, grouped: false }) {
        Command::Scan { min_mb, grouped } => cmd_scan(&cli.root, cli.json, min_mb, grouped),
        Command::Delete { paths, mode, yes, dry_run } => {
            cmd_delete(paths, mode.into(), yes, dry_run, cli.json)
        }
    }
}

fn cmd_scan(root: &PathBuf, json: bool, min_mb: u64, grouped: bool) -> Result<ExitCode> {
    let opts = ScanOptions {
        root: root.clone(),
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
                    println!("  {}  {}", human_size(e.size_bytes), e.path.display());
                }
            }
        }
    } else if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        let total: u64 = entries.iter().map(|e| e.size_bytes).sum();
        for e in &entries {
            println!("{}  {:?}  {}", human_size(e.size_bytes), e.package_manager, e.path.display());
        }
        println!("\n{} node_modules found, {} reclaimable", entries.len(), human_size(total));
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
