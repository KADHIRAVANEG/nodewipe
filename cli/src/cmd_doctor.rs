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
use std::path::PathBuf;
use std::process::ExitCode;

#[allow(clippy::too_many_arguments)]
pub fn run(
    root: PathBuf,
    json: bool,
    exclude_kinds: Vec<ArtifactKind>,
    ignore_patterns: Gitignore,
    threshold: Option<String>,
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

    if !json && !notify {
        eprintln!("\n 🩺  nodewipe doctor — scanning {} …\n", root.display());
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

    if json {
        let out = serde_json::json!({
            "report": report,
            "threshold_bytes": threshold_bytes,
            "threshold_exceeded": threshold_exceeded,
            "notification_sent": notification_sent,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(ExitCode::SUCCESS);
    }

    if notify {
        if let Some(limit) = threshold_bytes {
            if threshold_exceeded {
                println!(
                    " 🔔  Desktop notification sent: {} reclaimable (threshold: {})",
                    human_size(report.total_bytes),
                    human_size(limit)
                );
            } else {
                println!(
                    " ✅  Reclaimable space ({}) is below threshold ({}). No notification needed.",
                    human_size(report.total_bytes),
                    human_size(limit)
                );
            }
        } else {
            println!(
                " 🔔  Desktop notification sent: {} reclaimable across {} directories.",
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
    println!("    nodewipe doctor --install-service            # setup weekly background check");
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

pub fn send_desktop_notification(title: &str, body: &str) -> bool {
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
                title,
                body,
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
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            body.replace('"', "\\\""),
            title.replace('"', "\\\"")
        );
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
        let script = format!(
            "[void] [System.Reflection.Assembly]::LoadWithPartialName('System.Windows.Forms'); \
             $objNotify = New-Object System.Windows.Forms.NotifyIcon; \
             $objNotify.Icon = [System.Drawing.SystemIcons]::Information; \
             $objNotify.BalloonTipTitle = '{}'; \
             $objNotify.BalloonTipText = '{}'; \
             $objNotify.Visible = $True; \
             $objNotify.ShowBalloonTip(10000);",
            title.replace('\'', "''"),
            body.replace('\'', "''")
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
    eprintln!("[Notification] {}: {}", title, body);
    true
}

fn install_background_service(threshold: &str) -> Result<()> {
    if cfg!(target_os = "macos") {
        install_macos_launchagent(threshold)
    } else {
        install_linux_systemd_service(threshold)
    }
}

fn install_linux_systemd_service(threshold: &str) -> Result<()> {
    let home = dirs::home_dir().context("could not determine user home directory")?;
    let systemd_user_dir = home.join(".config").join("systemd").join("user");
    std::fs::create_dir_all(&systemd_user_dir)?;

    let exe_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("nodewipe"));
    let service_content = format!(
        "[Unit]\n\
         Description=nodewipe periodic doctor disk space check\n\
         Documentation=https://github.com/KADHIRAVANEG/nodewipe\n\n\
         [Service]\n\
         Type=oneshot\n\
         ExecStart={} doctor --notify --threshold {}\n",
        exe_path.display(),
        threshold
    );

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
        println!(" ✅  Enabled and started systemd user timer (nodewipe-doctor.timer).");
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
        println!(" ✅  Loaded LaunchAgent com.nodewipe.doctor.");
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
            println!(" ✅  Removed macOS LaunchAgent: {}", plist_path.display());
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
            println!(" ✅  Removed systemd user timer: {}", timer_path.display());
            removed = true;
        }
        if service_path.exists() {
            std::fs::remove_file(&service_path)?;
            println!(
                " ✅  Removed systemd user service: {}",
                service_path.display()
            );
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
    if path.len() <= max {
        return path.to_string();
    }
    let half = (max - 3) / 2;
    format!("{}...{}", &path[..half], &path[path.len() - half..])
}
