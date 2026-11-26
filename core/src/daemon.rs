use crate::dedupe::{DedupeOptions, dedupe_paths};
use crate::model::ScanOptions;
use crate::scanner::scan;
use anyhow::{Context, Result};
use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Real-time disk space capacity metrics for a given filesystem path.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct DiskSpaceInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub free_percent: f64,
}

/// Query filesystem storage capacity (total and available bytes) for a path.
#[cfg(unix)]
pub fn get_disk_space(path: &Path) -> std::io::Result<DiskSpaceInfo> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let probe_path = if path.exists() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    };

    let c_path = CString::new(probe_path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let res = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if res != 0 {
        return Err(std::io::Error::last_os_error());
    }

    let block_size = if stat.f_frsize != 0 {
        stat.f_frsize as u64
    } else {
        stat.f_bsize as u64
    };

    let total_bytes = stat.f_blocks as u64 * block_size;
    let available_bytes = stat.f_bavail as u64 * block_size;
    let used_bytes = total_bytes.saturating_sub(available_bytes);
    let free_percent = if total_bytes > 0 {
        (available_bytes as f64 / total_bytes as f64) * 100.0
    } else {
        0.0
    };

    Ok(DiskSpaceInfo {
        total_bytes,
        available_bytes,
        used_bytes,
        free_percent,
    })
}

#[cfg(windows)]
pub fn get_disk_space(path: &Path) -> std::io::Result<DiskSpaceInfo> {
    let probe_path = if path.exists() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    };

    let mut free_bytes_available: u64 = 0;
    let mut total_number_of_bytes: u64 = 0;
    let mut total_number_of_free_bytes: u64 = 0;

    let wide: Vec<u16> = probe_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let ok = windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_bytes_available,
            &mut total_number_of_bytes,
            &mut total_number_of_free_bytes,
        );
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }

    let total_bytes = total_number_of_bytes;
    let available_bytes = free_bytes_available;
    let used_bytes = total_bytes.saturating_sub(available_bytes);
    let free_percent = if total_bytes > 0 {
        (available_bytes as f64 / total_bytes as f64) * 100.0
    } else {
        0.0
    };

    Ok(DiskSpaceInfo {
        total_bytes,
        available_bytes,
        used_bytes,
        free_percent,
    })
}

#[cfg(not(any(unix, windows)))]
pub fn get_disk_space(_path: &Path) -> std::io::Result<DiskSpaceInfo> {
    Ok(DiskSpaceInfo {
        total_bytes: 500 * 1024 * 1024 * 1024,
        available_bytes: 200 * 1024 * 1024 * 1024,
        used_bytes: 300 * 1024 * 1024 * 1024,
        free_percent: 40.0,
    })
}

/// Check if a process identified by PID is currently alive on the system.
#[cfg(unix)]
pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let res = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if res == 0 {
        true
    } else {
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(windows)]
pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    if let Ok(output) = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.contains(&pid.to_string())
    } else {
        false
    }
}

#[cfg(not(any(unix, windows)))]
pub fn is_pid_alive(_pid: u32) -> bool {
    false
}

/// Send a graceful termination signal (SIGTERM on Unix, taskkill on Windows).
#[cfg(unix)]
pub fn terminate_process(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let res = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    res == 0
}

#[cfg(windows)]
pub fn terminate_process(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(any(unix, windows)))]
pub fn terminate_process(_pid: u32) -> bool {
    false
}

/// Default path for the daemon PID file (~/.config/nodewipe/daemon.pid).
pub fn default_pid_file() -> PathBuf {
    dirs::config_dir()
        .map(|d| d.join("nodewipe").join("daemon.pid"))
        .unwrap_or_else(|| PathBuf::from(".nodewipe-daemon.pid"))
}

/// Default path for daemon status state (~/.config/nodewipe/daemon_status.json).
pub fn default_status_file() -> PathBuf {
    dirs::config_dir()
        .map(|d| d.join("nodewipe").join("daemon_status.json"))
        .unwrap_or_else(|| PathBuf::from(".nodewipe-daemon-status.json"))
}

/// Read PID from file if present and valid.
pub fn read_pid(pid_path: &Path) -> Option<u32> {
    if !pid_path.exists() {
        return None;
    }
    let text = std::fs::read_to_string(pid_path).ok()?;
    text.trim().parse::<u32>().ok()
}

