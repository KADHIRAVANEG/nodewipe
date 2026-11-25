mod cli_def;
mod cmd_doctor;
mod cmd_stats;
mod report;
mod tui;
mod update;

use anyhow::{Context, Result};
use clap::Parser;
pub use cli_def::BANNER;
use cli_def::{
    BackupsAction, Cli, Command, ConfigAction, DeleteModeArg, IgnoreCommand, PinCommand,
    PlanAction, PresetArg, PresetCommand, ProfileArg, RulesAction, ScanFormat, SortKey,
};
use nodewipe_core::history::{format_date, now_unix};
use nodewipe_core::{
    ArtifactKind, CatalogEntry, CustomRule, DeleteMode, Entry, HistoryEntry, ProfileKind,
    RiskLevel, ScanOptions, add_ignored_pattern, annotate_workspace_roots, delete,
    find_stale_projects, group_by_workspace, history_append, list_ignored_patterns, load_config,
    load_ignore_patterns, remove_ignored_pattern_from_root, restore, scan,
};
use std::collections::HashSet;
use std::io::Write;
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

pub fn map_profile_arg(arg: Option<ProfileArg>) -> Option<ProfileKind> {
    arg.map(|a| match a {
        ProfileArg::OldDependencies => ProfileKind::OldDependencies,
        ProfileArg::BuildOutputs => ProfileKind::BuildOutputs,
        ProfileArg::LargeModels => ProfileKind::LargeModels,
        ProfileArg::SafeSweep => ProfileKind::SafeSweep,
    })
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

    let mut raw_includes = cli.types.clone();
    raw_includes.extend(cli.include_types.clone());

    let mut exclude_kinds = parse_exclude_types(&cli.exclude_types)?;
    if !raw_includes.is_empty() {
        let include_kinds: HashSet<ArtifactKind> =
            parse_exclude_types(&raw_includes)?.into_iter().collect();
        for &k in ArtifactKind::ALL {
            if !include_kinds.contains(&k) && !exclude_kinds.contains(&k) {
                exclude_kinds.push(k);
            }
        }
    }
    if let Some(defaults) = &config.default_exclude_types {
        match parse_exclude_types(defaults) {
            Ok(kinds) => exclude_kinds.extend(kinds),
            Err(e) => {
                eprintln!(
                    "warning: invalid artifact type in config default_exclude_types: {e} (skipping)"
                );
            }
        }
    }

    let profile = map_profile_arg(cli.profile);
    if let Some(prof) = profile {
        for &k in ArtifactKind::ALL {
            if !prof.includes_kind(k) && !exclude_kinds.contains(&k) {
                exclude_kinds.push(k);
            }
        }
    }

    exclude_kinds.sort_by_key(|k| k.slug());
    exclude_kinds.dedup_by_key(|k| k.slug());

    let ignore_patterns = if cli.no_ignore_file {
        nodewipe_core::Gitignore::empty()
    } else {
        load_ignore_patterns(&root)
    };

    if let Some(r) = &cli.rules {
        nodewipe_core::custom_rules::set_global_rules_override(r.clone());
    }

    if !cli.json && atty_stdout() && !matches!(cli.command, Some(Command::Update { .. })) {
        let _ = update::check_and_prompt_update();
    }

    let default_sort = parse_sort_key(config.default_sort.as_deref()).unwrap_or(SortKey::Size);
    let default_min_mb = config.default_min_mb.unwrap_or(0);

    match cli.command {
        None => {
            if cli.json || !atty_stdout() {
                cmd_scan(
                    &root,
                    cli.json,
                    default_min_mb,
                    false,
                    exclude_kinds,
                    ignore_patterns,
                    None,
                    default_sort,
                    None,
                    None,
                    profile,
                    None,
                    None,
                    None,
                    !cli.serial,
                    !cli.no_git,
                )
            } else {
                let initial_excluded = if exclude_kinds.is_empty() {
                    None
                } else {
                    Some(exclude_kinds.into_iter().collect())
                };
                tui::run(
                    root,
                    initial_excluded,
                    None,
                    !cli.serial,
                    !cli.no_git,
                    cli.no_ignore_file,
                    default_min_mb,
                    default_sort,
                )?;
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
            format,
            save_snapshot,
        }) => {
            let scan_root = sub_path.unwrap_or(root);
            let scan_ignore = if cli.no_ignore_file {
                nodewipe_core::Gitignore::empty()
            } else {
                load_ignore_patterns(&scan_root)
            };
            let effective_min_mb = min_mb.or(config.default_min_mb).unwrap_or(0);
            let effective_sort = sort
                .or_else(|| parse_sort_key(config.default_sort.as_deref()))
                .unwrap_or(SortKey::Size);
            cmd_scan(
                &scan_root,
                cli.json,
                effective_min_mb,
                grouped,
                exclude_kinds,
                scan_ignore,
                older_than,
                effective_sort,
                stale_days,
                preset,
                profile,
                export,
                format,
                save_snapshot,
                !cli.serial,
                !cli.no_git,
            )
        }
        Some(Command::Report {
            path: sub_path,
            output,
            open,
            min_mb,
            older_than,
            stale_days,
            preset,
        }) => {
            let report_root = sub_path.unwrap_or(root);
            let report_ignore = if cli.no_ignore_file {
                nodewipe_core::Gitignore::empty()
            } else {
                load_ignore_patterns(&report_root)
            };
            let effective_min_mb = min_mb.or(config.default_min_mb).unwrap_or(0);
            let effective_sort =
                parse_sort_key(config.default_sort.as_deref()).unwrap_or(SortKey::Size);
            cmd_report(
                &report_root,
                output,
                open,
                effective_min_mb,
                exclude_kinds,
                report_ignore,
                older_than,
                effective_sort,
                stale_days,
                preset,
                profile,
                !cli.serial,
                !cli.no_git,
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
        Some(Command::Doctor {
            path: sub_path,
            threshold,
            notify,
            install_service,
            uninstall_service,
        }) => {
            let doctor_root = sub_path
                .or(cli.path)
                .or_else(dirs::home_dir)
                .unwrap_or(root);
            let doctor_ignore = if cli.no_ignore_file {
                nodewipe_core::Gitignore::empty()
            } else {
                load_ignore_patterns(&doctor_root)
            };
            cmd_doctor::run(
                doctor_root,
                cli.json,
                exclude_kinds,
                doctor_ignore,
                threshold,
                profile,
                notify,
                install_service,
                uninstall_service,
                !cli.serial,
                !cli.no_git,
            )
        }
        Some(Command::Stats { days, path: _ }) => cmd_stats::run(cli.json, days),
        Some(Command::Completions { shell }) => {
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "nodewipe", &mut std::io::stdout());
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Rebuild { path }) => cmd_rebuild(&path),
        Some(Command::Dedupe {
            paths,
            dry_run,
            min_size,
            allow_hardlinks,
        }) => cmd_dedupe(paths, dry_run, min_size, allow_hardlinks, cli.json),
        Some(Command::Ignore { command }) => cmd_ignore(command, &root, cli.json),
        Some(Command::Man { install }) => cmd_man(install),
        Some(Command::Config { action }) => cmd_config(action, cli.json),
        Some(Command::Update { check, yes, force }) => update::run_update(check, yes, force),
        Some(Command::Explain { path }) => cmd_explain(&path, cli.json),
        Some(Command::Pin { command, path }) => cmd_pin(command, path, cli.json),
        Some(Command::Reclaim {
            target,
            path: sub_path,
            mode,
            yes,
            dry_run,
            prefer_local_rebuild,
        }) => {
            let reclaim_root = sub_path.unwrap_or(root);
            cmd_reclaim(
                &reclaim_root,
                &target,
                cli.profile,
                mode,
                yes,
                dry_run,
                prefer_local_rebuild,
                cli.json,
                cli.serial,
                cli.no_git,
                cli.no_ignore_file,
                &config,
            )
        }
        Some(Command::Preset {
            ref command,
            ref name,
        }) => cmd_preset(command.clone(), name.clone(), &root, &cli, &config),
        Some(Command::Tree {
            path: sub_path,
            depth,
            limit,
        }) => {
            let tree_path = if sub_path == Path::new(".") {
                root
            } else {
                sub_path
            };
            cmd_tree(&tree_path, depth, limit, cli.json)
        }
        Some(Command::Timeline { path: sub_path }) => {
            let time_path = if sub_path == Path::new(".") {
                root
            } else {
                sub_path
            };
            cmd_timeline(&time_path, cli.json, cli.no_git)
        }
        Some(Command::Workspace {
            path: sub_path,
            prune_caches,
            max_age,
            dry_run,
            yes,
            global_daemons,
        }) => {
            let ws_path = if sub_path == Path::new(".") {
                root
            } else {
                sub_path
            };
            cmd_workspace(
                &ws_path,
                cli.json,
                cli.serial,
                cli.no_git,
                cli.no_ignore_file,
                prune_caches,
                max_age,
                dry_run,
                yes,
                global_daemons,
            )
        }
        Some(Command::Backups {
            action,
            path: sub_path,
        }) => {
            let backups_path = sub_path.unwrap_or_else(|| root.clone());
            cmd_backups(action, &backups_path, cli.json)
        }
        Some(Command::Diff {
            snapshot,
            path: sub_path,
        }) => {
            let diff_path = sub_path.unwrap_or_else(|| root.clone());
            cmd_diff(
                snapshot,
                &diff_path,
                cli.json,
                cli.no_ignore_file,
                !cli.serial,
                !cli.no_git,
                &config,
            )
        }
        Some(Command::Plan { action }) => cmd_plan(
            action,
            &root,
            cli.json,
            cli.no_ignore_file,
            !cli.serial,
            !cli.no_git,
            &config,
        ),
        Some(Command::Rules { action, path }) => {
            cmd_rules(action, path, cli.rules.as_deref(), &root, cli.json)
        }
    }
}

fn cmd_explain(path: &Path, json: bool) -> Result<ExitCode> {
    let explanation = match nodewipe_core::explain_path(path) {
        Ok(exp) => exp,
        Err(e) => {
            if json {
                println!("{}", serde_json::json!({ "error": e.to_string() }));
            } else {
                eprintln!("error: {e}");
            }
            return Ok(ExitCode::FAILURE);
        }
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&explanation)?);
        return Ok(ExitCode::SUCCESS);
    }

    println!();
    println!("Explanation for {}", explanation.path.display());
    println!("{}", "─".repeat(72));
    println!("  • Path:           {}", explanation.path.display());
    println!(
        "  • Size:           {} ({} bytes)",
        explanation.formatted_size, explanation.size_bytes
    );
    println!(
        "  • Artifact Type:  {} {} ({})",
        explanation.artifact_kind.icon(),
        explanation.artifact_label,
        explanation.artifact_kind.slug()
    );
    println!("  • Detection:      {}", explanation.match_reason);
    let pin_str = if explanation.is_pinned {
        format!(
            "{} [PINNED] (Protected from deletion)",
            nodewipe_core::icons::PIN
        )
    } else {
        "Unpinned (Eligible for cleanup)".to_string()
    };
    println!("  • Protection:     {pin_str}");

    let risk_indicator = match explanation.risk_level {
        nodewipe_core::RiskLevel::Low => "Low Risk",
        nodewipe_core::RiskLevel::Moderate => "Moderate Risk",
        nodewipe_core::RiskLevel::Caution => "Caution",
    };
    println!(
        "  • Risk Level:     {risk_indicator} — {}",
        explanation.risk_level.label()
    );
    println!("                    {}", explanation.risk_description);

    if let Some(staleness) = &explanation.source_staleness {
        println!();
        println!("  • Source Activity:");
        println!("    - Description:  {}", staleness.description);
        println!("    - Days Inactive: {} days", staleness.stale_days);
    }

    if let Some(git) = &explanation.git_status {
        println!();
        println!("  • Git Repository:");
        if let Some(branch) = &git.branch {
            let merged_str = if git.is_merged {
                " (Merged to upstream)"
            } else {
                ""
            };
            println!(
                "    - Branch:       {} {branch}{merged_str}",
                nodewipe_core::icons::GIT_BRANCH
            );
        }
        let dirty_str = if git.is_dirty {
            "Uncommitted changes"
        } else {
            "Clean"
        };
        println!("    - Working Tree: {dirty_str}");
    }

    if let Some(recipe) = &explanation.rebuild_plan {
        println!();
        println!("  • Recovery Recipe:");
        println!(
            "    - Command:      {} {}",
            nodewipe_core::icons::TERMINAL,
            recipe.description
        );
        println!("    - Directory:    {}", recipe.work_dir.display());
    }
    println!("{}", "─".repeat(72));
    println!();

    Ok(ExitCode::SUCCESS)
}

