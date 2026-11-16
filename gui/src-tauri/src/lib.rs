use nodewipe_core::history::{daily_totals, format_date, now_unix, stats_by_ecosystem};
use nodewipe_core::{
    DeleteMode, Entry, HistoryEntry, ScanOptions, WorkspaceGroup, annotate_workspace_roots,
    delete as core_delete, doctor::build_report, find_archives as core_find_archives,
    group_by_workspace, history_append, history_load, restore as core_restore, scan as core_scan,
};
use std::path::PathBuf;

#[tauri::command(async)]
fn scan_command(root: String) -> Result<Vec<Entry>, String> {
    let opts = ScanOptions {
        root: PathBuf::from(root),
        ..Default::default()
    };
    let mut entries = core_scan(&opts).map_err(|e| e.to_string())?;
    annotate_workspace_roots(&mut entries, &opts.root);
    entries.sort_by_key(|b| std::cmp::Reverse(b.size_bytes));
    Ok(entries)
}

#[tauri::command(async)]
fn scan_grouped_command(root: String) -> Result<Vec<WorkspaceGroup>, String> {
    let opts = ScanOptions {
        root: PathBuf::from(root),
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
        let path = PathBuf::from(&path_str);
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
    let archives: Vec<serde_json::Value> = core_find_archives(&PathBuf::from(root))
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
    let path = PathBuf::from(&archive_path);
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
    let opts = ScanOptions {
        root: PathBuf::from(root),
        ..Default::default()
    };
    let entries = core_scan(&opts).map_err(|e| e.to_string())?;
    let report = build_report(&entries);
    serde_json::to_value(report).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn stats_command() -> Result<serde_json::Value, String> {
    let entries = history_load().map_err(|e| e.to_string())?;
    let total_bytes: u64 = entries.iter().map(|e| e.size_bytes).sum();
    let total_count = entries.len();
    let by_ecosystem = stats_by_ecosystem(&entries);
    let daily = daily_totals(&entries);
    Ok(serde_json::json!({
        "total_freed_bytes": total_bytes,
        "total_deletions": total_count,
        "by_ecosystem": by_ecosystem,
        "daily": daily,
    }))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            scan_command,
            scan_grouped_command,
            delete_command,
            find_archives_command,
            restore_command,
            home_dir_command,
            doctor_command,
            stats_command,
        ])
        .run(tauri::generate_context!())
        .expect("error while running nodewipe GUI");
}