/// Write PID to file, creating parent directories if needed.
pub fn write_pid(pid_path: &Path, pid: u32) -> std::io::Result<()> {
    if let Some(parent) = pid_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(pid_path, format!("{pid}\n"))
}

/// Remove PID file if it exists.
pub fn remove_pid(pid_path: &Path) -> std::io::Result<()> {
    if pid_path.exists() {
        std::fs::remove_file(pid_path)?;
    }
    Ok(())
}

/// Configuration options for the nodewipe background watcher daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DaemonConfig {
    pub watch_paths: Vec<PathBuf>,
    pub size_threshold_bytes: Option<u64>,
    pub min_free_disk_percent: Option<f64>,
    pub check_interval_secs: u64,
    pub debounce_secs: u64,
    pub alert_cooldown_secs: u64,
    pub auto_dedupe: bool,
    pub pid_file: PathBuf,
    pub status_file: PathBuf,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            watch_paths: Vec::new(),
            size_threshold_bytes: Some(25 * 1024 * 1024 * 1024), // 25 GB default
            min_free_disk_percent: Some(15.0),                   // 15% free disk default
            check_interval_secs: 30,                             // 30 seconds
            debounce_secs: 5,                                    // 5 seconds debounce
            alert_cooldown_secs: 1800,                           // 30 minutes alert cooldown
            auto_dedupe: false,
            pid_file: default_pid_file(),
            status_file: default_status_file(),
        }
    }
}

/// Diagnostic alert generated when workspace bloat or disk pressure exceeds thresholds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskPressureAlert {
    pub timestamp: u64,
    pub path: PathBuf,
    pub total_reclaimable_bytes: u64,
    pub total_reclaimable_count: usize,
    pub disk_total_bytes: u64,
    pub disk_available_bytes: u64,
    pub disk_free_percent: f64,
    pub threshold_exceeded: bool,
    pub low_disk_space: bool,
    pub alert_title: String,
    pub alert_message: String,
    pub dedupe_performed: bool,
    pub dedupe_reclaimed_bytes: u64,
}

/// Comprehensive health and runtime status of the daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DaemonStatusInfo {
    pub pid: u32,
    pub is_alive: bool,
    pub is_stale_pid: bool,
    pub started_at: u64,
    pub uptime_secs: u64,
    pub last_check_at: u64,
    pub watch_paths: Vec<PathBuf>,
    pub size_threshold_bytes: Option<u64>,
    pub min_free_disk_percent: Option<f64>,
    pub check_interval_secs: u64,
    pub auto_dedupe: bool,
    pub last_reclaimable_bytes: u64,
    pub last_reclaimable_count: usize,
    pub last_disk_available_bytes: u64,
    pub last_disk_total_bytes: u64,
    pub last_disk_free_percent: f64,
    pub last_alert_at: Option<u64>,
    pub last_alert_message: Option<String>,
}

impl DaemonStatusInfo {
    pub fn empty() -> Self {
        Self {
            pid: 0,
            is_alive: false,
            is_stale_pid: false,
            started_at: 0,
            uptime_secs: 0,
            last_check_at: 0,
            watch_paths: Vec::new(),
            size_threshold_bytes: None,
            min_free_disk_percent: None,
            check_interval_secs: 30,
            auto_dedupe: false,
            last_reclaimable_bytes: 0,
            last_reclaimable_count: 0,
            last_disk_available_bytes: 0,
            last_disk_total_bytes: 0,
            last_disk_free_percent: 0.0,
            last_alert_at: None,
            last_alert_message: None,
        }
    }
}

/// Inspect the live status of the daemon from its PID and status files.
pub fn check_daemon_status(config: &DaemonConfig) -> DaemonStatusInfo {
    let pid_opt = read_pid(&config.pid_file);
    let is_alive = pid_opt.map(is_pid_alive).unwrap_or(false);
    let is_stale_pid = pid_opt.is_some() && !is_alive;

    let mut info = if let Ok(content) = std::fs::read_to_string(&config.status_file) {
        serde_json::from_str::<DaemonStatusInfo>(&content)
            .unwrap_or_else(|_| DaemonStatusInfo::empty())
    } else {
        DaemonStatusInfo::empty()
    };

    info.pid = pid_opt.unwrap_or(0);
    info.is_alive = is_alive;
    info.is_stale_pid = is_stale_pid;

    if is_alive && info.started_at > 0 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        info.uptime_secs = now.saturating_sub(info.started_at);
    } else if !is_alive {
        info.uptime_secs = 0;
    }

    info
}