fn cmd_pin(
    command: Option<PinCommand>,
    direct_path: Option<PathBuf>,
    json: bool,
) -> Result<ExitCode> {
    let effective_command = match command {
        Some(cmd) => cmd,
        None => {
            if let Some(p) = direct_path {
                PinCommand::Add {
                    path: p,
                    reason: None,
                }
            } else {
                PinCommand::List
            }
        }
    };

    match effective_command {
        PinCommand::Add { path, reason } => {
            match nodewipe_core::pin::add_pinned_project(&path, reason.clone()) {
                Ok(pinned_path) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "status": "pinned",
                                "path": pinned_path.display().to_string(),
                                "reason": reason
                            })
                        );
                    } else {
                        println!(
                            "{} Pinned: {}",
                            nodewipe_core::icons::PIN,
                            pinned_path.display()
                        );
                        if let Some(r) = reason {
                            println!("   Note: {r}");
                        }
                        println!("   (Protected from deletion across CLI, TUI, and GUI)");
                    }
                    Ok(ExitCode::SUCCESS)
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    Ok(ExitCode::FAILURE)
                }
            }
        }
        PinCommand::List => {
            let list = nodewipe_core::pin::load_pinned_projects();
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if list.is_empty() {
                println!("No projects are currently pinned.");
                println!("Pin a project with: nodewipe pin <path>");
            } else {
                println!(
                    "{} Pinned Projects (Protected from Deletion):",
                    nodewipe_core::icons::PIN
                );
                println!("{}", "─".repeat(60));
                for item in list {
                    let reason_str = item
                        .reason
                        .as_deref()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default();
                    println!("  • {}{reason_str}", item.path.display());
                }
                println!("{}", "─".repeat(60));
            }
            Ok(ExitCode::SUCCESS)
        }
        PinCommand::Remove { path } => match nodewipe_core::pin::remove_pinned_project(&path) {
            Ok(true) => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "status": "unpinned",
                            "path": path.display().to_string()
                        })
                    );
                } else {
                    println!("Unpinned: {}", path.display());
                }
                Ok(ExitCode::SUCCESS)
            }
            Ok(false) => {
                eprintln!(
                    "warning: path was not in the pinned list: {}",
                    path.display()
                );
                Ok(ExitCode::FAILURE)
            }
            Err(e) => {
                eprintln!("error: {e}");
                Ok(ExitCode::FAILURE)
            }
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_reclaim(
    root: &Path,
    target_str: &str,
    profile_arg: Option<ProfileArg>,
    mode_arg: Option<DeleteModeArg>,
    yes: bool,
    dry_run: bool,
    prefer_local_rebuild: bool,
    json: bool,
    serial: bool,
    no_git: bool,
    no_ignore_file: bool,
    config: &nodewipe_core::Config,
) -> Result<ExitCode> {
    let target_bytes = match nodewipe_core::parse_size_to_bytes(target_str) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: invalid target size '{target_str}': {e}");
            return Ok(ExitCode::FAILURE);
        }
    };

    let profile = map_profile_arg(profile_arg);

    let ignore_patterns = if no_ignore_file {
        nodewipe_core::Gitignore::empty()
    } else {
        load_ignore_patterns(root)
    };

    let mut exclude_kinds = Vec::new();
    if let Some(defaults) = &config.default_exclude_types {
        match parse_exclude_types(defaults) {
            Ok(kinds) => exclude_kinds.extend(kinds),
            Err(e) => {
                eprintln!(
                    "warning: invalid artifact type in config default_exclude_types: {e} (skipping)"
                );
            }
        }
    }
    if let Some(prof) = profile {
        for kind in ArtifactKind::ALL {
            if !prof.includes_kind(*kind) && !exclude_kinds.contains(kind) {
                exclude_kinds.push(*kind);
            }
        }
    }

    if !json {
        println!(
            "\n {} nodewipe reclaim — analyzing candidates in {} …\n",
            nodewipe_core::icons::PACKAGE,
            root.display()
        );
        println!(
            " Target budget: {}",
            nodewipe_core::format_bytes(target_bytes)
        );
        if let Some(prof) = profile {
            println!(" Profile:       {} ({})", prof.label(), prof.description());
        }
        println!();
    }

    let opts = ScanOptions {
        root: root.to_path_buf(),
        exclude_kinds,
        ignore_patterns,
        compute_sizes: true,
        parallel: !serial,
        inspect_git: !no_git,
        ..Default::default()
    };

    let entries = scan(&opts)?;

    let plan = nodewipe_core::plan_reclaim(&entries, target_bytes, prefer_local_rebuild, profile);

    if json {
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(ExitCode::SUCCESS);
    }

    if plan.candidates.is_empty() {
        println!("No eligible candidate directories found to reclaim space.");
        if plan.pinned_skipped_count > 0 {
            println!(
                " Note: {} pinned project directories ({}) were protected from cleanup.",
                plan.pinned_skipped_count,
                nodewipe_core::format_bytes(plan.pinned_skipped_bytes)
            );
        }
        return Ok(ExitCode::SUCCESS);
    }

    println!(" Proposed Cleanup Plan:");
    println!(" ────────────────────────────────────────────────────────────────────────────────");
    println!(" {:<24} {:>10}   {:<10}  Path", "Ecosystem", "Size", "Risk");
    println!(" ────────────────────────────────────────────────────────────────────────────────");

    for cand in &plan.candidates {
        let label = format!("{} {}", cand.icon, cand.artifact_label);
        let risk_tag = match cand.risk_level {
            RiskLevel::Low => "Low",
            RiskLevel::Moderate => "Moderate",
            RiskLevel::Caution => "Caution",
        };
        println!(
            " {:<24} {:>10}   {:<10}  {}",
            label,
            cand.formatted_size,
            risk_tag,
            cand.path.display()
        );
        if let Some(recipe) = &cand.recovery_command {
            println!(
                "    {} Rebuild recipe: {}",
                nodewipe_core::icons::TERMINAL,
                recipe
            );
        }
    }
    println!(" ────────────────────────────────────────────────────────────────────────────────");
    println!(
        " Total Proposed: {} (Target: {})",
        plan.formatted_proposed, plan.formatted_target
    );
    if !plan.met_target {
        println!(" Note: Available unpinned candidates fell short of the requested target.");
    }
    if plan.pinned_skipped_count > 0 {
        println!(
            " {} {} pinned project directories ({}) protected from cleanup.",
            nodewipe_core::icons::PIN,
            plan.pinned_skipped_count,
            nodewipe_core::format_bytes(plan.pinned_skipped_bytes)
        );
    }
    println!();

    if dry_run {
        println!("Dry-run mode active — no directories were deleted.");
        return Ok(ExitCode::SUCCESS);
    }

    let mode = resolve_delete_mode(mode_arg, config);
    let mode_str = match mode {
        DeleteMode::Trash => "OS Trash (recoverable)",
        DeleteMode::Archive => "compressed archive (.tar.gz / .snapshot)",
        DeleteMode::Permanent => "PERMANENT deletion (unrecoverable)",
    };

    if !yes {
        print!(
            "Reclaim {} by moving proposed directories to {}? [y/N] ",
            plan.formatted_proposed, mode_str
        );
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let trimmed = input.trim().to_lowercase();
        if trimmed != "y" && trimmed != "yes" {
            println!("Operation cancelled.");
            return Ok(ExitCode::SUCCESS);
        }
    }

    println!("\nExecuting cleanup…");
    let mut total_freed: u64 = 0;
    for cand in &plan.candidates {
        match nodewipe_core::delete(&cand.path, mode, cand.size_bytes) {
            Ok(res) => {
                total_freed += res.freed_bytes;
                println!(
                    " {} {} ({})",
                    nodewipe_core::icons::CHECK,
                    cand.path.display(),
                    cand.formatted_size
                );
            }
            Err(e) => {
                eprintln!(
                    " {} Failed to delete {}: {e}",
                    nodewipe_core::icons::WARN,
                    cand.path.display()
                );
            }
        }
    }

    println!(
        "\n Successfully reclaimed {} of disk space!\n",
        nodewipe_core::format_bytes(total_freed)
    );

    Ok(ExitCode::SUCCESS)
}

fn cmd_preset(
    command: Option<PresetCommand>,
    name_arg: Option<String>,
    root: &Path,
    cli: &Cli,
    config: &nodewipe_core::Config,
) -> Result<ExitCode> {
    if command.is_none() {
        if let Some(name) = name_arg {
            return run_saved_preset(&name, cli, config);
        }
        return list_saved_presets(cli.json);
    }

    match command.unwrap() {
        PresetCommand::List => list_saved_presets(cli.json),
        PresetCommand::Run { name } => run_saved_preset(&name, cli, config),
        PresetCommand::Remove { name } => match nodewipe_core::remove_preset(&name) {
            Ok(true) => {
                if cli.json {
                    println!(
                        "{}",
                        serde_json::json!({ "status": "removed", "name": name })
                    );
                } else {
                    println!("Removed preset: '{name}'");
                }
                Ok(ExitCode::SUCCESS)
            }
            Ok(false) => {
                eprintln!("Preset '{name}' not found.");
                Ok(ExitCode::FAILURE)
            }
            Err(e) => {
                eprintln!("Failed to remove preset: {e}");
                Ok(ExitCode::FAILURE)
            }
        },
        PresetCommand::Save {
            name,
            path,
            profile,
            exclude_types,
            min_mb,
            older_than,
            sort,
            description,
        } => {
            let preset = nodewipe_core::Preset {
                name: name.clone(),
                path: path.or_else(|| Some(root.to_path_buf())),
                profile: profile.map(|p| match p {
                    ProfileArg::OldDependencies => "old-dependencies".into(),
                    ProfileArg::BuildOutputs => "build-outputs".into(),
                    ProfileArg::LargeModels => "large-models".into(),
                    ProfileArg::SafeSweep => "safe-sweep".into(),
                }),
                exclude_types: if exclude_types.is_empty() {
                    None
                } else {
                    Some(exclude_types)
                },
                min_mb,
                older_than,
                sort: sort.map(|s| match s {
                    SortKey::Size => "size".into(),
                    SortKey::Age => "age".into(),
                    SortKey::Git => "git".into(),
                    SortKey::Ecosystem => "ecosystem".into(),
                    SortKey::Path => "path".into(),
                }),
                description,
            };

            match nodewipe_core::save_preset(preset) {
                Ok(saved_path) => {
                    if cli.json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "status": "saved",
                                "name": name,
                                "config_file": saved_path.display().to_string()
                            })
                        );
                    } else {
                        println!("{} Saved preset: '{name}'", nodewipe_core::icons::CHECK);
                        println!("   Stored in: {}", saved_path.display());
                        println!("   Run with:  nodewipe preset run {name}");
                    }
                    Ok(ExitCode::SUCCESS)
                }
                Err(e) => {
                    eprintln!("Failed to save preset '{name}': {e}");
                    Ok(ExitCode::FAILURE)
                }
            }
        }
    }
}

