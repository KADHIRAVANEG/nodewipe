mod cli_def;
mod cmd_doctor;
mod cmd_stats;
mod tui;
mod update;

use anyhow::{Context, Result};
use clap::Parser;
pub use cli_def::BANNER;
use cli_def::{Cli, Command, DeleteModeArg, PresetArg, SortKey};
use nodewipe_core::history::{format_date, now_unix};
use nodewipe_core::{
    ArtifactKind, DeleteMode, Entry, HistoryEntry, ScanOptions, annotate_workspace_roots, delete,
    find_stale_projects, group_by_workspace, history_append, load_config, load_ignore_patterns,
    restore, scan,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

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

    let mut exclude_kinds = parse_exclude_types(&cli.exclude_types)?;
    if let Some(defaults) = &config.default_exclude_types {
        exclude_kinds.extend(parse_exclude_types(defaults)?);
    }
    exclude_kinds.sort_by_key(|k| k.slug());
    exclude_kinds.dedup_by_key(|k| k.slug());

    let ignore_patterns = if cli.no_ignore_file {
        nodewipe_core::Gitignore::empty()
    } else {
        load_ignore_patterns(&root)
    };

    if !cli.json && atty_stdout() {
        let _ = update::check_and_prompt_update();
    }

    match cli.command {
        None => {
            if cli.json || !atty_stdout() {
                cmd_scan(
                    &root,
                    cli.json,
                    0,
                    false,
                    exclude_kinds,
                    ignore_patterns,
                    None,
                    SortKey::Size,
                    None,
                    None,
                    None,
                )
            } else {
                tui::run(root)?;
                Ok(ExitCode::SUCCESS)
            }
        }
        Some(Command::Scan {
            path: sub_path,
            min_mb,
            grouped,
            older_than,
            sort,
            stale_days,
            preset,
            export,
        }) => {
            let scan_root = sub_path.unwrap_or(root);
            let scan_ignore = if cli.no_ignore_file {
                nodewipe_core::Gitignore::empty()
            } else {
                load_ignore_patterns(&scan_root)
            };
            cmd_scan(
                &scan_root,
                cli.json,
                min_mb,
                grouped,
                exclude_kinds,
                scan_ignore,
                older_than,
                sort,
                stale_days,
                preset,
                export,
            )
        }
        Some(Command::Delete {
            paths,
            mode,
            yes,
            dry_run,
        }) => {
            let resolved_mode = resolve_delete_mode(mode, &config);
            cmd_delete(paths, resolved_mode, yes, dry_run, cli.json)
        }
        Some(Command::Restore { archive }) => cmd_restore(&archive, cli.json),
        Some(Command::Types) => cmd_types(),
        Some(Command::Gui) => cmd_gui(),
        Some(Command::Doctor { path: sub_path }) => {
            let doctor_root = sub_path
                .or(cli.path)
                .or_else(dirs::home_dir)
                .unwrap_or(root);
            let doctor_ignore = if cli.no_ignore_file {
                nodewipe_core::Gitignore::empty()
            } else {
                load_ignore_patterns(&doctor_root)
            };
            cmd_doctor::run(doctor_root, cli.json, exclude_kinds, doctor_ignore)
        }
        Some(Command::Stats { days, path: _ }) => cmd_stats::run(cli.json, days),
        Some(Command::Completions { shell }) => {
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "nodewipe", &mut std::io::stdout());
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Man { install }) => cmd_man(install),
    }
}