/// Low-overhead cross-platform filesystem watcher wrapping OS native notification facilities.
pub struct WorkspaceWatcher {
    _watcher: RecommendedWatcher,
    event_rx: Receiver<Result<Event, notify::Error>>,
    watched_paths: Vec<PathBuf>,
}

impl WorkspaceWatcher {
    pub fn new() -> Result<Self> {
        let (tx, rx) = channel();
        let watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            NotifyConfig::default(),
        )
        .context("failed to initialize native filesystem watcher")?;

        Ok(Self {
            _watcher: watcher,
            event_rx: rx,
            watched_paths: Vec::new(),
        })
    }

    /// Watch a directory tree recursively with fallback to non-recursive watch if inotify limits occur.
    pub fn watch_safe(&mut self, path: &Path) -> Result<(), String> {
        if !path.exists() {
            return Err(format!("path does not exist: {}", path.display()));
        }

        match self._watcher.watch(path, RecursiveMode::Recursive) {
            Ok(()) => {
                self.watched_paths.push(path.to_path_buf());
                Ok(())
            }
            Err(e) => {
                // If recursive watch fails (e.g. EMFILE, ENOSPC on Linux inotify watches), fall back
                match self._watcher.watch(path, RecursiveMode::NonRecursive) {
                    Ok(()) => {
                        self.watched_paths.push(path.to_path_buf());
                        Ok(())
                    }
                    Err(fallback_err) => Err(format!(
                        "recursive watch failed: {e}; non-recursive fallback also failed: {fallback_err}"
                    )),
                }
            }
        }
    }

    /// Drain any pending events without blocking.
    pub fn poll_events(&self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(res) = self.event_rx.try_recv() {
            if let Ok(event) = res {
                events.push(event);
            }
        }
        events
    }

    pub fn watched_paths(&self) -> &[PathBuf] {
        &self.watched_paths
    }
}

/// Evaluates disk pressure and reclaimable artifact bloat for a target path.
pub fn evaluate_disk_pressure(
    path: &Path,
    threshold_bytes: Option<u64>,
    min_free_percent: Option<f64>,
) -> Result<DiskPressureAlert> {
    let disk_info = get_disk_space(path).unwrap_or(DiskSpaceInfo {
        total_bytes: 0,
        available_bytes: 0,
        used_bytes: 0,
        free_percent: 100.0,
    });

    let scan_opts = ScanOptions {
        root: path.to_path_buf(),
        compute_sizes: true,
        parallel: true,
        inspect_git: false,
        ..Default::default()
    };

    let entries = scan(&scan_opts).unwrap_or_default();
    let total_reclaimable_bytes: u64 = entries.iter().map(|e| e.size_bytes).sum();
    let total_reclaimable_count = entries.len();

    let threshold_exceeded = threshold_bytes
        .map(|limit| total_reclaimable_bytes >= limit)
        .unwrap_or(false);

    let low_disk_space = min_free_percent
        .map(|min_pct| disk_info.free_percent < min_pct)
        .unwrap_or(false);

    let mut reasons = Vec::new();
    if let Some(limit) = threshold_bytes
        && threshold_exceeded
    {
        reasons.push(format!(
            "Reclaimable dev artifacts exceed {} (currently {})",
            format_bytes(limit),
            format_bytes(total_reclaimable_bytes)
        ));
    }

    if let Some(min_pct) = min_free_percent
        && low_disk_space
    {
        reasons.push(format!(
            "Available disk space is {:.1}% (below {:.1}% minimum, {} free)",
            disk_info.free_percent,
            min_pct,
            format_bytes(disk_info.available_bytes)
        ));
    }

    let alert_title = if low_disk_space && threshold_exceeded {
        "nodewipe: Critical Storage Pressure Warning".to_string()
    } else if low_disk_space {
        "nodewipe: Low Disk Space Alert".to_string()
    } else if threshold_exceeded {
        "nodewipe: High Dev Artifact Bloat Detected".to_string()
    } else {
        "nodewipe: Storage Diagnostic Healthy".to_string()
    };

    let alert_message = if reasons.is_empty() {
        format!(
            "Workspace {} has {} across {} dev artifact directories. Free disk: {:.1}%.",
            path.display(),
            format_bytes(total_reclaimable_bytes),
            total_reclaimable_count,
            disk_info.free_percent
        )
    } else {
        format!(
            "{} in {}. Reclaimable: {} across {} directories.",
            reasons.join("; "),
            path.display(),
            format_bytes(total_reclaimable_bytes),
            total_reclaimable_count
        )
    };

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Ok(DiskPressureAlert {
        timestamp,
        path: path.to_path_buf(),
        total_reclaimable_bytes,
        total_reclaimable_count,
        disk_total_bytes: disk_info.total_bytes,
        disk_available_bytes: disk_info.available_bytes,
        disk_free_percent: disk_info.free_percent,
        threshold_exceeded,
        low_disk_space,
        alert_title,
        alert_message,
        dedupe_performed: false,
        dedupe_reclaimed_bytes: 0,
    })
}