fn list_saved_presets(json: bool) -> Result<ExitCode> {
    let presets = nodewipe_core::list_presets();
    if json {
        println!("{}", serde_json::to_string_pretty(&presets)?);
        return Ok(ExitCode::SUCCESS);
    }

    if presets.is_empty() {
        println!("No workflow presets saved yet.");
        println!("Save a preset with: nodewipe preset save <name> [--path ...] [--profile ...]");
        return Ok(ExitCode::SUCCESS);
    }

    println!("\n Saved Workflow Presets:");
    println!(" ────────────────────────────────────────────────────────────");
    for p in &presets {
        println!(" • {}", p.name);
        if let Some(desc) = &p.description {
            println!("   Description: {desc}");
        }
        if let Some(path) = &p.path {
            println!("   Path:        {}", path.display());
        }
        if let Some(prof) = &p.profile {
            println!("   Profile:     {prof}");
        }
        if let Some(min_mb) = p.min_mb {
            println!("   Min Size:    {min_mb} MB");
        }
        if let Some(older) = &p.older_than {
            println!("   Older Than:  {older}");
        }
        if let Some(sort) = &p.sort {
            println!("   Sort:        {sort}");
        }
        println!("   Run with:    nodewipe preset run {}", p.name);
        println!(" ────────────────────────────────────────────────────────────");
    }
    println!();
    Ok(ExitCode::SUCCESS)
}

fn run_saved_preset(name: &str, cli: &Cli, config: &nodewipe_core::Config) -> Result<ExitCode> {
    let preset = match nodewipe_core::get_preset(name) {
        Some(p) => p,
        None => {
            eprintln!("Error: Preset '{name}' not found.");
            eprintln!("List saved presets with: nodewipe preset list");
            return Ok(ExitCode::FAILURE);
        }
    };

    println!(
        "\n {} Running preset '{}'…",
        nodewipe_core::icons::PACKAGE,
        name
    );
    if let Some(desc) = &preset.description {
        println!("   Description: {desc}");
    }

    let scan_root = preset.path.clone().unwrap_or_else(|| PathBuf::from("."));
    let scan_ignore = if cli.no_ignore_file {
        nodewipe_core::Gitignore::empty()
    } else {
        load_ignore_patterns(&scan_root)
    };

    let mut exclude_kinds = Vec::new();
    if let Some(excludes) = &preset.exclude_types {
        exclude_kinds.extend(parse_exclude_types(excludes)?);
    }

    let profile = preset
        .profile
        .as_deref()
        .and_then(nodewipe_core::ProfileKind::from_slug);

    if let Some(prof) = profile {
        for kind in ArtifactKind::ALL {
            if !prof.includes_kind(*kind) && !exclude_kinds.contains(kind) {
                exclude_kinds.push(*kind);
            }
        }
    }

    let effective_min_mb = preset.min_mb.or(config.default_min_mb).unwrap_or(0);
    let effective_sort = parse_sort_key(preset.sort.as_deref())
        .or_else(|| parse_sort_key(config.default_sort.as_deref()))
        .unwrap_or(SortKey::Size);

    cmd_scan(
        &scan_root,
        cli.json,
        effective_min_mb,
        false,
        exclude_kinds,
        scan_ignore,
        preset.older_than,
        effective_sort,
        None,
        None,
        profile,
        None,
        None,
        None,
        !cli.serial,
        !cli.no_git,
    )
}

