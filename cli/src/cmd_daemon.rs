/// `nodewipe daemon` — background real-time filesystem watcher and disk pressure alert service.
use anyhow::{Context, Result};
use nodewipe_core::daemon::{
    DaemonConfig, DaemonRunner, check_daemon_status, evaluate_disk_pressure, format_bytes,
    install_daemon_service, is_pid_alive, read_pid, remove_pid, terminate_process,
    uninstall_daemon_service,
};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::cli_def::{DaemonAction, DaemonServiceAction};
use crate::cmd_doctor::{parse_size_threshold, send_desktop_notification};

pub fn run(action: DaemonAction) -> Result<ExitCode> {
    match action {
        DaemonAction::Start {
            watch_dirs,
            threshold,
            min_free_percent,
            interval,
            auto_dedupe,
            foreground,
        } => handle_start(
            watch_dirs,
            threshold,
            min_free_percent,
            interval,
            auto_dedupe,
            foreground,
        ),
        DaemonAction::Stop => handle_stop(),
        DaemonAction::Status { json } => handle_status(json),
        DaemonAction::Check {
            watch_dirs,
            threshold,
            min_free_percent,
            notify,
        } => handle_check(watch_dirs, threshold, min_free_percent, notify),
        DaemonAction::Service { action } => handle_service(action),
    }
}

fn handle_start(
    watch_dirs: Vec<PathBuf>,
    threshold: String,
    min_free_percent: f64,
    interval: u64,
    auto_dedupe: bool,
    foreground: bool,
) -> Result<ExitCode> {
    let threshold_bytes =
        parse_size_threshold(&threshold).context("invalid threshold size (e.g. 25GB, 500MB)")?;
    let target_watch_dirs = if watch_dirs.is_empty() {
        vec![std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))]
    } else {
        watch_dirs
    };
    let config = DaemonConfig {
        size_threshold_bytes: Some(threshold_bytes),
        min_free_disk_percent: Some(min_free_percent),
        check_interval_secs: interval,
        auto_dedupe,
        watch_paths: target_watch_dirs,
        ..Default::default()
    };

    // Check existing PID
    if let Some(existing_pid) = read_pid(&config.pid_file) {
        if is_pid_alive(existing_pid) {
            println!(
                "nodewipe daemon is already running (PID {}). Use 'nodewipe daemon status' or 'nodewipe daemon stop'.",
                existing_pid
            );
            return Ok(ExitCode::SUCCESS);
        } else {
            println!(
                "Cleaning up stale daemon PID file (PID {} was not running).",
                existing_pid
            );
            let _ = remove_pid(&config.pid_file);
        }
    }

    if foreground {
        println!("Starting nodewipe daemon in foreground...");
        println!(
            "Watching {} workspace path(s) | Threshold: {} | Min free disk: {:.1}% | Interval: {}s",
            config.watch_paths.len(),
            format_bytes(threshold_bytes),
            min_free_percent,
            interval
        );
        for wp in &config.watch_paths {
            println!("  - {}", wp.display());
        }

        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_signal);

        // Trap SIGINT / SIGTERM for graceful shutdown
        ctrlc_handler(stop_clone);

        let mut runner = DaemonRunner::new(config, stop_signal);
        runner.run(|alert| {
            println!(
                "[{}] ALERT: {} - {}",
                nodewipe_core::icons::DOCTOR,
                alert.alert_title,
                alert.alert_message
            );
            send_desktop_notification(&alert.alert_title, &alert.alert_message);
        })?;

        println!("nodewipe daemon shut down gracefully.");
        Ok(ExitCode::SUCCESS)
    } else {
        // Spawn background detached daemon process
        let current_exe =
            std::env::current_exe().context("could not determine nodewipe binary path")?;
        let mut cmd = std::process::Command::new(current_exe);
        cmd.arg("daemon").arg("start").arg("--foreground");

        for wp in &config.watch_paths {
            cmd.arg("--watch-dir").arg(wp);
        }
        cmd.arg("--threshold").arg(&threshold);
        cmd.arg("--min-free-percent")
            .arg(min_free_percent.to_string());
        cmd.arg("--interval").arg(interval.to_string());
        if auto_dedupe {
            cmd.arg("--auto-dedupe");
        }

        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        let child = cmd
            .spawn()
            .context("failed to spawn background daemon process")?;
        let pid = child.id();

        println!(
            "nodewipe daemon started in background (PID {}).\nMonitoring {} workspace path(s) with {} threshold.",
            pid,
            config.watch_paths.len(),
            format_bytes(threshold_bytes)
        );
        println!(
            "Run 'nodewipe daemon status' to view live metrics or 'nodewipe daemon stop' to terminate."
        );

        Ok(ExitCode::SUCCESS)
    }
}