/// Formatting helper for human-readable byte sizes.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

/// Runner for the background daemon event loop.
pub struct DaemonRunner {
    config: DaemonConfig,
    stop_signal: Arc<AtomicBool>,
    status: Arc<Mutex<DaemonStatusInfo>>,
}

impl DaemonRunner {
    pub fn new(config: DaemonConfig, stop_signal: Arc<AtomicBool>) -> Self {
        let status = Arc::new(Mutex::new(DaemonStatusInfo {
            pid: std::process::id(),
            is_alive: true,
            is_stale_pid: false,
            started_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            uptime_secs: 0,
            last_check_at: 0,
            watch_paths: config.watch_paths.clone(),
            size_threshold_bytes: config.size_threshold_bytes,
            min_free_disk_percent: config.min_free_disk_percent,
            check_interval_secs: config.check_interval_secs,
            auto_dedupe: config.auto_dedupe,
            last_reclaimable_bytes: 0,
            last_reclaimable_count: 0,
            last_disk_available_bytes: 0,
            last_disk_total_bytes: 0,
            last_disk_free_percent: 0.0,
            last_alert_at: None,
            last_alert_message: None,
        }));

        Self {
            config,
            stop_signal,
            status,
        }
    }

    /// Execute the daemon loop with an alert dispatch callback.
    pub fn run<F>(&mut self, mut alert_callback: F) -> Result<()>
    where
        F: FnMut(&DiskPressureAlert),
    {
        // 1. Write PID file
        let my_pid = std::process::id();
        write_pid(&self.config.pid_file, my_pid).context("failed to write daemon PID file")?;

        // 2. Initialize native filesystem watcher
        let mut watcher = WorkspaceWatcher::new()?;
        let mut registered_paths = Vec::new();

        let paths_to_watch = if self.config.watch_paths.is_empty() {
            vec![std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))]
        } else {
            self.config.watch_paths.clone()
        };

        for p in &paths_to_watch {
            if let Ok(canon) = std::fs::canonicalize(p) {
                if watcher.watch_safe(&canon).is_ok() {
                    registered_paths.push(canon);
                }
            } else if watcher.watch_safe(p).is_ok() {
                registered_paths.push(p.clone());
            }
        }

        if let Ok(mut st) = self.status.lock() {
            st.watch_paths = registered_paths.clone();
        }
        self.save_status_snapshot();

        let mut last_alert_time: Option<u64> = None;
        let mut last_eval_time = SystemTime::UNIX_EPOCH;
        let debounce_duration = Duration::from_secs(self.config.debounce_secs.max(1));
        let check_interval = Duration::from_secs(self.config.check_interval_secs.max(5));

        // 3. Event and polling loop
        while !self.stop_signal.load(Ordering::Relaxed) {
            let drained_events = watcher.poll_events();
            let has_fs_events = !drained_events.is_empty();
            let elapsed_since_eval = last_eval_time.elapsed().unwrap_or(check_interval);

            let should_evaluate = has_fs_events || elapsed_since_eval >= check_interval;

            if should_evaluate {
                // If triggered by fs events, wait debounce duration to let compiler batches settle
                if has_fs_events {
                    std::thread::sleep(debounce_duration);
                    // Drain any trailing compiler noise
                    let _ = watcher.poll_events();
                }

                self.evaluate_workspaces(
                    &registered_paths,
                    &mut last_alert_time,
                    &mut alert_callback,
                );
                last_eval_time = SystemTime::now();
                self.save_status_snapshot();
            }

            // Sleep briefly to yield CPU (<0.1% CPU idle consumption)
            for _ in 0..10 {
                if self.stop_signal.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        // Cleanup on graceful shutdown
        let _ = remove_pid(&self.config.pid_file);
        let _ = std::fs::remove_file(&self.config.status_file);

        Ok(())
    }

    fn evaluate_workspaces<F>(
        &self,
        paths: &[PathBuf],
        last_alert_time: &mut Option<u64>,
        alert_callback: &mut F,
    ) where
        F: FnMut(&DiskPressureAlert),
    {
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        for path in paths {
            if let Ok(mut alert) = evaluate_disk_pressure(
                path,
                self.config.size_threshold_bytes,
                self.config.min_free_disk_percent,
            ) {
                // Auto deduplication if enabled and threshold exceeded
                if self.config.auto_dedupe && (alert.threshold_exceeded || alert.low_disk_space) {
                    let dedupe_opts = DedupeOptions {
                        dry_run: false,
                        allow_hardlinks: false,
                        min_size_bytes: 1024 * 1024, // 1MB min
                    };
                    if let Ok(stats) = dedupe_paths(std::slice::from_ref(path), &dedupe_opts) {
                        alert.dedupe_performed = true;
                        alert.dedupe_reclaimed_bytes = stats.reclaimed_bytes;
                    }
                }

                // Update runtime status snapshot
                if let Ok(mut st) = self.status.lock() {
                    st.last_check_at = now_secs;
                    st.last_reclaimable_bytes = alert.total_reclaimable_bytes;
                    st.last_reclaimable_count = alert.total_reclaimable_count;
                    st.last_disk_available_bytes = alert.disk_available_bytes;
                    st.last_disk_total_bytes = alert.disk_total_bytes;
                    st.last_disk_free_percent = alert.disk_free_percent;
                }

                let should_alert = alert.threshold_exceeded || alert.low_disk_space;
                let cooldown_expired = match *last_alert_time {
                    None => true,
                    Some(t) => now_secs.saturating_sub(t) >= self.config.alert_cooldown_secs,
                };

                if should_alert && cooldown_expired {
                    alert_callback(&alert);
                    *last_alert_time = Some(now_secs);

                    if let Ok(mut st) = self.status.lock() {
                        st.last_alert_at = Some(now_secs);
                        st.last_alert_message = Some(alert.alert_message.clone());
                    }
                }
            }
        }
    }

    fn save_status_snapshot(&self) {
        if let Ok(mut st) = self.status.lock() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            st.uptime_secs = now.saturating_sub(st.started_at);

            if let Some(parent) = self.config.status_file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(&*st) {
                let _ = std::fs::write(&self.config.status_file, json);
            }
        }
    }
}

