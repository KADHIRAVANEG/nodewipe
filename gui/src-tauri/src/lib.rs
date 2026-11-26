use nodewipe_core::daemon::{
    DaemonConfig, check_daemon_status, read_pid, remove_pid, terminate_process,
};
use nodewipe_core::history::{
    daily_totals, format_date, monthly_totals, now_unix, stats_by_ecosystem, weekly_totals,
};
use nodewipe_core::{
    DeleteMode, Entry, HistoryEntry, ScanOptions, WorkspaceGroup, annotate_workspace_roots,
    delete as core_delete, doctor::build_report, find_archives as core_find_archives,
    group_by_workspace, history_append, history_load, load_ignore_patterns,
    restore as core_restore, scan as core_scan,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent};

/// Shared application state for tray minimize-to-tray behavior.
struct GuiAppState {
    minimize_to_tray: Arc<AtomicBool>,
}

/// Persisted user preferences for the desktop GUI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuiPreferences {
    pub theme: String,
    pub accent: String,
    pub minimize_to_tray: bool,
}

impl Default for GuiPreferences {
    fn default() -> Self {
        Self {
            theme: "system".to_string(),
            accent: "cobalt".to_string(),
            minimize_to_tray: true,
        }
    }
}

impl GuiPreferences {
    fn sanitized(mut self) -> Self {
        self.theme = match self.theme.as_str() {
            "dark" | "light" | "system" => self.theme,
            _ => "system".to_string(),
        };
        self.accent = match self.accent.as_str() {
            "cobalt" | "emerald" | "amber" | "violet" => self.accent,
            _ => "cobalt".to_string(),
        };
        self
    }
}

fn prefs_file_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("nodewipe").join("gui_prefs.json"))
}

fn load_prefs_from_path(path: &PathBuf) -> GuiPreferences {
    if !path.exists() {
        return GuiPreferences::default();
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<GuiPreferences>(&s).ok())
        .map(GuiPreferences::sanitized)
        .unwrap_or_default()
}

fn save_prefs_to_path(path: &PathBuf, prefs: &GuiPreferences) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let clean = prefs.clone().sanitized();
    let json = serde_json::to_string_pretty(&clean).map_err(|e| e.to_string())?;
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Validate raw path strings received over Tauri IPC.
pub fn validate_ipc_path(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty".to_string());
    }
    if trimmed.contains('\0') {
        return Err("path contains invalid NUL byte".to_string());
    }
    Ok(PathBuf::from(trimmed))
}

#[tauri::command(async)]
fn scan_command(root: String) -> Result<Vec<Entry>, String> {
    let root_path = validate_ipc_path(&root)?;
    let opts = ScanOptions {
        ignore_patterns: load_ignore_patterns(&root_path),
        root: root_path,
        ..Default::default()
    };
    let mut entries = core_scan(&opts).map_err(|e| e.to_string())?;
    annotate_workspace_roots(&mut entries, &opts.root);
    entries.sort_by_key(|b| std::cmp::Reverse(b.size_bytes));
    Ok(entries)
}

#[tauri::command(async)]
fn scan_grouped_command(root: String) -> Result<Vec<WorkspaceGroup>, String> {
    let root_path = validate_ipc_path(&root)?;
    let opts = ScanOptions {
        ignore_patterns: load_ignore_patterns(&root_path),
        root: root_path,
        ..Default::default()
    };
    let mut entries = core_scan(&opts).map_err(|e| e.to_string())?;
    annotate_workspace_roots(&mut entries, &opts.root);
    let groups = group_by_workspace(entries);
    let mut groups: Vec<WorkspaceGroup> = groups.into_values().collect();
    groups.sort_by_key(|b| std::cmp::Reverse(b.total_size_bytes));
    for g in &mut groups {
        g.entries.sort_by_key(|b| std::cmp::Reverse(b.size_bytes));
    }
    Ok(groups)
}

#[derive(serde::Serialize)]
struct DeleteOutcome {
    path: String,
    freed_bytes: u64,
    archive_path: Option<String>,
    error: Option<String>,
}