fn resolve_delete_mode(
    cli_mode: Option<DeleteModeArg>,
    config: &nodewipe_core::Config,
) -> DeleteMode {
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

fn type_help() -> String {
    ArtifactKind::ALL
        .iter()
        .map(|k| k.slug())
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_exclude_types(raw: &[String]) -> Result<Vec<ArtifactKind>> {
    raw.iter()
        .map(|s| {
            ArtifactKind::from_slug(s.trim()).ok_or_else(|| {
                anyhow::anyhow!("unknown artifact type '{s}'. Valid types: {}", type_help())
            })
        })
        .collect()
}

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
        println!("  {:<16} {}", kind.slug(), kind.label());
        println!("  {:<16} {}", "", kind.description());
        println!();
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_man(install: bool) -> Result<ExitCode> {
    use clap::CommandFactory;
    use clap_mangen::Man;

    let cmd = Cli::command();
    let man = Man::new(cmd);
    let mut buf = Vec::new();
    man.render(&mut buf)?;

    if install {
        let man_dir = dirs::data_dir()
            .map(|d| d.join("man").join("man1"))
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".local")
                    .join("share")
                    .join("man")
                    .join("man1")
            });

        std::fs::create_dir_all(&man_dir)
            .with_context(|| format!("creating directory {}", man_dir.display()))?;

        let dest = man_dir.join("nodewipe.1");
        std::fs::write(&dest, &buf)
            .with_context(|| format!("writing man page to {}", dest.display()))?;

        println!("Installed manual page to: {}", dest.display());

        // Update man database if mandb is installed
        let _ = std::process::Command::new("mandb")
            .arg("-u")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        println!("Run 'man nodewipe' to view it anytime.");
        return Ok(ExitCode::SUCCESS);
    }

    if atty_stdout() {
        // If man utility is available, pipe to man -l -
        if let Ok(mut child) = std::process::Command::new("man")
            .arg("-l")
            .arg("-")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(&buf);
            }
            let _ = child.wait();
            return Ok(ExitCode::SUCCESS);
        }
    }

    // Default or non-interactive: output raw roff to stdout
    use std::io::Write;
    std::io::stdout().write_all(&buf)?;
    Ok(ExitCode::SUCCESS)
}