// ── Native Service File Generators ──────────────────────────────────────────

/// Generate content for a Linux systemd user service running the background daemon.
pub fn generate_systemd_daemon_service(exe_path: &Path, config: &DaemonConfig) -> String {
    let mut args = vec![
        "daemon".to_string(),
        "start".to_string(),
        "--foreground".to_string(),
    ];

    for wp in &config.watch_paths {
        args.push(format!("--watch-dir=\"{}\"", wp.display()));
    }

    if let Some(limit) = config.size_threshold_bytes {
        args.push(format!(
            "--threshold={}",
            format_bytes(limit).replace(' ', "")
        ));
    }

    if let Some(pct) = config.min_free_disk_percent {
        args.push(format!("--min-free-percent={pct:.1}"));
    }

    args.push(format!("--interval={}", config.check_interval_secs));

    if config.auto_dedupe {
        args.push("--auto-dedupe".to_string());
    }

    format!(
        "[Unit]\n\
         Description=nodewipe real-time workspace watcher and disk pressure alert daemon\n\
         Documentation=https://github.com/KADHIRAVANEG/nodewipe\n\
         After=default.target\n\n\
         [Service]\n\
         Type=simple\n\
         ExecStart=\"{}\" {}\n\
         Restart=on-failure\n\
         RestartSec=10\n\n\
         [Install]\n\
         WantedBy=default.target\n",
        exe_path.display(),
        args.join(" ")
    )
}