#[tauri::command]
fn delete_command(
    paths: Vec<String>,
    mode: String,
    sizes: Vec<u64>,
) -> Result<Vec<DeleteOutcome>, String> {
    let delete_mode = match mode.as_str() {
        "trash" => DeleteMode::Trash,
        "archive" => DeleteMode::Archive,
        "permanent" => DeleteMode::Permanent,
        other => return Err(format!("unknown delete mode: {other}")),
    };
    if paths.len() != sizes.len() {
        return Err("paths and sizes must be the same length".into());
    }

    let mut results = Vec::with_capacity(paths.len());
    for (path_str, size) in paths.into_iter().zip(sizes) {
        let path = match validate_ipc_path(&path_str) {
            Ok(p) => p,
            Err(e) => {
                results.push(DeleteOutcome {
                    path: path_str,
                    freed_bytes: 0,
                    archive_path: None,
                    error: Some(e),
                });
                continue;
            }
        };
        let kind_slug = nodewipe_core::classify_path(&path)
            .map(|k| k.slug().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        match core_delete(&path, delete_mode, size) {
            Ok(res) => {
                let ts = now_unix();
                let _ = history_append(HistoryEntry {
                    deleted_at: ts,
                    date: format_date(ts),
                    path: path_str.clone(),
                    kind: kind_slug,
                    size_bytes: size,
                    mode: format!("{delete_mode:?}").to_lowercase(),
                });
                results.push(DeleteOutcome {
                    path: path_str,
                    freed_bytes: res.freed_bytes,
                    archive_path: res.archive_path.map(|p| p.display().to_string()),
                    error: None,
                });
            }
            Err(e) => results.push(DeleteOutcome {
                path: path_str,
                freed_bytes: 0,
                archive_path: None,
                error: Some(e.to_string()),
            }),
        }
    }
    Ok(results)
}

#[tauri::command(async)]
fn find_archives_command(root: String) -> Result<Vec<serde_json::Value>, String> {
    let root_path = validate_ipc_path(&root)?;
    let archives: Vec<serde_json::Value> = core_find_archives(&root_path)
        .into_iter()
        .map(|a| {
            serde_json::json!({
                "path": a.path.to_string_lossy().to_string(),
                "size_bytes": a.size_bytes,
                "is_snapshot": a.is_snapshot,
            })
        })
        .collect();
    Ok(archives)
}

#[tauri::command(async)]
fn restore_command(archive_path: String) -> Result<String, String> {
    let path = validate_ipc_path(&archive_path)?;
    core_restore(&path)
        .map(|restored| restored.display().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn home_dir_command() -> Result<String, String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| "could not determine home directory".to_string())
}

#[tauri::command(async)]
fn doctor_command(root: String) -> Result<serde_json::Value, String> {
    let root_path = validate_ipc_path(&root)?;
    let opts = ScanOptions {
        ignore_patterns: load_ignore_patterns(&root_path),
        root: root_path,
        ..Default::default()
    };
    let entries = core_scan(&opts).map_err(|e| e.to_string())?;
    let report = build_report(&entries);
    serde_json::to_value(report).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn stats_command() -> Result<serde_json::Value, String> {
    let entries = history_load().map_err(|e| e.to_string())?;
    Ok(build_stats_payload(&entries))
}

fn build_stats_payload(entries: &[HistoryEntry]) -> serde_json::Value {
    let total_bytes: u64 = entries.iter().map(|e| e.size_bytes).sum();
    let total_count = entries.len();
    let by_ecosystem = stats_by_ecosystem(entries);
    let daily = daily_totals(entries);
    let weekly = weekly_totals(entries);
    let monthly = monthly_totals(entries);
    let recent: Vec<&HistoryEntry> = entries.iter().rev().take(25).collect();
    serde_json::json!({
        "total_freed_bytes": total_bytes,
        "total_deletions": total_count,
        "by_ecosystem": by_ecosystem,
        "daily": daily,
        "weekly": weekly,
        "monthly": monthly,
        "recent": recent,
    })
}

#[tauri::command]
fn load_gui_prefs_command(state: tauri::State<'_, GuiAppState>) -> GuiPreferences {
    let prefs = prefs_file_path()
        .map(|p| load_prefs_from_path(&p))
        .unwrap_or_default();
    state
        .minimize_to_tray
        .store(prefs.minimize_to_tray, Ordering::Relaxed);
    prefs
}

#[tauri::command]
fn save_gui_prefs_command(
    prefs: GuiPreferences,
    state: tauri::State<'_, GuiAppState>,
) -> Result<GuiPreferences, String> {
    let clean = prefs.sanitized();
    state
        .minimize_to_tray
        .store(clean.minimize_to_tray, Ordering::Relaxed);
    if let Some(path) = prefs_file_path() {
        save_prefs_to_path(&path, &clean)?;
    }
    Ok(clean)
}

#[tauri::command]
fn daemon_status_command() -> serde_json::Value {
    let cfg = DaemonConfig::default();
    let status = check_daemon_status(&cfg);
    serde_json::to_value(status).unwrap_or_else(|_| serde_json::json!({ "is_alive": false }))
}

#[tauri::command]
fn daemon_toggle_command() -> Result<serde_json::Value, String> {
    let cfg = DaemonConfig::default();
    let status = check_daemon_status(&cfg);
    if status.is_alive {
        if let Some(pid) = read_pid(&cfg.pid_file) {
            let _ = terminate_process(pid);
            for _ in 0..20 {
                if !nodewipe_core::daemon::is_pid_alive(pid) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let _ = remove_pid(&cfg.pid_file);
            let _ = std::fs::remove_file(&cfg.status_file);
        }
    } else {
        // Locate CLI binary alongside GUI or on PATH
        let cli_bin = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("nodewipe")))
            .filter(|p| p.exists())
            .unwrap_or_else(|| PathBuf::from("nodewipe"));

        let watch_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let mut cmd = std::process::Command::new(cli_bin);
        cmd.args(["daemon", "start", "--watch-dir"])
            .arg(watch_dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        cmd.spawn()
            .map_err(|e| format!("failed to start nodewipe daemon: {e}"))?;
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    let updated = check_daemon_status(&cfg);
    serde_json::to_value(updated).map_err(|e| e.to_string())
}

fn show_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let initial_prefs = prefs_file_path()
        .map(|p| load_prefs_from_path(&p))
        .unwrap_or_default();
    let minimize_flag = Arc::new(AtomicBool::new(initial_prefs.minimize_to_tray));
    let minimize_for_window = Arc::clone(&minimize_flag);

    tauri::Builder::default()
        .manage(GuiAppState {
            minimize_to_tray: minimize_flag,
        })
        .setup(|app| {
            let open_item =
                MenuItem::with_id(app, "open_dashboard", "Open Dashboard", true, None::<&str>)?;
            let scan_item = MenuItem::with_id(
                app,
                "quick_scan",
                "Quick Scan Workspaces",
                true,
                None::<&str>,
            )?;
            let watcher_item =
                MenuItem::with_id(app, "toggle_watcher", "Toggle Watcher", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit nodewipe", true, None::<&str>)?;

            let menu = Menu::with_items(app, &[&open_item, &scan_item, &watcher_item, &quit_item])?;

            let mut tray_builder = TrayIconBuilder::with_id("nodewipe-tray")
                .menu(&menu)
                .tooltip("nodewipe - Workspace Storage Cleaner")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open_dashboard" => {
                        show_main_window(app);
                    }
                    "quick_scan" => {
                        show_main_window(app);
                        let _ = app.emit("tray-quick-scan", ());
                    }
                    "toggle_watcher" => {
                        let _ = daemon_toggle_command();
                        let _ = app.emit("tray-watcher-toggled", daemon_status_command());
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                });

            if let Some(icon) = app.default_window_icon().cloned() {
                tray_builder = tray_builder.icon(icon);
            }

            let _tray = tray_builder.build(app)?;
            Ok(())
        })
        .on_window_event(move |window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && minimize_for_window.load(Ordering::Relaxed)
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            scan_command,
            scan_grouped_command,
            delete_command,
            find_archives_command,
            restore_command,
            home_dir_command,
            doctor_command,
            stats_command,
            load_gui_prefs_command,
            save_gui_prefs_command,
            daemon_status_command,
            daemon_toggle_command,
        ])
        .run(tauri::generate_context!())
        .expect("error while running nodewipe GUI");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_ipc_path_rejects_empty_and_nul() {
        assert!(validate_ipc_path("").is_err());
        assert!(validate_ipc_path("   ").is_err());
        assert!(validate_ipc_path("/tmp/foo\0bar").is_err());
        assert_eq!(
            validate_ipc_path(" /tmp/workspace ").unwrap(),
            PathBuf::from("/tmp/workspace")
        );
    }

    #[test]
    fn test_gui_preferences_roundtrip_and_sanitization() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui_prefs.json");

        let dirty = GuiPreferences {
            theme: "invalid_theme".to_string(),
            accent: "xss_attempt".to_string(),
            minimize_to_tray: false,
        };
        let saved = save_prefs_to_path(&path, &dirty);
        assert!(saved.is_ok());

        let loaded = load_prefs_from_path(&path);
        assert_eq!(loaded.theme, "system");
        assert_eq!(loaded.accent, "cobalt");
        assert!(!loaded.minimize_to_tray);

        let dark = GuiPreferences {
            theme: "light".to_string(),
            accent: "emerald".to_string(),
            minimize_to_tray: true,
        };
        save_prefs_to_path(&path, &dark).unwrap();
        let reloaded = load_prefs_from_path(&path);
        assert_eq!(reloaded, dark);
    }

    #[test]
    fn test_stats_payload_includes_weekly_monthly_and_recent() {
        let entries = vec![
            HistoryEntry {
                deleted_at: 1_770_000_000,
                date: "2026-02-02".to_string(),
                path: "/proj/node_modules".to_string(),
                kind: "node_modules".to_string(),
                size_bytes: 1_048_576,
                mode: "trash".to_string(),
            },
            HistoryEntry {
                deleted_at: 1_772_500_000,
                date: "2026-03-03".to_string(),
                path: "/proj/target".to_string(),
                kind: "rust_target".to_string(),
                size_bytes: 2_097_152,
                mode: "archive".to_string(),
            },
        ];

        let payload = build_stats_payload(&entries);
        assert_eq!(payload["total_freed_bytes"], 3_145_728);
        assert_eq!(payload["total_deletions"], 2);
        assert_eq!(payload["weekly"].as_array().unwrap().len(), 2);
        assert_eq!(payload["monthly"].as_array().unwrap().len(), 2);
        assert_eq!(payload["recent"].as_array().unwrap().len(), 2);
    }
}
