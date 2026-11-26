/// `nodewipe doctor` — system-wide disk-hog analysis by ecosystem and background watcher.
///
/// Scans the given root (default: `~`), builds a `DoctorReport`, and prints
/// a human-readable summary. `--json` emits the raw report instead.
///
/// Features:
/// - `--threshold 20GB`: Sets an alert threshold.
/// - `--notify`: Sends a desktop notification if reclaimable space exceeds the threshold.
/// - `--install-service`: Installs a periodic systemd user timer (Linux) or LaunchAgent (macOS).
/// - `--uninstall-service`: Removes the background scheduler.
use anyhow::{Context, Result};
use nodewipe_core::{ArtifactKind, Gitignore, ScanOptions, doctor::build_report, scan};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[allow(clippy::too_many_arguments)]
pub fn run(
    root: PathBuf,
    json: bool,
    mut exclude_kinds: Vec<ArtifactKind>,
    ignore_patterns: Gitignore,
    threshold: Option<String>,
    profile: Option<nodewipe_core::ProfileKind>,
    notify: bool,
    install_service: bool,
    uninstall_service: bool,
    parallel: bool,
    inspect_git: bool,
) -> Result<ExitCode> {
    if install_service {
        let thresh = threshold.as_deref().unwrap_or("20GB");
        install_background_service(thresh)?;
        return Ok(ExitCode::SUCCESS);
    }

    if uninstall_service {
        uninstall_background_service()?;
        return Ok(ExitCode::SUCCESS);
    }

    let threshold_bytes = match &threshold {
        Some(t) => Some(parse_size_threshold(t)?),
        None => None,
    };

    if let Some(prof) = profile {
        for kind in ArtifactKind::ALL {
            if !prof.includes_kind(*kind) && !exclude_kinds.contains(kind) {
                exclude_kinds.push(*kind);
            }
        }
    }

    if !json && !notify {
        eprintln!(
            "\n {} nodewipe doctor — scanning {} …\n",
            nodewipe_core::icons::DOCTOR,
            root.display()
        );
        if let Some(prof) = profile {
            eprintln!(
                " Profile filter: {} ({})\n",
                prof.label(),
                prof.description()
            );
        }
        eprintln!(" This may take a moment on large trees.\n");
    }

    let opts = ScanOptions {
        root: root.clone(),
        exclude_kinds,
        ignore_patterns,
        compute_sizes: true,
        parallel,
        inspect_git,
        ..Default::default()
    };

    let entries = scan(&opts)?;
    let report = build_report(&entries);

    let threshold_exceeded = threshold_bytes
        .map(|limit| report.total_bytes >= limit)
        .unwrap_or(false);

    let mut notification_sent = false;

    if notify && (threshold_bytes.is_none() || threshold_exceeded) {
        let title = "nodewipe doctor: Reclaimable Disk Space Alert";
        let body = format!(
            "{} reclaimable found across {} dev-artifact directories.",
            human_size(report.total_bytes),
            report.total_count
        );
        notification_sent = send_desktop_notification(title, &body);
    }

    let daemon_cfg = nodewipe_core::DaemonConfig::default();
    let daemon_status = nodewipe_core::check_daemon_status(&daemon_cfg);

    if json {
        let out = serde_json::json!({
            "report": report,
            "threshold_bytes": threshold_bytes,
            "threshold_exceeded": threshold_exceeded,
            "notification_sent": notification_sent,
            "daemon": daemon_status,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(ExitCode::SUCCESS);
    }

    if notify {
        if let Some(limit) = threshold_bytes {
            if threshold_exceeded {
                println!(
                    " Notification sent: {} reclaimable (threshold: {})",
                    human_size(report.total_bytes),
                    human_size(limit)
                );
            } else {
                println!(
                    " Reclaimable space ({}) is below threshold ({}). No notification needed.",
                    human_size(report.total_bytes),
                    human_size(limit)
                );
            }
        } else {
            println!(
                " Notification sent: {} reclaimable across {} directories.",
                human_size(report.total_bytes),
                report.total_count
            );
        }
        return Ok(ExitCode::SUCCESS);
    }

    // ── human output ──────────────────────────────────────────────────────

    println!(
        " Total reclaimable: {} across {} artifact directories\n",
        human_size(report.total_bytes),
        report.total_count
    );

    if report.ecosystems.is_empty() {
        println!(" No artifact directories found. Your system is clean!");
        return Ok(ExitCode::SUCCESS);
    }

    // Table header
    println!(
        " {:<24} {:>6}   {:>10}   {:<10}  Oldest path",
        "Ecosystem", "Count", "Total", "Oldest"
    );
    println!(" {}", "─".repeat(80));

    for eco in &report.ecosystems {
        let oldest_str = match eco.oldest_days {
            Some(d) => format!("{d} days"),
            None => "unknown".to_string(),
        };
        let oldest_path = eco.oldest_path.as_deref().unwrap_or("—").to_string();
        let oldest_path = truncate_path(&oldest_path, 45);

        let icon = nodewipe_core::ArtifactKind::from_slug(&eco.kind)
            .map(|k| k.icon())
            .unwrap_or(nodewipe_core::icons::PACKAGE);
        let eco_label = format!("{icon} {}", eco.label);

        println!(
            " {:<24} {:>6}   {:>10}   {:<10}  {}",
            eco_label,
            eco.count,
            human_size(eco.total_bytes),
            oldest_str,
            oldest_path,
        );
    }

    println!("\n {}", "─".repeat(80));

    // Top space consumers
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
    if daemon_status.is_alive {
        println!(
            " Background Watcher: Active (PID {}) — watching {} workspace(s)",
            daemon_status.pid,
            daemon_status.watch_paths.len()
        );
    } else if daemon_status.is_stale_pid {
        println!(
            " Background Watcher: Stale PID (PID {} died). Run 'nodewipe daemon start' to restart.",
            daemon_status.pid
        );
    } else {
        println!(
            " Background Watcher: Inactive (run 'nodewipe daemon start' for continuous disk pressure alerts)"
        );
    }

    if cfg!(target_os = "macos") || report.ecosystems.iter().any(|e| e.kind == "derived_data") {
        println!("\n Mobile / Apple tip:");
        println!(
            "    Run `xcrun simctl delete unavailable` to purge orphaned iOS simulator runtimes.\n"
        );
    }

    println!("\n Next steps:");
    println!("    nodewipe scan --older-than 90d               # list stale artifacts");
    println!("    nodewipe scan --older-than 90d --json | jq     # script-friendly");
    println!("    nodewipe delete <path> --mode trash             # safe delete");
    println!(
        "    nodewipe daemon start --threshold 25GB          # start real-time watcher daemon"
    );
    println!("    nodewipe doctor --install-service               # setup weekly background check");
    println!();

    Ok(ExitCode::SUCCESS)
}

// ── helpers ───────────────────────────────────────────────────────────────────

pub fn parse_size_threshold(s: &str) -> Result<u64> {
    let s = s.trim();
    let (num_part, multiplier) = if let Some(n) = s
        .strip_suffix("TB")
        .or_else(|| s.strip_suffix("tb"))
        .or_else(|| s.strip_suffix('T'))
        .or_else(|| s.strip_suffix('t'))
    {
        (n.trim(), 1024u64 * 1024 * 1024 * 1024)
    } else if let Some(n) = s
        .strip_suffix("GB")
        .or_else(|| s.strip_suffix("gb"))
        .or_else(|| s.strip_suffix('G'))
        .or_else(|| s.strip_suffix('g'))
    {
        (n.trim(), 1024u64 * 1024 * 1024)
    } else if let Some(n) = s
        .strip_suffix("MB")
        .or_else(|| s.strip_suffix("mb"))
        .or_else(|| s.strip_suffix('M'))
        .or_else(|| s.strip_suffix('m'))
    {
        (n.trim(), 1024u64 * 1024)
    } else if let Some(n) = s
        .strip_suffix("KB")
        .or_else(|| s.strip_suffix("kb"))
        .or_else(|| s.strip_suffix('K'))
        .or_else(|| s.strip_suffix('k'))
    {
        (n.trim(), 1024u64)
    } else if let Some(n) = s.strip_suffix('B').or_else(|| s.strip_suffix('b')) {
        (n.trim(), 1u64)
    } else {
        (s, 1024u64 * 1024 * 1024) // Default unit is GB if not specified
    };

    let num: f64 = num_part.parse().map_err(|_| {
        anyhow::anyhow!("invalid size threshold '{s}', expected e.g. 20GB, 500MB, 10G")
    })?;
    Ok((num * multiplier as f64) as u64)
}

/// Strips control characters (such as newlines, carriage returns, null bytes)
/// from notification text to prevent statement or script injection.
pub fn sanitize_notification_text(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Safely escapes a string for inclusion in AppleScript double quotes.
#[allow(dead_code)]
pub fn escape_applescript_string(text: &str) -> String {
    sanitize_notification_text(text)
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Builds a sanitized AppleScript notification script string.
#[allow(dead_code)]
pub fn build_macos_notification_script(title: &str, body: &str) -> String {
    format!(
        "display notification \"{}\" with title \"{}\"",
        escape_applescript_string(body),
        escape_applescript_string(title)
    )
}

pub fn send_desktop_notification(title: &str, body: &str) -> bool {
    let clean_title = sanitize_notification_text(title);
    let clean_body = sanitize_notification_text(body);

    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("notify-send")
            .args([
                "-a",
                "nodewipe",
                "-u",
                "normal",
                "-i",
                "dialog-information",
                &clean_title,
                &clean_body,
            ])
            .status();
        if let Ok(s) = status
            && s.success()
        {
            return true;
        }
    }

    #[cfg(target_os = "macos")]
    {
        let script = build_macos_notification_script(&clean_title, &clean_body);
        let status = std::process::Command::new("osascript")
            .args(["-e", &script])
            .status();
        if let Ok(s) = status
            && s.success()
        {
            return true;
        }
    }

    #[cfg(target_os = "windows")]
    {
        let safe_title = clean_title.replace('\'', "''");
        let safe_body = clean_body.replace('\'', "''");
        let script = format!(
            "[void] [System.Reflection.Assembly]::LoadWithPartialName('System.Windows.Forms'); \
             $objNotify = New-Object System.Windows.Forms.NotifyIcon; \
             $objNotify.Icon = [System.Drawing.SystemIcons]::Information; \
             $objNotify.BalloonTipTitle = '{}'; \
             $objNotify.BalloonTipText = '{}'; \
             $objNotify.Visible = $True; \
             $objNotify.ShowBalloonTip(10000);",
            safe_title, safe_body
        );
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .status();
        if let Ok(s) = status
            && s.success()
        {
            return true;
        }
    }

    // Fallback: print to stderr if notification binary wasn't available
    eprintln!("[Notification] {}: {}", clean_title, clean_body);
    true
}

fn install_background_service(threshold: &str) -> Result<()> {
    if cfg!(target_os = "macos") {
        install_macos_launchagent(threshold)
    } else {
        install_linux_systemd_service(threshold)
    }
}

pub fn build_systemd_service_content(exe_path: &Path, threshold: &str) -> String {
    format!(
        "[Unit]\n\
         Description=nodewipe periodic doctor disk space check\n\
         Documentation=https://github.com/KADHIRAVANEG/nodewipe\n\n\
         [Service]\n\
         Type=oneshot\n\
         ExecStart=\"{}\" doctor --notify --threshold {}\n",
        exe_path.display(),
        threshold
    )
}

fn install_linux_systemd_service(threshold: &str) -> Result<()> {
    let home = dirs::home_dir().context("could not determine user home directory")?;
    let systemd_user_dir = home.join(".config").join("systemd").join("user");
    std::fs::create_dir_all(&systemd_user_dir)?;

    let exe_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("nodewipe"));
    let service_content = build_systemd_service_content(&exe_path, threshold);

    let timer_content = "\
[Unit]\n\
Description=Run nodewipe doctor periodic check weekly\n\n\
[Timer]\n\
OnCalendar=weekly\n\
Persistent=true\n\n\
[Install]\n\
WantedBy=timers.target\n";

    let service_path = systemd_user_dir.join("nodewipe-doctor.service");
    let timer_path = systemd_user_dir.join("nodewipe-doctor.timer");

    std::fs::write(&service_path, service_content)?;
    std::fs::write(&timer_path, timer_content)?;

    println!("Installed systemd user service: {}", service_path.display());
    println!("Installed systemd user timer:   {}", timer_path.display());

    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    let enable_status = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", "nodewipe-doctor.timer"])
        .status();

    if let Ok(s) = enable_status
        && s.success()
    {
        println!(" Enabled and started systemd user timer (nodewipe-doctor.timer).");
    } else {
        println!(
            "To activate the timer, run:\n    systemctl --user enable --now nodewipe-doctor.timer"
        );
    }

    Ok(())
}

fn install_macos_launchagent(threshold: &str) -> Result<()> {
    let home = dirs::home_dir().context("could not determine user home directory")?;
    let agents_dir = home.join("Library").join("LaunchAgents");
    std::fs::create_dir_all(&agents_dir)?;

    let exe_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("nodewipe"));
    let plist_content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
             <key>Label</key>\n\
             <string>com.nodewipe.doctor</string>\n\
             <key>ProgramArguments</key>\n\
             <array>\n\
                 <string>{}</string>\n\
                 <string>doctor</string>\n\
                 <string>--notify</string>\n\
                 <string>--threshold</string>\n\
                 <string>{}</string>\n\
             </array>\n\
             <key>StartInterval</key>\n\
             <integer>604800</integer>\n\
         </dict>\n\
         </plist>\n",
        exe_path.display(),
        threshold
    );

    let plist_path = agents_dir.join("com.nodewipe.doctor.plist");
    std::fs::write(&plist_path, plist_content)?;
    println!("Installed macOS LaunchAgent: {}", plist_path.display());

    let load_status = std::process::Command::new("launchctl")
        .args(["load", "-w", &plist_path.to_string_lossy()])
        .status();

    if let Ok(s) = load_status
        && s.success()
    {
        println!(" Loaded LaunchAgent com.nodewipe.doctor.");
    } else {
        println!(
            "To load the agent, run:\n    launchctl load -w {}",
            plist_path.display()
        );
    }

    Ok(())
}