fn cmd_tree(path: &Path, depth: usize, limit: usize, json: bool) -> Result<ExitCode> {
    if !path.exists() {
        eprintln!("error: path does not exist: {}", path.display());
        return Ok(ExitCode::FAILURE);
    }
    let tree = match nodewipe_core::inspect_artifact_tree(path, depth, limit) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: failed to inspect directory tree: {e}");
            return Ok(ExitCode::FAILURE);
        }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&tree)?);
    } else {
        println!(
            "\n {} Storage Explorer — {}\n",
            nodewipe_core::icons::FOLDER,
            path.display()
        );
        print!("{}", tree.render_text());
        println!();
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_timeline(path: &Path, json: bool, no_git: bool) -> Result<ExitCode> {
    if !path.exists() {
        eprintln!("error: path does not exist: {}", path.display());
        return Ok(ExitCode::FAILURE);
    }

    let explanation = match nodewipe_core::explain_path(path) {
        Ok(exp) => exp,
        Err(e) => {
            if json {
                println!("{}", serde_json::json!({ "error": e.to_string() }));
            } else {
                eprintln!("error: {e}");
            }
            return Ok(ExitCode::FAILURE);
        }
    };

    let git_status = if no_git {
        None
    } else {
        nodewipe_core::inspect_git_repo(path)
    };

    let entry = Entry {
        path: explanation.path.clone(),
        size_bytes: explanation.size_bytes,
        kind: explanation.artifact_kind,
        package_manager: None,
        last_modified: std::fs::metadata(&explanation.path)
            .ok()
            .and_then(|m| m.modified().ok()),
        workspace_root: explanation.project_root.clone(),
        git_status,
        custom_rule: None,
    };

    let timeline = nodewipe_core::inspect_timeline(&entry);
    if json {
        println!("{}", serde_json::to_string_pretty(&timeline)?);
    } else {
        print!("{}", nodewipe_core::format_timeline_text(&timeline));
    }
    Ok(ExitCode::SUCCESS)
}

#[allow(clippy::too_many_arguments)]
fn cmd_workspace(
    path: &Path,
    json: bool,
    serial: bool,
    no_git: bool,
    no_ignore_file: bool,
    prune_caches: bool,
    max_age: u64,
    dry_run: bool,
    yes: bool,
    global_daemons: bool,
) -> Result<ExitCode> {
    if prune_caches {
        if !dry_run && !yes {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "error": "Refusing to prune workspace caches without --yes (or pass --dry-run to preview)" })
                );
            } else {
                eprintln!(
                    "refusing to prune workspace caches without --yes (pass --dry-run to preview)"
                );
            }
            return Ok(ExitCode::FAILURE);
        }

        let prune_report =
            nodewipe_core::prune_monorepo_caches(&nodewipe_core::CachePruneOptions {
                root: path.to_path_buf(),
                max_age_days: max_age,
                dry_run,
                include_global_caches: global_daemons,
                extra_cache_roots: Vec::new(),
            })?;

        if json {
            println!("{}", serde_json::to_string_pretty(&prune_report)?);
        } else {
            let mode_tag = if dry_run { " [DRY-RUN]" } else { "" };
            println!(
                "\n {} Workspace Cache Pruning{} — {} (threshold: {}d+)\n",
                nodewipe_core::icons::PACKAGE,
                mode_tag,
                path.display(),
                prune_report.max_age_days
            );
            for item in &prune_report.items {
                let status_label = match item.status {
                    nodewipe_core::CachePruneStatus::Pruned => "PRUNED",
                    nodewipe_core::CachePruneStatus::WouldPrune => "WOULD PRUNE",
                    nodewipe_core::CachePruneStatus::SkippedFresh => "KEEP (FRESH)",
                    nodewipe_core::CachePruneStatus::SkippedActiveLock => "SKIP (LOCKED)",
                    nodewipe_core::CachePruneStatus::SkippedPinned => "SKIP (PINNED)",
                    nodewipe_core::CachePruneStatus::Error => "ERROR",
                };
                let reason = item
                    .reason
                    .as_deref()
                    .map(|r| format!(" - {r}"))
                    .unwrap_or_default();
                println!(
                    "  [{:<12}] {:<16} {:>9} ({:>3}d)  {}{}",
                    status_label,
                    item.kind.label(),
                    item.formatted_size,
                    item.age_days,
                    item.path.display(),
                    reason
                );
            }
            println!(
                "\n Summary: {} targeted ({} reclaimable), {} fresh preserved, {} active-lock skipped\n",
                prune_report.pruned_count,
                prune_report.formatted_reclaimable,
                prune_report.skipped_fresh_count,
                prune_report.skipped_locked_count
            );
        }
        return Ok(ExitCode::SUCCESS);
    }

    let ignore_patterns = if no_ignore_file {
        nodewipe_core::Gitignore::empty()
    } else {
        load_ignore_patterns(path)
    };

    let opts = ScanOptions {
        root: path.to_path_buf(),
        parallel: !serial,
        inspect_git: !no_git,
        ignore_patterns,
        ..Default::default()
    };

    let entries = scan(&opts)?;
    let analysis = match nodewipe_core::analyze_workspace(path, &entries) {
        Some(a) => a,
        None => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "error": "No monorepo workspace marker found in target path" })
                );
            } else {
                println!(
                    "\n No monorepo workspace marker (pnpm-workspace.yaml, turbo.json, nx.json, Cargo.toml [workspace], etc.) found at {}.",
                    path.display()
                );
            }
            return Ok(ExitCode::FAILURE);
        }
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&analysis)?);
    } else {
        println!(
            "\n {} Workspace Analysis: {} ({})\n",
            nodewipe_core::icons::PACKAGE,
            analysis.root.display(),
            analysis.workspace_type
        );
        if !analysis.declared_patterns.is_empty() {
            println!(
                " Declared Manifest Globs:  {}",
                analysis.declared_patterns.join(", ")
            );
        }
        println!(
            " Total Artifact Footprint: {} across {} member packages ({} manifest targets resolved)",
            analysis.formatted_total,
            analysis.member_packages.len(),
            analysis.resolved_member_count
        );
        if analysis.shared_root_artifact_bytes > 0 {
            println!(
                " Shared Root Artifacts:    {}",
                nodewipe_core::format_bytes(analysis.shared_root_artifact_bytes)
            );
        }
        if !analysis.cache_summary.is_empty() {
            println!("\n Computation & Daemon Caches:");
            for c in &analysis.cache_summary {
                let lock_tag = if c.locked { " [LOCKED]" } else { "" };
                println!(
                    "  • {:<18} {:>9}  ({} files){}  {}",
                    c.kind.label(),
                    c.formatted_size,
                    c.entry_count,
                    lock_tag,
                    c.path.display()
                );
            }
        }
        println!("\n Member Packages (sorted by size):");
        println!(" ────────────────────────────────────────────────────────────────────────");
        for pkg in &analysis.member_packages {
            println!(
                " • {:<32} {:>10}  ({} artifacts)",
                pkg.name,
                pkg.formatted_total,
                pkg.entries.len()
            );
        }
        println!(" ────────────────────────────────────────────────────────────────────────");
        if !analysis.deduplication_advice.is_empty() {
            println!(" Recommendations:");
            for advice in &analysis.deduplication_advice {
                println!(" {} {}", nodewipe_core::icons::CHECK, advice);
            }
            println!();
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_backups(action: Option<BackupsAction>, path: &Path, json: bool) -> Result<ExitCode> {
    let action = action.unwrap_or(BackupsAction::List);
    match action {
        BackupsAction::List => {
            let backups = nodewipe_core::list_backups(path);
            if json {
                println!("{}", serde_json::to_string_pretty(&backups)?);
            } else {
                println!(
                    "\n {} Backup Archive Manager — {}\n",
                    nodewipe_core::icons::PACKAGE,
                    path.display()
                );
                if backups.is_empty() {
                    println!(" No backup archives found under {}.", path.display());
                    println!(" Create one anytime with: nodewipe delete <path> --mode archive\n");
                } else {
                    let total_bytes: u64 = backups.iter().map(|b| b.size_bytes).sum();
                    println!(
                        " Total backup storage: {} across {} archives\n",
                        nodewipe_core::format_bytes(total_bytes),
                        backups.len()
                    );
                    println!(
                        " {:<40} {:>10}  {:<8}  Type",
                        "Archive / Snapshot", "Size", "Age"
                    );
                    println!(
                        " ────────────────────────────────────────────────────────────────────────"
                    );
                    for b in &backups {
                        let kind = if b.is_snapshot {
                            "CoW Snapshot"
                        } else {
                            "tar.gz archive"
                        };
                        let age = b
                            .age_days
                            .map(|d| format!("{d}d ago"))
                            .unwrap_or_else(|| "recent".into());
                        println!(
                            " • {:<38} {:>10}  {:<8}  {}",
                            b.filename, b.formatted_size, age, kind
                        );
                    }
                    println!(
                        " ────────────────────────────────────────────────────────────────────────"
                    );
                    println!(
                        " Prune old archives: nodewipe backups prune --older-than 14d --keep 3"
                    );
                    println!(" Restore an archive: nodewipe backups restore <path>\n");
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        BackupsAction::Prune {
            older_than,
            keep,
            dry_run,
            yes,
        } => {
            let older_days = match older_than {
                Some(s) => {
                    let d = parse_duration(&s)?;
                    Some(d.as_secs() / 86400)
                }
                None => None,
            };

            if !yes && !dry_run {
                print!("Prune matching backups under {}? (y/N): ", path.display());
                std::io::stdout().flush()?;
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Aborted.");
                    return Ok(ExitCode::SUCCESS);
                }
            }

            let report = nodewipe_core::prune_backups(path, older_days, keep, dry_run)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let status_prefix = if dry_run {
                    "[DRY-RUN] Would prune"
                } else {
                    "Pruned"
                };
                println!(
                    "\n {} Backup Retention Prune Summary:\n",
                    nodewipe_core::icons::TRASH
                );
                println!(
                    " Total freed: {} ({} archives pruned, {} kept)",
                    report.formatted_freed,
                    report.pruned.len(),
                    report.kept.len()
                );
                for p in &report.pruned {
                    println!("   • {} {}", status_prefix, p.filename);
                }
                println!();
            }
            Ok(ExitCode::SUCCESS)
        }
        BackupsAction::Restore { archive } => cmd_restore(&archive, json),
    }
}

fn cmd_diff(
    snapshot_opt: Option<PathBuf>,
    path: &Path,
    json: bool,
    no_ignore_file: bool,
    parallel: bool,
    inspect_git: bool,
    config: &nodewipe_core::Config,
) -> Result<ExitCode> {
    let snapshot_file = snapshot_opt.unwrap_or_else(nodewipe_core::default_snapshot_file);
    if !snapshot_file.exists() {
        if json {
            println!(
                "{}",
                serde_json::json!({ "error": format!("Snapshot file {} does not exist", snapshot_file.display()) })
            );
        } else {
            eprintln!(
                "error: baseline snapshot file does not exist: {}",
                snapshot_file.display()
            );
            eprintln!("Run `nodewipe scan --save-snapshot` to create a baseline snapshot first.");
        }
        return Ok(ExitCode::FAILURE);
    }

    let before = nodewipe_core::load_snapshot(&snapshot_file)?;

    let ignore_patterns = if no_ignore_file {
        nodewipe_core::Gitignore::empty()
    } else {
        load_ignore_patterns(path)
    };

    let mut exclude_kinds = Vec::new();
    if let Some(defaults) = &config.default_exclude_types {
        match parse_exclude_types(defaults) {
            Ok(kinds) => exclude_kinds.extend(kinds),
            Err(e) => {
                eprintln!(
                    "warning: invalid artifact type in config default_exclude_types: {e} (skipping)"
                );
            }
        }
    }

    let opts = ScanOptions {
        root: path.to_path_buf(),
        parallel,
        inspect_git,
        exclude_kinds,
        ignore_patterns,
        ..Default::default()
    };

    let live_entries = scan(&opts)?;
    let diff = nodewipe_core::compute_scan_diff(&before, &live_entries);

    if json {
        println!("{}", serde_json::to_string_pretty(&diff)?);
    } else {
        print!("{}", diff.render_text());
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_plan(
    action: PlanAction,
    root: &Path,
    json: bool,
    no_ignore_file: bool,
    parallel: bool,
    inspect_git: bool,
    config: &nodewipe_core::Config,
) -> Result<ExitCode> {
    match action {
        PlanAction::Create {
            output,
            path: sub_path,
            profile,
            reclaim_target,
        } => {
            let scan_root = sub_path.unwrap_or_else(|| root.to_path_buf());
            let profile_kind = map_profile_arg(profile);
            let target_bytes = match reclaim_target {
                Some(t) => Some(nodewipe_core::parse_size_to_bytes(&t)?),
                None => None,
            };

            let ignore_patterns = if no_ignore_file {
                nodewipe_core::Gitignore::empty()
            } else {
                load_ignore_patterns(&scan_root)
            };

            let mut exclude_kinds = Vec::new();
            if let Some(defaults) = &config.default_exclude_types {
                exclude_kinds.extend(parse_exclude_types(defaults)?);
            }

            let opts = ScanOptions {
                root: scan_root.clone(),
                parallel,
                inspect_git,
                exclude_kinds,
                ignore_patterns,
                ..Default::default()
            };

            let entries = scan(&opts)?;
            let plan = nodewipe_core::create_cleanup_plan(
                &scan_root,
                &entries,
                profile_kind,
                target_bytes,
            );
            nodewipe_core::save_plan_to_file(&plan, &output)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                println!(
                    "\n {} Created cleanup plan: {}",
                    nodewipe_core::icons::PACKAGE,
                    output.display()
                );
                println!(
                    " Total proposed: {} across {} items",
                    plan.formatted_total,
                    plan.items.len()
                );
                println!(" Review with:  nodewipe plan view {}", output.display());
                println!(" Apply with:   nodewipe plan apply {}\n", output.display());
            }
            Ok(ExitCode::SUCCESS)
        }
        PlanAction::View { file } => {
            let plan = nodewipe_core::load_plan_from_file(&file)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                println!(
                    "\n {} Cleanup Plan: {}\n",
                    nodewipe_core::icons::PACKAGE,
                    file.display()
                );
                println!(" Root:           {}", plan.root.display());
                println!(
                    " Total proposed: {} across {} items\n",
                    plan.formatted_total,
                    plan.items.len()
                );
                println!(" {:<24} {:>10}   {:<10}  Path", "Ecosystem", "Size", "Risk");
                println!(
                    " ────────────────────────────────────────────────────────────────────────"
                );
                for item in &plan.items {
                    println!(
                        " • {:<22} {:>10}   {:<10}  {}",
                        item.kind,
                        item.formatted_size,
                        item.risk_level,
                        item.path.display()
                    );
                }
                println!(
                    " ────────────────────────────────────────────────────────────────────────"
                );
                println!(" Apply this plan: nodewipe plan apply {}\n", file.display());
            }
            Ok(ExitCode::SUCCESS)
        }
        PlanAction::Apply {
            file,
            mode,
            dry_run,
            yes,
        } => {
            let plan = nodewipe_core::load_plan_from_file(&file)?;
            let delete_mode = resolve_delete_mode(mode, config);

            if !yes && !dry_run {
                println!(
                    "\n {} About to apply cleanup plan: {}",
                    nodewipe_core::icons::WARN,
                    file.display()
                );
                println!(
                    " Total to delete: {} across {} items",
                    plan.formatted_total,
                    plan.items.len()
                );
                print!("Proceed with deletion? (y/N): ");
                std::io::stdout().flush()?;
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Aborted.");
                    return Ok(ExitCode::SUCCESS);
                }
            }

            let report = nodewipe_core::apply_cleanup_plan(&plan, delete_mode, dry_run)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let status_msg = if dry_run {
                    "[DRY-RUN] Would free"
                } else {
                    "Freed"
                };
                println!(
                    "\n {} Plan Execution Report:\n",
                    nodewipe_core::icons::CHECK
                );
                println!(
                    " {} {} across {} items",
                    status_msg, report.formatted_freed, report.applied_count
                );
                if !report.skipped.is_empty() {
                    println!(" Skipped {} items:", report.skipped.len());
                    for (path, reason) in &report.skipped {
                        println!("   • {} ({reason})", path.display());
                    }
                }
                println!();
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn cmd_rules(
    action: Option<RulesAction>,
    path: Option<PathBuf>,
    rules_flag: Option<&Path>,
    root: &Path,
    json: bool,
) -> Result<ExitCode> {
    let effective_action = match action {
        Some(a) => a,
        None => {
            if let Some(p) = path {
                if p.is_file()
                    && (p.extension().and_then(|e| e.to_str()) == Some("toml")
                        || p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.contains("nodewipe"))
                            .unwrap_or(false))
                {
                    RulesAction::Check { file: Some(p) }
                } else {
                    RulesAction::Test { path: p }
                }
            } else {
                RulesAction::List {
                    custom_only: false,
                    builtin_only: false,
                    category: None,
                }
            }
        }
    };

    match effective_action {
        RulesAction::List {
            custom_only,
            builtin_only,
            category,
        } => {
            let custom_rules = if builtin_only {
                Vec::new()
            } else if let Some(rf) = rules_flag {
                nodewipe_core::load_rules_from_file(rf)?
            } else {
                nodewipe_core::load_active_rules(root)
            };

            let cat_filter = category.as_ref().map(|c| c.to_lowercase());

            let filtered_custom: Vec<CustomRule> = custom_rules
                .into_iter()
                .filter(|r| {
                    if let Some(cf) = &cat_filter {
                        r.name.to_lowercase().contains(cf)
                            || r.ecosystem.to_lowercase().contains(cf)
                            || r.dir_name.to_lowercase().contains(cf)
                    } else {
                        true
                    }
                })
                .collect();

            let catalog_entries = nodewipe_core::catalog();
            let filtered_catalog: Vec<&CatalogEntry> = catalog_entries
                .iter()
                .filter(|entry| {
                    if custom_only {
                        return false;
                    }
                    if let Some(cf) = &cat_filter {
                        entry.name.to_lowercase().contains(cf)
                            || entry.slug.to_lowercase().contains(cf)
                            || entry.category.label().to_lowercase().contains(cf)
                            || entry.description.to_lowercase().contains(cf)
                    } else {
                        true
                    }
                })
                .collect();

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "custom_rules": filtered_custom,
                        "builtin_catalog": filtered_catalog,
                    }))?
                );
                return Ok(ExitCode::SUCCESS);
            }

            println!(
                "\n {} nodewipe rules — Extensibility & Artifact Catalog\n",
                nodewipe_core::icons::SETTINGS
            );

            if !builtin_only {
                println!(
                    "  {} Active Custom Rules (.nodewipe.toml / rules.toml)",
                    nodewipe_core::icons::TERMINAL
                );
                println!("  {}", "─".repeat(66));
                if filtered_custom.is_empty() {
                    if custom_only {
                        println!("  No custom rules matched the filter.");
                    } else {
                        println!(
                            "  (no custom rules found in project or ~/.config/nodewipe/rules.toml)"
                        );
                        println!(
                            "  Tip: run 'nodewipe rules init' to generate a starter .nodewipe.toml."
                        );
                    }
                } else {
                    for r in &filtered_custom {
                        let marker_info = match (&r.parent_marker, &r.self_marker) {
                            (Some(p), Some(s)) => format!("parent: '{p}', self: '{s}'"),
                            (Some(p), None) => format!("parent: '{p}'"),
                            (None, Some(s)) => format!("self: '{s}'"),
                            (None, None) => "none".to_string(),
                        };
                        println!(
                            "  • {:<24} target: {:<16} risk: {:<8}",
                            r.name, r.dir_name, r.risk
                        );
                        println!(
                            "    Ecosystem: {:<18} Markers: {}",
                            r.ecosystem, marker_info
                        );
                        if let Some(recipe) = &r.recipe {
                            println!("    Rebuild:   {}", recipe);
                        }
                        if let Some(desc) = &r.description {
                            println!("    Desc:      {}", desc);
                        }
                        println!();
                    }
                }
                println!();
            }

            if !custom_only {
                println!(
                    "  {} Built-in Ecosystem Catalog (39+ ecosystems)",
                    nodewipe_core::icons::PACKAGE
                );
                println!("  {}", "─".repeat(66));

                let mut current_cat = None;
                for entry in &filtered_catalog {
                    if current_cat != Some(entry.category) {
                        current_cat = Some(entry.category);
                        println!("\n  {} {}", entry.category.icon(), entry.category.label());
                    }
                    let recipe_str = entry.default_recipe.unwrap_or("none");
                    println!(
                        "    • {:<18} slug: {:<18} risk: {:<8} rebuild: {}",
                        entry.name,
                        entry.slug,
                        entry.default_risk.label(),
                        recipe_str
                    );
                    println!("      {}", entry.description);
                }
                println!("\n  {}", "─".repeat(66));
                println!(
                    "  Catalog total: {} built-in artifact types.",
                    filtered_catalog.len()
                );
                println!();
            }

            Ok(ExitCode::SUCCESS)
        }
        RulesAction::Test { path: test_target } => {
            let resolved = if test_target == Path::new(".") {
                root.to_path_buf()
            } else {
                test_target
            };

            let custom_rules = if let Some(rf) = rules_flag {
                nodewipe_core::load_rules_from_file(rf)?
            } else {
                nodewipe_core::load_active_rules(&resolved)
            };

            let matched_custom = custom_rules
                .iter()
                .find(|r| r.matches_path(&resolved))
                .cloned();
            let matched_kind = nodewipe_core::classify_path_with_custom(&resolved, &custom_rules);
            let explanation = nodewipe_core::explain_path(&resolved).ok();

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "path": resolved.display().to_string(),
                        "matched": matched_kind.is_some(),
                        "custom_rule": matched_custom,
                        "artifact_kind": matched_kind.map(|k| k.slug()),
                        "explanation": explanation,
                    }))?
                );
                return Ok(ExitCode::SUCCESS);
            }

            println!(
                "\n {} nodewipe rules test — Evaluating {}\n",
                nodewipe_core::icons::INSPECT,
                resolved.display()
            );

            if let Some(rule) = matched_custom {
                println!(
                    "  {} Matched Custom Rule: \"{}\"",
                    nodewipe_core::icons::SUCCESS,
                    rule.name
                );
                println!("    Target Directory:    {}", rule.dir_name);
                println!("    Ecosystem:           {}", rule.ecosystem);
                println!("    Risk Classification: {}", rule.risk);
                if let Some(pm) = &rule.parent_marker {
                    println!("    Parent Marker:       {} (satisfied)", pm);
                }
                if let Some(sm) = &rule.self_marker {
                    println!("    Self Marker:         {} (satisfied)", sm);
                }
                if let Some(rec) = &rule.recipe {
                    println!("    Recovery Recipe:     {}", rec);
                }
                if let Some(desc) = &rule.description {
                    println!("    Description:         {}", desc);
                }
            } else if let Some(kind) = matched_kind {
                let cat_name = nodewipe_core::get_by_kind(kind)
                    .map(|c| c.category.label())
                    .unwrap_or("General");
                println!(
                    "  {} Matched Built-in Artifact: {} ({})",
                    nodewipe_core::icons::SUCCESS,
                    kind.label(),
                    kind.slug()
                );
                if let Some(exp) = explanation {
                    println!("    Ecosystem:           {}", cat_name);
                    println!(
                        "    Risk Classification: {} ({})",
                        exp.risk_level.label(),
                        exp.risk_description
                    );
                    if let Some(plan) = &exp.rebuild_plan {
                        println!("    Recovery Recipe:     {}", plan.description);
                    }
                    println!("    Detection Rationale: {}", exp.match_reason);
                }
            } else {
                let dir_name = resolved
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                println!(
                    "  {} No matching rule or artifact signature found.",
                    nodewipe_core::icons::ERROR
                );
                println!("    Directory Name:      {}", dir_name);
                println!("    Active Custom Rules: {}", custom_rules.len());
                println!(
                    "    Tip: Add a custom rule to .nodewipe.toml using 'nodewipe rules init'."
                );
            }
            println!();

            Ok(ExitCode::SUCCESS)
        }
        RulesAction::Check { file } => {
            let check_file = match file {
                Some(f) => f,
                None => {
                    let candidates = [
                        root.join(".nodewipe.toml"),
                        root.join(".nodewipe-rules.toml"),
                        root.join("nodewipe.toml"),
                        nodewipe_core::global_rules_file(),
                    ];
                    candidates
                        .into_iter()
                        .find(|p| p.is_file())
                        .unwrap_or_else(|| root.join(".nodewipe.toml"))
                }
            };

            if !check_file.exists() {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "status": "error",
                            "file": check_file.display().to_string(),
                            "message": "file not found"
                        })
                    );
                } else {
                    eprintln!("error: rules file '{}' not found", check_file.display());
                    eprintln!("Tip: create a starter template with 'nodewipe rules init'.");
                }
                return Ok(ExitCode::FAILURE);
            }

            match nodewipe_core::load_rules_from_file(&check_file) {
                Ok(rules) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "status": "valid",
                                "file": check_file.display().to_string(),
                                "count": rules.len(),
                                "rules": rules,
                            }))?
                        );
                    } else {
                        println!(
                            "\n {} Rules validation passed: {}\n",
                            nodewipe_core::icons::SUCCESS,
                            check_file.display()
                        );
                        println!(
                            "  Discovered {} valid rule(s) with zero safety issues:\n",
                            rules.len()
                        );
                        for r in &rules {
                            let marker_info = match (&r.parent_marker, &r.self_marker) {
                                (Some(p), Some(s)) => format!("parent='{p}', self='{s}'"),
                                (Some(p), None) => format!("parent='{p}'"),
                                (None, Some(s)) => format!("self='{s}'"),
                                (None, None) => "none".to_string(),
                            };
                            println!(
                                "  • {:<20} dir: {:<16} risk: {:<8} [{}]",
                                r.name, r.dir_name, r.risk, marker_info
                            );
                        }
                        println!();
                    }
                    Ok(ExitCode::SUCCESS)
                }
                Err(e) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "status": "invalid",
                                "file": check_file.display().to_string(),
                                "error": e.to_string(),
                            })
                        );
                    } else {
                        eprintln!(
                            "\n {} Invalid rules file: {}\n",
                            nodewipe_core::icons::ERROR,
                            check_file.display()
                        );
                        eprintln!("  Error: {e}\n");
                    }
                    Ok(ExitCode::FAILURE)
                }
            }
        }
        RulesAction::Init { output, force } => {
            let dest = output.unwrap_or_else(|| root.join(".nodewipe.toml"));

            if dest.exists() && !force {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "status": "error",
                            "file": dest.display().to_string(),
                            "message": "file already exists, pass --force to overwrite"
                        })
                    );
                } else {
                    eprintln!(
                        "error: file '{}' already exists. Use --force to overwrite.",
                        dest.display()
                    );
                }
                return Ok(ExitCode::FAILURE);
            }

            let template = nodewipe_core::generate_default_rules_template();
            std::fs::write(&dest, template)?;

            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "status": "created",
                        "file": dest.display().to_string(),
                    })
                );
            } else {
                println!(
                    "\n {} Initialized custom rules configuration at {}\n",
                    nodewipe_core::icons::SUCCESS,
                    dest.display()
                );
                println!(
                    "  You can now customize artifact directory names, markers, risks, and recipes."
                );
                println!("  Validate your rules anytime: nodewipe rules check");
                println!("  Test a directory match:       nodewipe rules test <path>\n");
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn cmd_config(action: Option<ConfigAction>, json: bool) -> Result<ExitCode> {
    let action = action.unwrap_or(ConfigAction::Show);
    match action {
        ConfigAction::Path => {
            let path = nodewipe_core::config_path().unwrap_or_else(|| PathBuf::from(""));
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "path": path.display().to_string() })
                );
            } else {
                println!("{}", path.display());
            }
            Ok(ExitCode::SUCCESS)
        }
        ConfigAction::Show => {
            let cfg = nodewipe_core::load_config();
            if json {
                println!("{}", serde_json::to_string_pretty(&cfg)?);
            } else {
                let toml_str = toml::to_string_pretty(&cfg)?;
                let path = nodewipe_core::config_path();
                let path_display = path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "unknown".into());
                let exists = path.as_ref().map(|p| p.exists()).unwrap_or(false);
                println!(
                    "# Configuration (file: {}, exists: {})\n{}",
                    path_display, exists, toml_str
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        ConfigAction::Init => {
            let (path, created) =
                nodewipe_core::init_default_config().context("failed to initialize config file")?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "path": path.display().to_string(),
                        "created": created,
                    })
                );
            } else if created {
                println!("✓ Created configuration file: {}", path.display());
            } else {
                println!("Configuration file already exists: {}", path.display());
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn cmd_ignore(command: IgnoreCommand, root: &Path, json: bool) -> Result<ExitCode> {
    match command {
        IgnoreCommand::Add { path } => match add_ignored_pattern(&path) {
            Ok(file) => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "status": "ok",
                            "action": "add",
                            "path": path.display().to_string(),
                            "file": file.display().to_string()
                        })
                    );
                } else {
                    println!("✓ Added '{}' to {}", path.display(), file.display());
                }
                Ok(ExitCode::SUCCESS)
            }
            Err(e) => {
                eprintln!("error: {e}");
                Ok(ExitCode::FAILURE)
            }
        },
        IgnoreCommand::List => {
            let patterns = list_ignored_patterns(root);
            if json {
                println!("{}", serde_json::to_string_pretty(&patterns)?);
            } else if patterns.is_empty() {
                println!("No ignore patterns found in ~/.nodewipeignore or ./nodewipeignore");
            } else {
                println!("Ignored patterns:\n─────────────────────────────────────────────");
                for p in patterns {
                    let src = match p.source {
                        nodewipe_core::IgnoreSource::Global => "[global]",
                        nodewipe_core::IgnoreSource::Local(_) => "[local]",
                    };
                    println!("  {:<8} {}", src, p.pattern);
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        IgnoreCommand::Remove { pattern } => match remove_ignored_pattern_from_root(root, &pattern)
        {
            Ok(removed) => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "status": "ok",
                            "action": "remove",
                            "pattern": pattern,
                            "removed": removed
                        })
                    );
                } else if removed {
                    println!("✓ Removed pattern '{pattern}' from ignore list");
                } else {
                    println!("Pattern '{pattern}' not found in .nodewipeignore files");
                }
                Ok(ExitCode::SUCCESS)
            }
            Err(e) => {
                eprintln!("error: {e}");
                Ok(ExitCode::FAILURE)
            }
        },
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
        Some("permanent") => {
            eprintln!(
                "warning: 'permanent' delete mode cannot be set silently via config file; using 'trash' (pass --mode permanent on CLI to delete permanently)"
            );
            DeleteMode::Trash
        }
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
    let n: u64 = num_part.parse().map_err(|_| {
        anyhow::anyhow!("invalid duration '{s}', expected e.g. 24h, 30d, 2w, 6m, 1y")
    })?;

    let secs = match unit {
        'h' => n * 3600,
        'd' => n * 86400,
        'w' => n * 7 * 86400,
        'm' => n * 30 * 86400,
        'y' => n * 365 * 86400,
        _ => anyhow::bail!("invalid duration unit in '{s}', expected h/d/w/m/y"),
    };
    Ok(Duration::from_secs(secs))
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