fn handle_stop() -> Result<ExitCode> {
    let config = DaemonConfig::default();
    let pid_opt = read_pid(&config.pid_file);

    match pid_opt {
        None => {
            println!("No nodewipe daemon PID file found. Daemon is not running.");
            Ok(ExitCode::SUCCESS)
        }
        Some(pid) => {
            if is_pid_alive(pid) {
                print!("Stopping nodewipe daemon (PID {})... ", pid);
                let _ = terminate_process(pid);

                // Wait up to 5 seconds for termination
                let mut stopped = false;
                for _ in 0..50 {
                    if !is_pid_alive(pid) {
                        stopped = true;
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }

                if stopped {
                    println!("Stopped.");
                } else {
                    println!("Sent termination signal.");
                }
            } else {
                println!(
                    "Found stale daemon PID file (PID {} is not running). Removing.",
                    pid
                );
            }

            let _ = remove_pid(&config.pid_file);
            let _ = std::fs::remove_file(&config.status_file);
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn handle_status(json: bool) -> Result<ExitCode> {
    let config = DaemonConfig::default();
    let status = check_daemon_status(&config);

    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(ExitCode::SUCCESS);
    }

    println!("\n  nodewipe Background Watcher Daemon Status");
    println!("  =========================================");

    if status.is_alive {
        println!("  State:               RUNNING");
        println!("  PID:                 {}", status.pid);
        println!("  Uptime:              {}s", status.uptime_secs);
        println!("  Check Interval:      {}s", status.check_interval_secs);
        println!(
            "  Auto CoW Reflink:    {}",
            if status.auto_dedupe {
                "enabled"
            } else {
                "disabled"
            }
        );

        if let Some(threshold) = status.size_threshold_bytes {
            println!("  Alert Threshold:     {}", format_bytes(threshold));
        }
        if let Some(pct) = status.min_free_disk_percent {
            println!("  Min Free Disk:       {:.1}%", pct);
        }

        println!("\n  Monitored Workspaces ({}):", status.watch_paths.len());
        if status.watch_paths.is_empty() {
            println!("    (none registered)");
        } else {
            for wp in &status.watch_paths {
                println!("    - {}", wp.display());
            }
        }

        println!("\n  Storage Capacity Diagnostics:");
        println!(
            "    Available Disk Space: {} / {} ({:.1}% free)",
            format_bytes(status.last_disk_available_bytes),
            format_bytes(status.last_disk_total_bytes),
            status.last_disk_free_percent
        );
        println!(
            "    Reclaimable Bloat:    {} across {} dev directories",
            format_bytes(status.last_reclaimable_bytes),
            status.last_reclaimable_count
        );

        if let Some(msg) = &status.last_alert_message {
            println!("\n  Last Alert:          {}", msg);
        }
    } else if status.is_stale_pid {
        println!(
            "  State:               STALE PID (process {} exited abnormally)",
            status.pid
        );
        println!(
            "  Remedy:              Run 'nodewipe daemon start' to restart or 'nodewipe daemon stop' to clear."
        );
    } else {
        println!("  State:               STOPPED");
        println!(
            "  Remedy:              Run 'nodewipe daemon start' to activate real-time disk pressure alerts."
        );
    }

    println!();
    Ok(ExitCode::SUCCESS)
}

fn handle_check(
    watch_dirs: Vec<PathBuf>,
    threshold: String,
    min_free_percent: f64,
    notify: bool,
) -> Result<ExitCode> {
    let threshold_bytes = parse_size_threshold(&threshold)?;
    let target_dirs = if watch_dirs.is_empty() {
        vec![std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))]
    } else {
        watch_dirs
    };

    println!(
        "\n  nodewipe daemon check — Evaluating {} workspace(s) [Threshold: {}, Min Free: {:.1}%]\n",
        target_dirs.len(),
        format_bytes(threshold_bytes),
        min_free_percent
    );

    let mut total_bloat: u64 = 0;
    let mut total_count: usize = 0;
    let mut any_pressure = false;

    for dir in &target_dirs {
        match evaluate_disk_pressure(dir, Some(threshold_bytes), Some(min_free_percent)) {
            Ok(alert) => {
                total_bloat += alert.total_reclaimable_bytes;
                total_count += alert.total_reclaimable_count;
                let status_label = if alert.threshold_exceeded || alert.low_disk_space {
                    any_pressure = true;
                    "[PRESSURE DETECTED]"
                } else {
                    "[HEALTHY]"
                };

                println!(
                    "  {:<20} {} — Reclaimable: {} ({}), Free Disk: {:.1}%",
                    status_label,
                    dir.display(),
                    format_bytes(alert.total_reclaimable_bytes),
                    alert.total_reclaimable_count,
                    alert.disk_free_percent
                );

                if (alert.threshold_exceeded || alert.low_disk_space) && notify {
                    send_desktop_notification(&alert.alert_title, &alert.alert_message);
                }
            }
            Err(e) => {
                eprintln!("  [ERROR] {}: {e}", dir.display());
            }
        }
    }

    println!(
        "\n  Summary: {} total reclaimable dev bloat across {} directories.",
        format_bytes(total_bloat),
        total_count
    );

    if any_pressure {
        println!("  Disk pressure warning: One or more targets exceeded storage limits.\n");
    } else {
        println!("  All target workspaces are within healthy storage limits.\n");
    }

    Ok(ExitCode::SUCCESS)
}

fn handle_service(action: DaemonServiceAction) -> Result<ExitCode> {
    match action {
        DaemonServiceAction::Install {
            watch_dirs,
            threshold,
            min_free_percent,
            interval,
            auto_dedupe,
        } => {
            let threshold_bytes = parse_size_threshold(&threshold)?;
            let target_watch_paths = if watch_dirs.is_empty() {
                vec![dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))]
            } else {
                watch_dirs
            };
            let config = DaemonConfig {
                size_threshold_bytes: Some(threshold_bytes),
                min_free_disk_percent: Some(min_free_percent),
                check_interval_secs: interval,
                auto_dedupe,
                watch_paths: target_watch_paths,
                ..Default::default()
            };

            let service_path = install_daemon_service(&config)?;
            println!(
                "Installed nodewipe background daemon service: {}",
                service_path.display()
            );
            println!("Service is configured to start automatically and keep alive.");
            Ok(ExitCode::SUCCESS)
        }
        DaemonServiceAction::Uninstall => {
            let removed = uninstall_daemon_service()?;
            if removed {
                println!("nodewipe background daemon service was successfully uninstalled.");
            } else {
                println!("No installed nodewipe background daemon service files were found.");
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

#[cfg(unix)]
fn ctrlc_handler(stop_signal: Arc<AtomicBool>) {
    static STOPPED: AtomicBool = AtomicBool::new(false);

    extern "C" fn handle_signal(_: libc::c_int) {
        STOPPED.store(true, Ordering::SeqCst);
    }

    unsafe {
        libc::signal(
            libc::SIGINT,
            handle_signal as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            handle_signal as *const () as libc::sighandler_t,
        );
    }

    std::thread::spawn(move || {
        while !stop_signal.load(Ordering::Relaxed) {
            if STOPPED.load(Ordering::Relaxed) {
                stop_signal.store(true, Ordering::SeqCst);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}

#[cfg(not(unix))]
fn ctrlc_handler(_stop_signal: Arc<AtomicBool>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_cmd_daemon_status_stopped() {
        let code = handle_status(false).expect("status command should succeed");
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn test_cmd_daemon_status_json_contract() {
        let code = handle_status(true).expect("status json command should succeed");
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn test_cmd_daemon_check_healthy_workspace() {
        let dir = tempdir().unwrap();
        let code = handle_check(
            vec![dir.path().to_path_buf()],
            "10GB".to_string(),
            10.0,
            false,
        )
        .expect("check command should succeed");
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn test_cmd_daemon_check_bloat_detected() {
        let dir = tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        std::fs::create_dir_all(&nm).unwrap();
        std::fs::write(nm.join("package.json"), "{}").unwrap();
        std::fs::write(nm.join("heavy.dat"), vec![0u8; 5000]).unwrap();

        let code = handle_check(
            vec![dir.path().to_path_buf()],
            "1KB".to_string(),
            5.0,
            false,
        )
        .expect("check should report pressure successfully");
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn test_cmd_daemon_stop_when_not_running() {
        let code = handle_stop().expect("stop should succeed gracefully when daemon not running");
        assert_eq!(code, ExitCode::SUCCESS);
    }
}