fn uninstall_background_service() -> Result<()> {
    let home = dirs::home_dir().context("could not determine user home directory")?;

    if cfg!(target_os = "macos") {
        let plist_path = home
            .join("Library")
            .join("LaunchAgents")
            .join("com.nodewipe.doctor.plist");
        if plist_path.exists() {
            let _ = std::process::Command::new("launchctl")
                .args(["unload", "-w", &plist_path.to_string_lossy()])
                .status();
            std::fs::remove_file(&plist_path)?;
            println!(" Removed macOS LaunchAgent: {}", plist_path.display());
        } else {
            println!("No nodewipe LaunchAgent found at {}", plist_path.display());
        }
    } else {
        let systemd_user_dir = home.join(".config").join("systemd").join("user");
        let timer_path = systemd_user_dir.join("nodewipe-doctor.timer");
        let service_path = systemd_user_dir.join("nodewipe-doctor.service");

        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "--now", "nodewipe-doctor.timer"])
            .status();

        let mut removed = false;
        if timer_path.exists() {
            std::fs::remove_file(&timer_path)?;
            println!(" Removed systemd user timer: {}", timer_path.display());
            removed = true;
        }
        if service_path.exists() {
            std::fs::remove_file(&service_path)?;
            println!(" Removed systemd user service: {}", service_path.display());
            removed = true;
        }

        if removed {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();
        } else {
            println!("No nodewipe systemd service/timer files found.");
        }
    }

    Ok(())
}

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