#[allow(clippy::too_many_arguments)]
fn cmd_scan(
    root: &Path,
    json: bool,
    min_mb: u64,
    grouped: bool,
    mut exclude_kinds: Vec<ArtifactKind>,
    ignore_patterns: nodewipe_core::Gitignore,
    older_than: Option<String>,
    sort: SortKey,
    stale_days: Option<u64>,
    preset: Option<PresetArg>,
    export: Option<PathBuf>,
) -> Result<ExitCode> {
    if let Some(preset) = preset {
        let allowed: &[ArtifactKind] = match preset {
            PresetArg::Node | PresetArg::Web | PresetArg::Frontend => &[
                ArtifactKind::NodeModules,
                ArtifactKind::NextCache,
                ArtifactKind::TurboCache,
                ArtifactKind::GenericDist,
                ArtifactKind::PlaywrightBrowsers,
                ArtifactKind::CypressCache,
                ArtifactKind::AstroCache,
                ArtifactKind::SvelteKitCache,
                ArtifactKind::NuxtCache,
                ArtifactKind::DocusaurusCache,
                ArtifactKind::ViteCache,
            ],
            PresetArg::Python => &[
                ArtifactKind::PythonVenv,
                ArtifactKind::PythonPycache,
                ArtifactKind::PythonPytestCache,
                ArtifactKind::PythonMypyCache,
                ArtifactKind::PythonRuffCache,
            ],
            PresetArg::Rust => &[ArtifactKind::RustTarget],
            PresetArg::Java => &[ArtifactKind::JavaMavenTarget, ArtifactKind::JavaGradleBuild],
            PresetArg::Go => &[ArtifactKind::GoModCache, ArtifactKind::GoBuildCache],
            PresetArg::Dotnet => &[ArtifactKind::DotnetBin, ArtifactKind::DotnetObj],
            PresetArg::Ai | PresetArg::Ml => &[
                ArtifactKind::HuggingFaceCache,
                ArtifactKind::OllamaModels,
                ArtifactKind::TorchCache,
                ArtifactKind::TritonCache,
            ],
            PresetArg::Mobile | PresetArg::Apple | PresetArg::Ios | PresetArg::Android => &[
                ArtifactKind::XcodeDerivedData,
                ArtifactKind::SwiftPmBuild,
                ArtifactKind::CocoaPods,
                ArtifactKind::GradleCache,
                ArtifactKind::AndroidBuildCache,
            ],
        };

        for kind in ArtifactKind::ALL {
            if !allowed.contains(kind) && !exclude_kinds.contains(kind) {
                exclude_kinds.push(*kind);
            }
        }
    }

    let opts = ScanOptions {
        root: root.to_path_buf(),
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
            Some(t) => now
                .duration_since(t)
                .map(|age| age >= threshold)
                .unwrap_or(false),
            None => false,
        });
    }

    match sort {
        SortKey::Size => {
            entries.sort_by_key(|b| std::cmp::Reverse(b.size_bytes));
        }
        SortKey::Age => {
            entries.sort_by(|a, b| match (a.last_modified, b.last_modified) {
                (Some(ta), Some(tb)) => ta.cmp(&tb),
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, None) => std::cmp::Ordering::Equal,
            });
        }
    }

    let stale = if let Some(days) = stale_days {
        let threshold = Duration::from_secs(days * 86400);
        find_stale_projects(&entries, threshold)
    } else {
        Vec::new()
    };

    if grouped {
        let groups = group_by_workspace(entries.clone());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&groups.into_values().collect::<Vec<_>>())?
            );
        } else {
            for group in groups.values() {
                println!(
                    "\n{} ({})",
                    group.root.display(),
                    human_size(group.total_size_bytes)
                );
                for e in &group.entries {
                    println!(
                        "  {}  {}  {}",
                        human_size(e.size_bytes),
                        kind_display(e),
                        e.path.display()
                    );
                }
            }
        }
    } else if json {
        let out = serde_json::json!({
            "entries": entries,
            "stale_projects": stale,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        let total: u64 = entries.iter().map(|e| e.size_bytes).sum();
        let now = SystemTime::now();
        for e in &entries {
            let age_str = match e.last_modified {
                Some(mtime) => {
                    let days = now
                        .duration_since(mtime)
                        .map(|d| d.as_secs() / 86400)
                        .unwrap_or(0);
                    format!("{:>4}d", days)
                }
                None => "   ?d".to_string(),
            };
            println!(
                "{}  {}  {}  {}",
                human_size(e.size_bytes),
                age_str,
                kind_display(e),
                e.path.display()
            );
        }
        println!(
            "\n{} artifacts found, {} reclaimable",
            entries.len(),
            human_size(total)
        );

        if !stale.is_empty() {
            println!(
                "\n ⚠  {} likely-abandoned project{} (source untouched for {}+ days):\n",
                stale.len(),
                if stale.len() == 1 { "" } else { "s" },
                stale_days.unwrap_or(90),
            );
            for s in &stale {
                println!(
                    "   {} days   {}   {}",
                    s.stale_days,
                    human_size(s.reclaimable_bytes),
                    s.root.display(),
                );
            }
            println!();
        }
    }

    if let Some(export_path) = export {
        export_scan_results(&export_path, &entries, &stale)?;
        if !json {
            eprintln!(
                "Exported {} results to {}",
                entries.len(),
                export_path.display()
            );
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn export_scan_results(
    path: &Path,
    entries: &[Entry],
    stale: &[nodewipe_core::StaleProject],
) -> Result<()> {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("csv") {
        let mut wtr = String::from("path,kind,size_bytes,package_manager,last_modified_unix\n");
        for e in entries {
            let pm = e
                .package_manager
                .map(|p| format!("{p:?}"))
                .unwrap_or_default();
            let mtime = e
                .last_modified
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs().to_string())
                .unwrap_or_default();
            wtr.push_str(&format!(
                "\"{}\",\"{}\",{},\"{}\",{}\n",
                e.path.display(),
                e.kind.slug(),
                e.size_bytes,
                pm,
                mtime
            ));
        }
        std::fs::write(path, wtr)
            .with_context(|| format!("writing CSV export to {}", path.display()))?;
    } else {
        let out = serde_json::json!({
            "entries": entries,
            "stale_projects": stale,
        });
        let content = serde_json::to_string_pretty(&out)?;
        std::fs::write(path, content)
            .with_context(|| format!("writing JSON export to {}", path.display()))?;
    }
    Ok(())
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
        let kind_slug = nodewipe_core::classify_path(p)
            .map(|k| k.slug().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        match delete(p, mode, size) {
            Ok(r) => {
                let ts = now_unix();
                let _ = history_append(HistoryEntry {
                    deleted_at: ts,
                    date: format_date(ts),
                    path: p.display().to_string(),
                    kind: kind_slug,
                    size_bytes: size,
                    mode: format!("{mode:?}").to_lowercase(),
                });
                results.push(r);
            }
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

    Ok(if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn cmd_restore(archive: &Path, json: bool) -> Result<ExitCode> {
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
        if let Some(kind) = nodewipe_core::classify_path(p)
            && let Some(note) = kind.risk_note()
        {
            eprintln!("⚠ WARNING [{}] {}\n  {}\n", kind.label(), p.display(), note);
        }
    }
}

fn cmd_gui() -> Result<ExitCode> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

    if let Some(dir) = &exe_dir {
        let gui_bin_name = if cfg!(windows) {
            "nodewipe-gui.exe"
        } else {
            "nodewipe-gui"
        };
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

    if cfg!(target_os = "macos") {
        let launched = std::process::Command::new("open")
            .arg("-a")
            .arg("nodewipe")
            .status();
        if matches!(launched, Ok(status) if status.success()) {
            return Ok(ExitCode::SUCCESS);
        }
    }

    eprintln!("nodewipe GUI isn't installed alongside this CLI.");
    eprintln!();
    eprintln!("Install both with:");
    eprintln!(
        "  curl -fsSL https://raw.githubusercontent.com/KADHIRAVANEG/nodewipe/main/install.sh | bash"
    );
    eprintln!("  (choose option 2: CLI + GUI)");
    eprintln!();
    eprintln!("Or from source:");
    eprintln!("  cargo build --release --workspace   # builds nodewipe-gui alongside this CLI");
    Ok(ExitCode::FAILURE)
}

fn dir_size_quick(path: &Path) -> u64 {
    nodewipe_core::scanner::dir_size(path)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_human_size_formatting() {
        assert_eq!(human_size(0), "0.00 B");
        assert_eq!(human_size(512), "512.00 B");
        assert_eq!(human_size(1024), "1.00 KB");
        assert_eq!(human_size(1024 * 1024), "1.00 MB");
        assert_eq!(human_size(1024 * 1024 * 1024), "1.00 GB");
        assert_eq!(human_size(1024 * 1024 * 1024 * 1024), "1.00 TB");
    }

    #[test]
    fn test_parse_duration() {
        assert_eq!(
            parse_duration("10d").unwrap(),
            Duration::from_secs(10 * 86400)
        );
        assert_eq!(
            parse_duration("2w").unwrap(),
            Duration::from_secs(14 * 86400)
        );
        assert_eq!(
            parse_duration("1m").unwrap(),
            Duration::from_secs(30 * 86400)
        );
        assert_eq!(
            parse_duration("1y").unwrap(),
            Duration::from_secs(365 * 86400)
        );
        assert!(parse_duration("invalid").is_err());
        assert!(parse_duration("10x").is_err());
    }

    #[test]
    fn test_parse_exclude_types() {
        let types = vec![
            "venv".to_string(),
            "rust_target".to_string(),
            "hf_cache".to_string(),
            "ollama_models".to_string(),
            "torch_cache".to_string(),
            "triton_cache".to_string(),
            "derived_data".to_string(),
            "swiftpm_build".to_string(),
            "cocoapods".to_string(),
            "gradle_cache".to_string(),
            "android_build_cache".to_string(),
            "playwright_browsers".to_string(),
            "cypress_cache".to_string(),
            "astro_cache".to_string(),
            "sveltekit_cache".to_string(),
            "nuxt_cache".to_string(),
            "docusaurus_cache".to_string(),
            "vite_cache".to_string(),
        ];
        let parsed = parse_exclude_types(&types).unwrap();
        assert_eq!(
            parsed,
            vec![
                ArtifactKind::PythonVenv,
                ArtifactKind::RustTarget,
                ArtifactKind::HuggingFaceCache,
                ArtifactKind::OllamaModels,
                ArtifactKind::TorchCache,
                ArtifactKind::TritonCache,
                ArtifactKind::XcodeDerivedData,
                ArtifactKind::SwiftPmBuild,
                ArtifactKind::CocoaPods,
                ArtifactKind::GradleCache,
                ArtifactKind::AndroidBuildCache,
                ArtifactKind::PlaywrightBrowsers,
                ArtifactKind::CypressCache,
                ArtifactKind::AstroCache,
                ArtifactKind::SvelteKitCache,
                ArtifactKind::NuxtCache,
                ArtifactKind::DocusaurusCache,
                ArtifactKind::ViteCache,
            ]
        );

        let invalid = vec!["not_a_valid_slug".to_string()];
        assert!(parse_exclude_types(&invalid).is_err());
    }

    #[test]
    fn test_resolve_delete_mode() {
        let empty_config = nodewipe_core::Config::default();
        assert_eq!(resolve_delete_mode(None, &empty_config), DeleteMode::Trash);
        assert_eq!(
            resolve_delete_mode(Some(DeleteModeArg::Archive), &empty_config),
            DeleteMode::Archive
        );

        let archive_config = nodewipe_core::Config {
            default_delete_mode: Some("archive".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_delete_mode(None, &archive_config),
            DeleteMode::Archive
        );
        assert_eq!(
            resolve_delete_mode(Some(DeleteModeArg::Permanent), &archive_config),
            DeleteMode::Permanent
        );
    }
}
