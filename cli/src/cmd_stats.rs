/// `nodewipe stats` — lifetime deletion dashboard.
///
/// Reads `~/.local/share/nodewipe/history.json` and prints a summary of
/// everything deleted so far, broken down by ecosystem and by day.
///
/// Example output:
///
/// ```
/// nodewipe stats
///
///  nodewipe stats — lifetime savings
///
///  Total freed:  23.7 GB   across 63 deletions
///
///  By ecosystem:
///  ─────────────────────────────────────────────
///  Rust target/       12    14.2 GB   last: 3 days ago
///  node_modules       31     6.1 GB   last: today
///  Python venv         9     2.1 GB   last: 14 days ago
///  dist                8     1.2 GB   last: 2 days ago
///
///  Last 7 days:
///  2026-08-20  ████████████████  2.1 GB (4 deletions)
///  2026-08-22  ████████          1.1 GB (2 deletions)
///  2026-08-23  ████              512 MB (1 deletion)
/// ```
use anyhow::Result;
use nodewipe_core::history::{
    daily_totals, history_path, load as history_load, stats_by_ecosystem,
};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn run(json: bool, days: u64) -> Result<ExitCode> {
    let entries = history_load()?;

    if json {
        let by_eco = stats_by_ecosystem(&entries);
        let daily = daily_totals(&entries);
        let out = serde_json::json!({
            "total_freed_bytes": entries.iter().map(|e| e.size_bytes).sum::<u64>(),
            "total_deletions": entries.len(),
            "by_ecosystem": by_eco,
            "daily": daily,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(ExitCode::SUCCESS);
    }

    println!(
        "\n {} nodewipe stats — lifetime savings\n",
        nodewipe_core::icons::STATS
    );

    if entries.is_empty() {
        if let Some(path) = history_path() {
            println!(" No deletions recorded yet.");
            println!(" History will be written to: {}", path.display());
        } else {
            println!(" No deletions recorded yet.");
        }
        println!();
        println!(" Delete something first:");
        println!("   nodewipe scan            # find artifacts");
        println!("   nodewipe delete <path> --mode trash --yes");
        println!();
        return Ok(ExitCode::SUCCESS);
    }

    let total_bytes: u64 = entries.iter().map(|e| e.size_bytes).sum();
    let total_count = entries.len();

    println!(
        " Total freed:  {}   across {} deletion{}\n",
        human_size(total_bytes),
        total_count,
        if total_count == 1 { "" } else { "s" },
    );

    // ── By ecosystem ──────────────────────────────────────────────────────
    let by_eco = stats_by_ecosystem(&entries);
    println!(" By ecosystem:");
    println!(" {}", "─".repeat(56));

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    for eco in &by_eco {
        let last_str = format_relative(now_secs, eco.last_deleted_at);
        println!(
            "  {:<22} {:>5}   {:>10}   last: {}",
            eco.kind,
            eco.count,
            human_size(eco.total_bytes),
            last_str,
        );
    }

    // ── Daily bar chart (last N days) ─────────────────────────────────────
    println!();
    println!(" Last {} days:", days);
    println!(" {}", "─".repeat(56));

    let all_daily = daily_totals(&entries);

    // Compute cutoff date string.
    let cutoff_secs = now_secs.saturating_sub(days * 86400);
    let cutoff_date = nodewipe_core::history::format_date(cutoff_secs);

    let recent: Vec<_> = all_daily
        .iter()
        .filter(|d| d.date.as_str() >= cutoff_date.as_str())
        .collect();

    if recent.is_empty() {
        println!(" (no deletions in the last {} days)", days);
    } else {
        let max_bytes = recent.iter().map(|d| d.bytes_freed).max().unwrap_or(1);
        for d in &recent {
            let bar_len = (d.bytes_freed as f64 / max_bytes as f64 * 24.0) as usize;
            let bar = "█".repeat(bar_len.max(1));
            println!(
                "  {}  {:<24}  {}  ({} deletion{})",
                d.date,
                bar,
                human_size(d.bytes_freed),
                d.count,
                if d.count == 1 { "" } else { "s" },
            );
        }
    }

    // ── All-time total line ───────────────────────────────────────────────
    println!();
    println!(" All-time total: {}", human_size(total_bytes));
    if let Some(path) = history_path() {
        println!(" History file:   {}", path.display());
    }
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

fn format_relative(now_secs: u64, then_secs: u64) -> String {
    if then_secs == 0 {
        return "unknown".to_string();
    }
    let diff = now_secs.saturating_sub(then_secs);
    let days = diff / 86400;
    match days {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        d => format!("{d} days ago"),
    }
}