fn truncate_path(path: &str, max: usize) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= max {
        return path.to_string();
    }
    if max <= 3 {
        return chars.into_iter().take(max).collect();
    }
    let half = (max - 3) / 2;
    let prefix: String = chars[..half].iter().collect();
    let suffix: String = chars[chars.len() - half..].iter().collect();
    format!("{prefix}...{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_path_unicode_safety() {
        // Multi-byte UTF-8 path with emojis and CJK characters
        let path = "🦀/projets_étudiants/東京/workspace/target";
        // Must not panic on character boundaries
        let truncated = truncate_path(path, 15);
        assert!(truncated.contains("..."));
        assert!(truncated.chars().count() <= 15);

        // Short strings
        assert_eq!(truncate_path("short", 10), "short");
        assert_eq!(truncate_path("abcde", 3), "abc");
    }

    #[test]
    fn test_applescript_notification_escaping() {
        let malicious_body = "hello\nset v to do shell script \"id\"\r\nsecond statement";
        let script = build_macos_notification_script("nodewipe", malicious_body);
        assert!(!script.contains('\n'));
        assert!(!script.contains('\r'));
        assert!(!script.contains('\0'));
        assert!(script.contains("display notification \"helloset v to do shell script \\\"id\\\"second statement\" with title \"nodewipe\""));
    }

    #[test]
    fn test_notification_sanitization() {
        assert_eq!(
            sanitize_notification_text("test\nwith\r\nnewlines\tand\0nulls"),
            "testwithnewlinesandnulls"
        );
        assert_eq!(
            escape_applescript_string(r#"path\with\"quotes""#),
            r#"path\\with\\\"quotes\""#
        );
    }

    #[test]
    fn test_systemd_service_content_quoting() {
        let exe = Path::new("/opt/Developer Tools/bin/nodewipe");
        let content = build_systemd_service_content(exe, "20GB");
        assert!(content.contains(
            "ExecStart=\"/opt/Developer Tools/bin/nodewipe\" doctor --notify --threshold 20GB"
        ));
    }
}