fn cmd_rebuild(path: &Path) -> Result<ExitCode> {
    let target_entry = if let Some(kind) = nodewipe_core::classify_path(path) {
        Entry {
            path: path.to_path_buf(),
            size_bytes: 0,
            kind,
            package_manager: None,
            last_modified: None,
            workspace_root: None,
            git_status: None,
            custom_rule: None,
        }
    } else if path.join("Cargo.toml").exists() {
        Entry {
            path: path.join("target"),
            size_bytes: 0,
            kind: ArtifactKind::RustTarget,
            package_manager: None,
            last_modified: None,
            workspace_root: None,
            git_status: None,
            custom_rule: None,
        }
    } else if path.join("package.json").exists() {
        Entry {
            path: path.join("node_modules"),
            size_bytes: 0,
            kind: ArtifactKind::NodeModules,
            package_manager: None,
            last_modified: None,
            workspace_root: None,
            git_status: None,
            custom_rule: None,
        }
    } else if path.join("pyproject.toml").exists() || path.join("requirements.txt").exists() {
        Entry {
            path: path.join(".venv"),
            size_bytes: 0,
            kind: ArtifactKind::PythonVenv,
            package_manager: None,
            last_modified: None,
            workspace_root: None,
            git_status: None,
            custom_rule: None,
        }
    } else if path.join("pom.xml").exists() {
        Entry {
            path: path.join("target"),
            size_bytes: 0,
            kind: ArtifactKind::JavaMavenTarget,
            package_manager: None,
            last_modified: None,
            workspace_root: None,
            git_status: None,
            custom_rule: None,
        }
    } else {
        eprintln!(
            "error: no recognised project manifest or artifact found in {}",
            path.display()
        );
        return Ok(ExitCode::FAILURE);
    };

    let plan = match nodewipe_core::resolve_rebuild_plan(&target_entry) {
        Some(p) => p,
        None => {
            eprintln!(
                "error: no rebuild toolchain plan found for {}",
                path.display()
            );
            return Ok(ExitCode::FAILURE);
        }
    };

    println!(
        "Running rebuild: {} in {}",
        plan.description,
        plan.work_dir.display()
    );
    let status = std::process::Command::new(&plan.program)
        .args(&plan.args)
        .current_dir(&plan.work_dir)
        .status()
        .with_context(|| format!("failed to execute '{}'", plan.program))?;

    if status.success() {
        println!("Rebuild completed successfully.");
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!("Rebuild process exited with code: {:?}", status.code());
        Ok(ExitCode::FAILURE)
    }
}