/// Generate content for a macOS launchd LaunchAgent plist.
pub fn generate_macos_daemon_plist(exe_path: &Path, config: &DaemonConfig) -> String {
    let mut args_xml = String::new();
    args_xml.push_str(&format!(
        "        <string>{}</string>\n",
        exe_path.display()
    ));
    args_xml.push_str("        <string>daemon</string>\n");
    args_xml.push_str("        <string>start</string>\n");
    args_xml.push_str("        <string>--foreground</string>\n");

    for wp in &config.watch_paths {
        args_xml.push_str("        <string>--watch-dir</string>\n");
        args_xml.push_str(&format!("        <string>{}</string>\n", wp.display()));
    }

    if let Some(limit) = config.size_threshold_bytes {
        args_xml.push_str("        <string>--threshold</string>\n");
        args_xml.push_str(&format!(
            "        <string>{}</string>\n",
            format_bytes(limit).replace(' ', "")
        ));
    }

    if let Some(pct) = config.min_free_disk_percent {
        args_xml.push_str("        <string>--min-free-percent</string>\n");
        args_xml.push_str(&format!("        <string>{pct:.1}</string>\n"));
    }

    args_xml.push_str("        <string>--interval</string>\n");
    args_xml.push_str(&format!(
        "        <string>{}</string>\n",
        config.check_interval_secs
    ));

    if config.auto_dedupe {
        args_xml.push_str("        <string>--auto-dedupe</string>\n");
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
             <key>Label</key>\n\
             <string>com.nodewipe.daemon</string>\n\
             <key>ProgramArguments</key>\n\
             <array>\n\
     {}\
             </array>\n\
             <key>RunAtLoad</key>\n\
             <true/>\n\
             <key>KeepAlive</key>\n\
             <true/>\n\
             <key>StandardErrorPath</key>\n\
             <string>/tmp/nodewipe-daemon.err</string>\n\
             <key>StandardOutPath</key>\n\
             <string>/tmp/nodewipe-daemon.out</string>\n\
         </dict>\n\
         </plist>\n",
        args_xml
    )
}

/// Install native background service for current OS.
pub fn install_daemon_service(config: &DaemonConfig) -> Result<PathBuf> {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("nodewipe"));
    if cfg!(target_os = "macos") {
        let home = dirs::home_dir().context("could not determine user home directory")?;
        let agents_dir = home.join("Library").join("LaunchAgents");
        std::fs::create_dir_all(&agents_dir)?;
        let plist_path = agents_dir.join("com.nodewipe.daemon.plist");
        let content = generate_macos_daemon_plist(&exe, config);
        std::fs::write(&plist_path, content)?;

        let _ = std::process::Command::new("launchctl")
            .args(["load", "-w", &plist_path.to_string_lossy()])
            .status();

        Ok(plist_path)
    } else {
        let home = dirs::home_dir().context("could not determine user home directory")?;
        let user_systemd_dir = home.join(".config").join("systemd").join("user");
        std::fs::create_dir_all(&user_systemd_dir)?;
        let service_path = user_systemd_dir.join("nodewipe-daemon.service");
        let content = generate_systemd_daemon_service(&exe, config);
        std::fs::write(&service_path, content)?;

        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "enable", "--now", "nodewipe-daemon.service"])
            .status();

        Ok(service_path)
    }
}

