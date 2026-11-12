/// `nodewipe doctor` — system-wide disk-hog analysis by ecosystem.
///
/// Scans the given root (default: `~`), builds a `DoctorReport`, and prints
/// a human-readable summary.  `--json` emits the raw report instead.
///
/// Example output:
///
/// ```
/// nodewipe doctor
///
///  🩺  nodewipe doctor — scanning /home/user ...
///
///  Total reclaimable: 18.4 GB across 47 artifact directories
///
///  Ecosystem         Count   Total      Oldest untouched
///  ─────────────────────────────────────────────────────────────────
///  Rust target/        8    12.1 GB    247 days  ~/old-project/target
///  node_modules       21     3.4 GB     89 days  ~/archive/foo/node_modules
///  Python venv         9     1.8 GB    142 days  ~/scripts/.venv
///  ...
///
///  Run `nodewipe scan --older-than 90d` to list candidates for deletion.
/// ```
use anyhow::Result;
use nodewipe_core::{ArtifactKind, Gitignore, ScanOptions, doctor::build_report, scan};
use std::path::PathBuf;
use std::process::ExitCode;

pub fn run(
    root: PathBuf,
    json: bool,
    exclude_kinds: Vec<ArtifactKind>,
    ignore_patterns: Gitignore,
) -> Result<ExitCode> {
    if !json {
        eprintln!("\n 🩺  nodewipe doctor — scanning {} …\n", root.display());
        eprintln!(" This may take a moment on large trees.\n");
    }

    let opts = ScanOptions {
        root: root.clone(),
        exclude_kinds,
        ignore_patterns,
        compute_sizes: true,
        ..Default::default()
    };

    let entries = scan(&opts)?;
    let report = build_report(&entries);

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(ExitCode::SUCCESS);
    }

    // ── human output ──────────────────────────────────────────────────────

    println!(
        " Total reclaimable: {} across {} artifact directories\n",
        human_size(report.total_bytes),
        report.total_count
    );

    if report.ecosystems.is_empty() {
        println!(" ✅  No artifact directories found. Your system is clean!");
        return Ok(ExitCode::SUCCESS);
    }

    // Table header
    println!(
        " {:<22} {:>6}   {:>10}   {:<10}  Oldest path",
        "Ecosystem", "Count", "Total", "Oldest"
    );
    println!(" {}", "─".repeat(80));

    for eco in &report.ecosystems {
        let oldest_str = match eco.oldest_days {
            Some(d) => format!("{d} days"),
            None => "unknown".to_string(),
        };
        let oldest_path = eco.oldest_path.as_deref().unwrap_or("—").to_string();
        // Truncate very long paths for display.
        let oldest_path = truncate_path(&oldest_path, 45);

        println!(
            " {:<22} {:>6}   {:>10}   {:<10}  {}",
            eco.label,
            eco.count,
            human_size(eco.total_bytes),
            oldest_str,
            oldest_path,
        );
    }

    println!("\n {}", "─".repeat(80));

    // Biggest single artifacts across all ecosystems — actionable quick wins.
    let mut all_biggest: Vec<_> = report
        .ecosystems
        .iter()
        .flat_map(|e| e.biggest.iter())
        .collect();
    all_biggest.sort_by_key(|b| std::cmp::Reverse(b.size_bytes));
    all_biggest.truncate(5);

    if !all_biggest.is_empty() {
        println!("\n Top space consumers:\n");
        for b in &all_biggest {
            let age = match b.age_days {
                Some(d) => format!("{d}d ago"),
                None => "unknown age".to_string(),
            };
            println!(
                "   {}  ({})  {}",
                human_size(b.size_bytes),
                age,
                truncate_path(&b.path, 60),
            );
        }
    }

    println!();
    if cfg!(target_os = "macos") || report.ecosystems.iter().any(|e| e.kind == "derived_data") {
        println!(" 📱 Mobile / Apple tip:");
        println!(
            "    Run `xcrun simctl delete unavailable` to purge orphaned iOS simulator runtimes.\n"
        );
    }

    println!(" 💡 Next steps:");
    println!("    nodewipe scan --older-than 90d    # list stale artifacts");
    println!("    nodewipe scan --older-than 90d --json | jq  # script-friendly");
    println!("    nodewipe delete <path> --mode trash          # safe delete");
    println!();

    Ok(ExitCode::SUCCESS)
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

/// Shorten a path to at most `max` chars, eliding the middle.
fn truncate_path(path: &str, max: usize) -> String {
    if path.len() <= max {
        return path.to_string();
    }
    let half = (max - 3) / 2;
    format!("{}...{}", &path[..half], &path[path.len() - half..])
}