fn cmd_dedupe(
    mut paths: Vec<PathBuf>,
    dry_run: bool,
    min_size: u64,
    allow_hardlinks: bool,
    json: bool,
) -> Result<ExitCode> {
    if paths.is_empty() {
        paths.push(PathBuf::from("."));
    }

    if !json {
        println!(
            "\n nodewipe dedupe — analyzing identical files across {} target(s) …\n",
            paths.len()
        );
        if dry_run {
            println!(
                " [DRY-RUN MODE] Previewing potential storage savings without modifying files.\n"
            );
        } else if allow_hardlinks {
            eprintln!(
                " Warning: --allow-hardlinks enabled. Linked files will share inodes; in-place edits to one file will affect all linked copies.\n"
            );
        }
    }

    let opts = nodewipe_core::DedupeOptions {
        dry_run,
        min_size_bytes: min_size,
        allow_hardlinks,
    };

    let stats =
        nodewipe_core::dedupe_paths(&paths, &opts).with_context(|| "deduplication scan failed")?;

    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
        return Ok(ExitCode::SUCCESS);
    }

    println!(" Scanned:                  {} files", stats.files_scanned);
    println!(" Duplicate files found:    {}", stats.duplicate_files_found);
    if !dry_run {
        println!(
            " Files deduplicated:       {}",
            stats.duplicate_files_linked
        );
    }
    println!(
        " Storage reclaimed:        {}",
        human_size(stats.reclaimed_bytes)
    );

    if stats.duplicate_files_found == 0 {
        println!("\n No duplicate files found. Your storage is already deduplicated!");
    } else if dry_run {
        println!("\n Run without `--dry-run` to execute hardlink/reflink deduplication.");
    } else if stats.duplicate_files_linked == 0 {
        println!(
            "\n 0 duplicate files linked. Current filesystem does not support CoW reflink clones;\n pass `--allow-hardlinks` to deduplicate identical files using hardlinks."
        );
    } else {
        println!("\n Deduplication complete! Linked identical files with zero data loss.");
    }
    println!();

    Ok(ExitCode::SUCCESS)
}

pub fn parse_sort_key(s: Option<&str>) -> Option<SortKey> {
    match s?.trim().to_ascii_lowercase().as_str() {
        "size" => Some(SortKey::Size),
        "age" => Some(SortKey::Age),
        "git" => Some(SortKey::Git),
        "eco" | "ecosystem" | "kind" => Some(SortKey::Ecosystem),
        "path" => Some(SortKey::Path),
        _ => None,
    }
}