/// Uninstall native background service for current OS.
pub fn uninstall_daemon_service() -> Result<bool> {
    if cfg!(target_os = "macos") {
        let home = dirs::home_dir().context("could not determine user home directory")?;
        let plist_path = home
            .join("Library")
            .join("LaunchAgents")
            .join("com.nodewipe.daemon.plist");
        if plist_path.exists() {
            let _ = std::process::Command::new("launchctl")
                .args(["unload", "-w", &plist_path.to_string_lossy()])
                .status();
            std::fs::remove_file(&plist_path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    } else {
        let home = dirs::home_dir().context("could not determine user home directory")?;
        let user_systemd_dir = home.join(".config").join("systemd").join("user");
        let service_path = user_systemd_dir.join("nodewipe-daemon.service");

        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "--now", "nodewipe-daemon.service"])
            .status();

        if service_path.exists() {
            std::fs::remove_file(&service_path)?;
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_disk_space_query() {
        let cwd = std::env::current_dir().unwrap();
        let space = get_disk_space(&cwd).expect("get_disk_space should succeed on current dir");
        assert!(space.total_bytes > 0, "total disk space must be non-zero");
        assert!(space.available_bytes <= space.total_bytes);
        assert!(space.free_percent >= 0.0 && space.free_percent <= 100.0);
    }

    #[test]
    fn test_pid_management_and_stale_detection() {
        let dir = tempdir().unwrap();
        let pid_path = dir.path().join("test_daemon.pid");
        let current_pid = std::process::id();

        // Initially does not exist
        assert_eq!(read_pid(&pid_path), None);

        // Write live PID
        write_pid(&pid_path, current_pid).unwrap();
        assert_eq!(read_pid(&pid_path), Some(current_pid));
        assert!(is_pid_alive(current_pid));

        // Fake dead PID (using a high PID that is virtually guaranteed not to exist, e.g. 9999999)
        let dead_pid = 9_999_999;
        write_pid(&pid_path, dead_pid).unwrap();
        assert_eq!(read_pid(&pid_path), Some(dead_pid));
        assert!(!is_pid_alive(dead_pid));

        // Removal
        remove_pid(&pid_path).unwrap();
        assert_eq!(read_pid(&pid_path), None);
    }

    #[test]
    fn test_evaluate_disk_pressure_threshold_trigger() {
        let dir = tempdir().unwrap();
        // Create simulated node_modules
        let nm = dir.path().join("node_modules");
        std::fs::create_dir_all(&nm).unwrap();
        std::fs::write(nm.join("package.json"), "{}").unwrap();
        std::fs::write(nm.join("dummy.bin"), vec![0u8; 10000]).unwrap();

        // 1. With very low threshold (100 bytes) -> should trigger
        let alert = evaluate_disk_pressure(dir.path(), Some(100), None).unwrap();
        assert!(alert.threshold_exceeded);
        assert!(alert.total_reclaimable_bytes >= 10000);
        assert_eq!(alert.total_reclaimable_count, 1);
        assert!(alert.alert_title.contains("Dev Artifact Bloat"));

        // 2. With high threshold (100 GB) -> should not trigger
        let alert_high =
            evaluate_disk_pressure(dir.path(), Some(100 * 1024 * 1024 * 1024), None).unwrap();
        assert!(!alert_high.threshold_exceeded);
        assert!(alert_high.alert_title.contains("Healthy"));
    }

    #[test]
    fn test_systemd_service_file_generation() {
        let exe = Path::new("/usr/local/bin/nodewipe");
        let config = DaemonConfig {
            watch_paths: vec![PathBuf::from("/home/dev/projects")],
            size_threshold_bytes: Some(20 * 1024 * 1024 * 1024),
            min_free_disk_percent: Some(12.5),
            check_interval_secs: 45,
            auto_dedupe: true,
            ..Default::default()
        };

        let service = generate_systemd_daemon_service(exe, &config);
        assert!(
            service.contains("ExecStart=\"/usr/local/bin/nodewipe\" daemon start --foreground")
        );
        assert!(service.contains("--watch-dir=\"/home/dev/projects\""));
        assert!(service.contains("--threshold=20.0GB"));
        assert!(service.contains("--min-free-percent=12.5"));
        assert!(service.contains("--interval=45"));
        assert!(service.contains("--auto-dedupe"));
        assert!(service.contains("Restart=on-failure"));
    }

    #[test]
    fn test_macos_plist_generation() {
        let exe = Path::new("/opt/homebrew/bin/nodewipe");
        let config = DaemonConfig {
            watch_paths: vec![PathBuf::from("/Users/dev/workspace")],
            size_threshold_bytes: Some(10 * 1024 * 1024 * 1024),
            min_free_disk_percent: Some(20.0),
            check_interval_secs: 60,
            auto_dedupe: false,
            ..Default::default()
        };

        let plist = generate_macos_daemon_plist(exe, &config);
        assert!(plist.contains("<string>com.nodewipe.daemon</string>"));
        assert!(plist.contains("<string>/opt/homebrew/bin/nodewipe</string>"));
        assert!(plist.contains("<string>--watch-dir</string>"));
        assert!(plist.contains("<string>/Users/dev/workspace</string>"));
        assert!(plist.contains("<string>--threshold</string>"));
        assert!(plist.contains("<string>10.0GB</string>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains("<true/>"));
    }

    #[test]
    fn test_daemon_status_serialization() {
        let mut status = DaemonStatusInfo::empty();
        status.pid = 12345;
        status.is_alive = true;
        status.watch_paths = vec![PathBuf::from("/test/path")];
        status.last_reclaimable_bytes = 42_000_000;

        let json = serde_json::to_string(&status).unwrap();
        let parsed: DaemonStatusInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.pid, 12345);
        assert!(parsed.is_alive);
        assert_eq!(parsed.watch_paths, vec![PathBuf::from("/test/path")]);
        assert_eq!(parsed.last_reclaimable_bytes, 42_000_000);
    }
}