pub fn sort_entries_by_key(entries: &mut [Entry], sort: SortKey) {
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
        SortKey::Git => {
            entries.sort_by(|a, b| {
                let ga = a
                    .git_status
                    .as_ref()
                    .and_then(|g| g.branch.as_deref())
                    .unwrap_or("");
                let gb = b
                    .git_status
                    .as_ref()
                    .and_then(|g| g.branch.as_deref())
                    .unwrap_or("");
                ga.cmp(gb)
            });
        }
        SortKey::Ecosystem => {
            entries.sort_by(|a, b| {
                a.kind
                    .label()
                    .cmp(b.kind.label())
                    .then_with(|| b.size_bytes.cmp(&a.size_bytes))
            });
        }
        SortKey::Path => {
            entries.sort_by(|a, b| a.path.cmp(&b.path));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_scan_pipeline(
    root: &Path,
    min_mb: u64,
    mut exclude_kinds: Vec<ArtifactKind>,
    ignore_patterns: nodewipe_core::Gitignore,
    older_than: Option<String>,
    sort: SortKey,
    stale_days: Option<u64>,
    preset: Option<PresetArg>,
    profile: Option<nodewipe_core::ProfileKind>,
    save_snapshot: Option<Option<PathBuf>>,
    parallel: bool,
    inspect_git: bool,
) -> Result<(Vec<Entry>, Vec<nodewipe_core::StaleProject>)> {
    if let Some(prof) = profile {
        for kind in ArtifactKind::ALL {
            if !prof.includes_kind(*kind) && !exclude_kinds.contains(kind) {
                exclude_kinds.push(*kind);
            }
        }
    }

    let older_than = if older_than.is_none() {
        profile
            .and_then(|p| p.default_min_age_days())
            .map(|d| format!("{d}d"))
    } else {
        older_than
    };
    if let Some(preset) = preset {
        let allowed: &[ArtifactKind] = match preset {
            PresetArg::Node | PresetArg::Web | PresetArg::Frontend => &[
                ArtifactKind::NodeModules,
                ArtifactKind::PnpmIgnored,
                ArtifactKind::NextCache,
                ArtifactKind::TurboCache,
                ArtifactKind::NxCache,
                ArtifactKind::GenericDist,
                ArtifactKind::PlaywrightBrowsers,
                ArtifactKind::CypressCache,
                ArtifactKind::AstroCache,
                ArtifactKind::SvelteKitCache,
                ArtifactKind::NuxtCache,
                ArtifactKind::DocusaurusCache,
                ArtifactKind::ViteCache,
                ArtifactKind::YarnCache,
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
            PresetArg::Systems | PresetArg::Native | PresetArg::Cpp | PresetArg::C => &[
                ArtifactKind::RustTarget,
                ArtifactKind::CmakeBuild,
                ArtifactKind::ZigCache,
                ArtifactKind::ElixirBuild,
                ArtifactKind::MesonBuild,
                ArtifactKind::BazelCache,
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
        parallel,
        inspect_git,
        ..Default::default()
    };

    let mut entries: Vec<Entry> = scan(&opts)?;
    annotate_workspace_roots(&mut entries, &opts.root);

    // Save snapshot if explicitly requested or auto-update default snapshot file
    if let Some(dest_opt) = save_snapshot {
        let dest = dest_opt.unwrap_or_else(nodewipe_core::default_snapshot_file);
        if let Err(e) = nodewipe_core::save_snapshot(root, &entries, &dest) {
            eprintln!(
                "warning: failed to save scan snapshot to {}: {e}",
                dest.display()
            );
        }
    } else {
        let default_file = nodewipe_core::default_snapshot_file();
        let _ = nodewipe_core::save_snapshot(root, &entries, &default_file);
    }

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

    sort_entries_by_key(&mut entries, sort);

    let stale = if let Some(days) = stale_days {
        let threshold = Duration::from_secs(days * 86400);
        find_stale_projects(&entries, threshold)
    } else {
        Vec::new()
    };

    Ok((entries, stale))
}

#[allow(clippy::too_many_arguments)]
fn cmd_scan(
    root: &Path,
    json: bool,
    min_mb: u64,
    grouped: bool,
    exclude_kinds: Vec<ArtifactKind>,
    ignore_patterns: nodewipe_core::Gitignore,
    older_than: Option<String>,
    sort: SortKey,
    stale_days: Option<u64>,
    preset: Option<PresetArg>,
    profile: Option<nodewipe_core::ProfileKind>,
    export: Option<PathBuf>,
    format: Option<ScanFormat>,
    save_snapshot: Option<Option<PathBuf>>,
    parallel: bool,
    inspect_git: bool,
) -> Result<ExitCode> {
    let (entries, stale) = execute_scan_pipeline(
        root,
        min_mb,
        exclude_kinds,
        ignore_patterns,
        older_than,
        sort,
        stale_days,
        preset,
        profile,
        save_snapshot,
        parallel,
        inspect_git,
    )?;

    let effective_format = format.unwrap_or(if json {
        ScanFormat::Json
    } else {
        ScanFormat::Table
    });

    match effective_format {
        ScanFormat::Html => {
            let html = report::generate_html_report(root, &entries, &stale)?;
            println!("{html}");
        }
        ScanFormat::Csv => {
            print!("{}", format_csv_results(&entries));
        }
        ScanFormat::Json | ScanFormat::Table => {
            let emit_json = effective_format == ScanFormat::Json;
            if grouped {
                let groups = group_by_workspace(entries.clone());
                if emit_json {
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
                            let git_info = match &e.git_status {
                                Some(git) => {
                                    let branch = git.branch.as_deref().unwrap_or("?");
                                    let dirty = if git.is_dirty { "*" } else { "" };
                                    let merged = if git.is_merged { " [merged]" } else { "" };
                                    format!(
                                        "({} {branch}{dirty}{merged}) ",
                                        nodewipe_core::icons::GIT_BRANCH
                                    )
                                }
                                None => String::new(),
                            };
                            println!(
                                "  {}  {}  {}{}",
                                human_size(e.size_bytes),
                                kind_display(e),
                                git_info,
                                e.path.display()
                            );
                        }
                    }
                }
            } else if emit_json {
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
                    let is_pinned = nodewipe_core::is_path_pinned(&e.path);
                    let pin_tag = if is_pinned {
                        format!(" {} [PINNED]", nodewipe_core::icons::PIN)
                    } else {
                        String::new()
                    };
                    let git_info = match &e.git_status {
                        Some(git) => {
                            let branch = git.branch.as_deref().unwrap_or("?");
                            let dirty = if git.is_dirty { "*" } else { "" };
                            let merged = if git.is_merged { " [merged]" } else { "" };
                            format!(
                                "({} {branch}{dirty}{merged})",
                                nodewipe_core::icons::GIT_BRANCH
                            )
                        }
                        None => String::new(),
                    };
                    if git_info.is_empty() {
                        println!(
                            "{}  {}  {}{pin_tag}  {}",
                            human_size(e.size_bytes),
                            age_str,
                            kind_display(e),
                            e.path.display()
                        );
                    } else {
                        println!(
                            "{}  {}  {}  {:<22}{pin_tag}  {}",
                            human_size(e.size_bytes),
                            age_str,
                            kind_display(e),
                            git_info,
                            e.path.display()
                        );
                    }
                }
                println!(
                    "\n{} artifacts found, {} reclaimable",
                    entries.len(),
                    human_size(total)
                );

                if !stale.is_empty() {
                    println!(
                        "\n {} likely-abandoned project{} (source untouched for {}+ days):\n",
                        stale.len(),
                        if stale.len() == 1 { "" } else { "s" },
                        stale_days.unwrap_or(90),
                    );
                    for s in &stale {
                        let git_info = match &s.git_branch {
                            Some(b) => {
                                let dirty = if s.is_dirty { "*" } else { "" };
                                let merged = if s.is_merged { " [merged]" } else { "" };
                                format!("({b}{dirty}{merged}) ")
                            }
                            None => String::new(),
                        };
                        println!(
                            "   {} days   {}   {}{}",
                            s.stale_days,
                            human_size(s.reclaimable_bytes),
                            git_info,
                            s.root.display(),
                        );
                    }
                    println!();
                }
            }
        }
    }

    if let Some(export_path) = export {
        export_scan_results(&export_path, root, &entries, &stale)?;
        if effective_format == ScanFormat::Table {
            eprintln!(
                "Exported {} results to {}",
                entries.len(),
                export_path.display()
            );
        }
    }

    Ok(ExitCode::SUCCESS)
}

#[allow(clippy::too_many_arguments)]
fn cmd_report(
    root: &Path,
    output: Option<PathBuf>,
    open: bool,
    min_mb: u64,
    exclude_kinds: Vec<ArtifactKind>,
    ignore_patterns: nodewipe_core::Gitignore,
    older_than: Option<String>,
    sort: SortKey,
    stale_days: Option<u64>,
    preset: Option<PresetArg>,
    profile: Option<nodewipe_core::ProfileKind>,
    parallel: bool,
    inspect_git: bool,
) -> Result<ExitCode> {
    let (entries, stale) = execute_scan_pipeline(
        root,
        min_mb,
        exclude_kinds,
        ignore_patterns,
        older_than,
        sort,
        stale_days,
        preset,
        profile,
        None,
        parallel,
        inspect_git,
    )?;

    let html = report::generate_html_report(root, &entries, &stale)?;

    let target_file = match output {
        Some(path) => Some(path),
        None if open => Some(std::env::temp_dir().join("nodewipe-audit-report.html")),
        None => None,
    };

    if let Some(dest) = target_file {
        std::fs::write(&dest, &html)
            .with_context(|| format!("writing HTML report to {}", dest.display()))?;
        eprintln!(
            "Wrote HTML storage audit report ({} entries) to {}",
            entries.len(),
            dest.display()
        );
        if open {
            tui::open_browser(&dest.to_string_lossy());
        }
    } else {
        println!("{html}");
    }

    Ok(ExitCode::SUCCESS)
}

/// Sanitizes and escapes a string field for RFC 4180 compliant CSV export,
/// preventing spreadsheet formula injection (=, +, -, @).
fn escape_csv_field(s: &str) -> String {
    let sanitized = if s.starts_with('=')
        || s.starts_with('+')
        || s.starts_with('-')
        || s.starts_with('@')
        || s.starts_with('\t')
        || s.starts_with('\r')
    {
        format!("'{s}")
    } else {
        s.to_string()
    };
    sanitized.replace('"', "\"\"")
}

fn format_csv_results(entries: &[Entry]) -> String {
    let mut wtr = String::from(
        "path,kind,size_bytes,package_manager,last_modified_unix,git_branch,git_dirty,git_merged\n",
    );
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
        let (branch, dirty, merged) = match &e.git_status {
            Some(g) => (
                g.branch.as_deref().unwrap_or(""),
                if g.is_dirty { "true" } else { "false" },
                if g.is_merged { "true" } else { "false" },
            ),
            None => ("", "", ""),
        };
        wtr.push_str(&format!(
            "\"{}\",\"{}\",{},\"{}\",{},\"{}\",{},{}\n",
            escape_csv_field(&e.path.to_string_lossy()),
            escape_csv_field(e.kind.slug()),
            e.size_bytes,
            escape_csv_field(&pm),
            mtime,
            escape_csv_field(branch),
            dirty,
            merged,
        ));
    }
    wtr
}

fn export_scan_results(
    path: &Path,
    root: &Path,
    entries: &[Entry],
    stale: &[nodewipe_core::StaleProject],
) -> Result<()> {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("csv") {
        let csv = format_csv_results(entries);
        std::fs::write(path, csv)
            .with_context(|| format!("writing CSV export to {}", path.display()))?;
    } else if ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm") {
        let html = report::generate_html_report(root, entries, stale)?;
        std::fs::write(path, html)
            .with_context(|| format!("writing HTML export to {}", path.display()))?;
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
        let mut total_freed = 0u64;
        for r in &results {
            print!("deleted {} ({:?})", r.path.display(), r.mode);
            if let Some(a) = &r.archive_path {
                print!(" -> archived at {}", a.display());
            }
            println!();
            total_freed += r.freed_bytes;
        }
        if total_freed > 0 {
            println!(
                "\n ⭐ Reclaimed {}! Enjoying nodewipe? Support us with a star: https://github.com/KADHIRAVANEG/nodewipe",
                human_size(total_freed)
            );
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
            eprintln!("WARNING [{}] {}\n  {}\n", kind.label(), p.display(), note);
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
        Some(pm) => format!("{} {:<14} ({pm:?})", e.kind.icon(), e.kind.label()),
        None => format!("{} {}", e.kind.icon(), e.kind.label()),
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
            parse_duration("24h").unwrap(),
            Duration::from_secs(24 * 3600)
        );
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
            "cmake_build".to_string(),
            "zig_cache".to_string(),
            "elixir_build".to_string(),
            "meson_build".to_string(),
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
                ArtifactKind::CmakeBuild,
                ArtifactKind::ZigCache,
                ArtifactKind::ElixirBuild,
                ArtifactKind::MesonBuild,
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

        let perm_config = nodewipe_core::Config {
            default_delete_mode: Some("permanent".into()),
            ..Default::default()
        };
        // Permanent in config falls back to trash for safety
        assert_eq!(resolve_delete_mode(None, &perm_config), DeleteMode::Trash);
        // But explicit CLI flag still permits permanent deletion
        assert_eq!(
            resolve_delete_mode(Some(DeleteModeArg::Permanent), &perm_config),
            DeleteMode::Permanent
        );
    }

    #[test]
    fn test_parse_size_threshold() {
        use crate::cmd_doctor::parse_size_threshold;

        assert_eq!(
            parse_size_threshold("20GB").unwrap(),
            20 * 1024 * 1024 * 1024
        );
        assert_eq!(
            parse_size_threshold("10G").unwrap(),
            10 * 1024 * 1024 * 1024
        );
        assert_eq!(parse_size_threshold("500MB").unwrap(), 500 * 1024 * 1024);
        assert_eq!(
            parse_size_threshold("1TB").unwrap(),
            1024 * 1024 * 1024 * 1024
        );
        assert_eq!(parse_size_threshold("1024KB").unwrap(), 1024 * 1024);
        assert_eq!(parse_size_threshold("1048576B").unwrap(), 1048576);
        assert_eq!(parse_size_threshold("5").unwrap(), 5 * 1024 * 1024 * 1024); // defaults to GB
    }

    #[test]
    fn test_include_types_resolution() {
        let types = vec!["node_modules".to_string(), "rust_target".to_string()];
        let include_kinds: HashSet<ArtifactKind> =
            parse_exclude_types(&types).unwrap().into_iter().collect();
        let mut exclude_kinds = Vec::new();
        for &k in ArtifactKind::ALL {
            if !include_kinds.contains(&k) {
                exclude_kinds.push(k);
            }
        }
        assert!(!exclude_kinds.contains(&ArtifactKind::NodeModules));
        assert!(!exclude_kinds.contains(&ArtifactKind::RustTarget));
        assert!(exclude_kinds.contains(&ArtifactKind::PythonVenv));
        assert!(exclude_kinds.contains(&ArtifactKind::GoModCache));
    }

    #[test]
    fn test_age_filter_logic() {
        use crate::tui::AgeFilter;

        let now = SystemTime::now();
        let days10 = now - Duration::from_secs(10 * 86400);
        let days45 = now - Duration::from_secs(45 * 86400);
        let days100 = now - Duration::from_secs(100 * 86400);
        let days400 = now - Duration::from_secs(400 * 86400);

        assert!(AgeFilter::All.matches(Some(days10)));
        assert!(AgeFilter::All.matches(None));

        assert!(!AgeFilter::Days30.matches(Some(days10)));
        assert!(AgeFilter::Days30.matches(Some(days45)));

        assert!(!AgeFilter::Days90.matches(Some(days45)));
        assert!(AgeFilter::Days90.matches(Some(days100)));

        assert!(!AgeFilter::Year1.matches(Some(days100)));
        assert!(AgeFilter::Year1.matches(Some(days400)));

        assert_eq!(AgeFilter::All.next(), AgeFilter::Days30);
        assert_eq!(AgeFilter::Year1.next(), AgeFilter::All);
    }

    #[test]
    fn test_config_serialization_and_defaults() {
        let cfg = nodewipe_core::Config {
            default_delete_mode: Some("archive".into()),
            default_exclude_types: Some(vec!["venv".into(), "rust_target".into()]),
            default_min_mb: Some(50),
            default_sort: Some("age".into()),
            presets: Default::default(),
        };
        let toml_str = toml::to_string(&cfg).unwrap();
        let loaded: nodewipe_core::Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(cfg, loaded);
    }

    #[test]
    fn test_parse_sort_key_and_cli_flag_defaults() {
        assert_eq!(parse_sort_key(Some("size")), Some(SortKey::Size));
        assert_eq!(parse_sort_key(Some("SIZE")), Some(SortKey::Size));
        assert_eq!(parse_sort_key(Some("age")), Some(SortKey::Age));
        assert_eq!(parse_sort_key(Some("git")), Some(SortKey::Git));
        assert_eq!(parse_sort_key(Some("eco")), Some(SortKey::Ecosystem));
        assert_eq!(parse_sort_key(Some("ecosystem")), Some(SortKey::Ecosystem));
        assert_eq!(parse_sort_key(Some("path")), Some(SortKey::Path));
        assert_eq!(parse_sort_key(Some("PATH")), Some(SortKey::Path));
        assert_eq!(parse_sort_key(Some("invalid")), None);
        assert_eq!(parse_sort_key(None), None);

        let cfg = nodewipe_core::Config {
            default_delete_mode: None,
            default_exclude_types: None,
            default_min_mb: Some(42),
            default_sort: Some("age".into()),
            presets: Default::default(),
        };
        let effective_min_mb = None.or(cfg.default_min_mb).unwrap_or(0);
        assert_eq!(effective_min_mb, 42);

        let cli_override_min_mb = Some(100).or(cfg.default_min_mb).unwrap_or(0);
        assert_eq!(cli_override_min_mb, 100);

        let effective_sort = None
            .or_else(|| parse_sort_key(cfg.default_sort.as_deref()))
            .unwrap_or(SortKey::Size);
        assert_eq!(effective_sort, SortKey::Age);

        let cli_override_sort = Some(SortKey::Git)
            .or_else(|| parse_sort_key(cfg.default_sort.as_deref()))
            .unwrap_or(SortKey::Size);
        assert_eq!(cli_override_sort, SortKey::Git);
    }

    #[test]
    fn test_unicode_width_padding() {
        use crate::tui::pad_to_width;

        // ASCII string
        let padded = pad_to_width("test", 8, true);
        assert_eq!(padded, "test    ");
        assert_eq!(padded.len(), 8);

        // Emoji (wide 2 cells)
        let emoji = pad_to_width("📦", 4, true);
        assert_eq!(emoji, "📦  "); // 2 width + 2 spaces = 4 terminal cells

        // CJK characters (2 cells each)
        let cjk = pad_to_width("日本語", 8, false);
        assert_eq!(cjk, "  日本語"); // 2 spaces + 6 width = 8 terminal cells
    }

    #[test]
    fn test_cmd_explain_cli() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("my-node-project");
        let nm = proj.join("node_modules");
        std::fs::create_dir_all(&nm).unwrap();
        std::fs::write(proj.join("package.json"), "{}").unwrap();
        std::fs::write(nm.join("index.js"), "console.log('hi');").unwrap();

        let code_json = cmd_explain(&nm, true).unwrap();
        assert_eq!(code_json, ExitCode::SUCCESS);

        let code_human = cmd_explain(&nm, false).unwrap();
        assert_eq!(code_human, ExitCode::SUCCESS);
    }

    #[test]
    fn test_cmd_pin_cli() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("my-pinned-project");
        std::fs::create_dir_all(&proj).unwrap();

        // Pin add via direct path
        let code = cmd_pin(None, Some(proj.clone()), true).unwrap();
        assert_eq!(code, ExitCode::SUCCESS);

        // Pin list
        let code_list = cmd_pin(Some(PinCommand::List), None, true).unwrap();
        assert_eq!(code_list, ExitCode::SUCCESS);

        // Pin remove
        let code_rm = cmd_pin(Some(PinCommand::Remove { path: proj.clone() }), None, true).unwrap();
        assert_eq!(code_rm, ExitCode::SUCCESS);
    }

    #[test]
    fn test_cmd_reclaim_cli_dry_run() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("my-rust-project");
        let target = proj.join("target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            proj.join("Cargo.toml"),
            "[package]\nname=\"foo\"\nversion=\"0.1.0\"",
        )
        .unwrap();
        std::fs::write(target.join("blob"), "data".repeat(1000)).unwrap();

        let cfg = nodewipe_core::Config::default();
        let code = cmd_reclaim(
            tmp.path(),
            "1KB",
            Some(ProfileArg::BuildOutputs),
            None,
            true, // yes
            true, // dry_run
            true, // prefer_local_rebuild
            true, // json
            true, // serial
            true, // no_git
            true, // no_ignore_file
            &cfg,
        )
        .unwrap();
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn test_cmd_preset_cli_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let config_file = tmp.path().join("config.toml");
        nodewipe_core::set_config_path_override(Some(config_file));

        let cli = Cli::parse_from(["nodewipe"]);
        let cfg = nodewipe_core::Config::default();

        // Save preset
        let code_save = cmd_preset(
            Some(PresetCommand::Save {
                name: "test-preset".into(),
                path: Some(tmp.path().to_path_buf()),
                profile: Some(ProfileArg::BuildOutputs),
                exclude_types: vec!["venv".into()],
                min_mb: Some(10),
                older_than: Some("30d".into()),
                sort: Some(SortKey::Size),
                description: Some("Test preset".into()),
            }),
            None,
            tmp.path(),
            &cli,
            &cfg,
        )
        .unwrap();
        assert_eq!(code_save, ExitCode::SUCCESS);

        // List presets
        let code_list =
            cmd_preset(Some(PresetCommand::List), None, tmp.path(), &cli, &cfg).unwrap();
        assert_eq!(code_list, ExitCode::SUCCESS);

        // Remove preset
        let code_rm = cmd_preset(
            Some(PresetCommand::Remove {
                name: "test-preset".into(),
            }),
            None,
            tmp.path(),
            &cli,
            &cfg,
        )
        .unwrap();
        assert_eq!(code_rm, ExitCode::SUCCESS);

        nodewipe_core::set_config_path_override(None);
    }

    #[test]
    fn test_cmd_tree_and_workspace_cli() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let cargo_toml = root.join("Cargo.toml");
        std::fs::write(&cargo_toml, "[workspace]\nmembers = [\"core\"]\n").unwrap();

        let core_dir = root.join("core");
        std::fs::create_dir_all(&core_dir).unwrap();
        std::fs::write(
            core_dir.join("Cargo.toml"),
            "[package]\nname = \"core\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let target_dir = core_dir.join("target");
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::write(target_dir.join("libcore.rlib"), vec![0u8; 1024]).unwrap();

        // Test cmd_tree
        let code_tree = cmd_tree(&target_dir, 2, 5, true).unwrap();
        assert_eq!(code_tree, ExitCode::SUCCESS);

        // Test cmd_workspace
        let code_ws =
            cmd_workspace(root, true, true, true, true, false, 14, false, false, false).unwrap();
        assert_eq!(code_ws, ExitCode::SUCCESS);
    }

    #[test]
    fn test_cmd_backups_cli() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let archive_file = root.join("node_modules.20261120_120000.tar.gz");
        std::fs::write(&archive_file, b"dummy archive content").unwrap();

        let code_list = cmd_backups(Some(BackupsAction::List), root, true).unwrap();
        assert_eq!(code_list, ExitCode::SUCCESS);

        let code_prune = cmd_backups(
            Some(BackupsAction::Prune {
                older_than: None,
                keep: Some(1),
                dry_run: true,
                yes: true,
            }),
            root,
            true,
        )
        .unwrap();
        assert_eq!(code_prune, ExitCode::SUCCESS);
    }

    #[test]
    fn test_cmd_diff_and_snapshot_cli() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let cargo_toml = root.join("Cargo.toml");
        std::fs::write(
            &cargo_toml,
            "[package]\nname = \"diff-test\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let target_dir = root.join("target");
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::write(target_dir.join("lib.rlib"), vec![0u8; 1024]).unwrap();

        let snapshot_file = tmp.path().join("scan_snap.json");
        let opts = nodewipe_core::ScanOptions {
            root: root.to_path_buf(),
            ..Default::default()
        };
        let entries = nodewipe_core::scan(&opts).unwrap();
        nodewipe_core::save_snapshot(root, &entries, &snapshot_file).unwrap();

        let cfg = nodewipe_core::Config::default();
        let code_diff = cmd_diff(Some(snapshot_file), root, true, false, true, true, &cfg).unwrap();
        assert_eq!(code_diff, ExitCode::SUCCESS);
    }

    #[test]
    fn test_cmd_plan_cli() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let cargo_toml = root.join("Cargo.toml");
        std::fs::write(
            &cargo_toml,
            "[package]\nname = \"plan-test\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let target_dir = root.join("target");
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::write(target_dir.join("lib.rlib"), vec![0u8; 1024]).unwrap();

        let plan_file = tmp.path().join("cleanup.toml");
        let cfg = nodewipe_core::Config::default();

        let code_create = cmd_plan(
            PlanAction::Create {
                output: plan_file.clone(),
                path: Some(root.to_path_buf()),
                profile: None,
                reclaim_target: None,
            },
            root,
            true,
            false,
            true,
            true,
            &cfg,
        )
        .unwrap();
        assert_eq!(code_create, ExitCode::SUCCESS);
        assert!(plan_file.exists());

        let code_view = cmd_plan(
            PlanAction::View {
                file: plan_file.clone(),
            },
            root,
            true,
            false,
            true,
            true,
            &cfg,
        )
        .unwrap();
        assert_eq!(code_view, ExitCode::SUCCESS);

        let code_apply = cmd_plan(
            PlanAction::Apply {
                file: plan_file,
                mode: None,
                dry_run: true,
                yes: true,
            },
            root,
            true,
            false,
            true,
            true,
            &cfg,
        )
        .unwrap();
        assert_eq!(code_apply, ExitCode::SUCCESS);
    }

    #[test]
    fn test_cmd_rules_cli_lifecycle() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let rules_file = root.join(".nodewipe.toml");

        // 1. Rules Init
        let code_init = cmd_rules(
            Some(RulesAction::Init {
                output: Some(rules_file.clone()),
                force: false,
            }),
            None,
            None,
            root,
            true,
        )
        .unwrap();
        assert_eq!(code_init, ExitCode::SUCCESS);
        assert!(rules_file.exists());

        // 2. Rules Check
        let code_check = cmd_rules(
            Some(RulesAction::Check {
                file: Some(rules_file.clone()),
            }),
            None,
            None,
            root,
            true,
        )
        .unwrap();
        assert_eq!(code_check, ExitCode::SUCCESS);

        // 3. Rules List
        let code_list = cmd_rules(
            Some(RulesAction::List {
                custom_only: true,
                builtin_only: false,
                category: None,
            }),
            None,
            Some(&rules_file),
            root,
            true,
        )
        .unwrap();
        assert_eq!(code_list, ExitCode::SUCCESS);

        // 4. Rules Test on matched directory
        let proj = root.join("sim_project");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("simulator.json"), "{}").unwrap();
        let cache_dir = proj.join(".sim_cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let code_test = cmd_rules(
            Some(RulesAction::Test {
                path: cache_dir.clone(),
            }),
            None,
            Some(&rules_file),
            root,
            true,
        )
        .unwrap();
        assert_eq!(code_test, ExitCode::SUCCESS);

        // 5. Catalog List filtered
        let code_cat = cmd_rules(
            Some(RulesAction::List {
                custom_only: false,
                builtin_only: true,
                category: Some("rust".into()),
            }),
            None,
            None,
            root,
            true,
        )
        .unwrap();
        assert_eq!(code_cat, ExitCode::SUCCESS);
    }

    #[test]
    fn test_escape_csv_field_formula_and_quotes() {
        assert_eq!(escape_csv_field("=cmd|calc!A1"), "'=cmd|calc!A1");
        assert_eq!(escape_csv_field("+123"), "'+123");
        assert_eq!(escape_csv_field("-cmd"), "'-cmd");
        assert_eq!(escape_csv_field("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(
            escape_csv_field("path/with \"quotes\"/node_modules"),
            "path/with \"\"quotes\"\"/node_modules"
        );
        assert_eq!(escape_csv_field("normal/path"), "normal/path");
    }
}
