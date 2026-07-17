use nodewipe_core::{annotate_workspace_roots, delete as core_delete, group_by_workspace, restore as core_restore, scan as core_scan, DeleteMode, Entry, ScanOptions, WorkspaceGroup};
use std::path::PathBuf;
use walkdir::WalkDir;

#[tauri::command]
fn scan_command(root: String) -> Result<Vec<Entry>, String> {
    let opts = ScanOptions { root: PathBuf::from(root), ..Default::default() };
    let mut entries = core_scan(&opts).map_err(|e| e.to_string())?;
    annotate_workspace_roots(&mut entries, &opts.root);
    entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    Ok(entries)
}

#[tauri::command]
fn scan_grouped_command(root: String) -> Result<Vec<WorkspaceGroup>, String> {
    let opts = ScanOptions { root: PathBuf::from(root), ..Default::default() };
    let mut entries = core_scan(&opts).map_err(|e| e.to_string())?;
    annotate_workspace_roots(&mut entries, &opts.root);
    let groups = group_by_workspace(entries);
    let mut groups: Vec<WorkspaceGroup> = groups.into_values().collect();
    groups.sort_by(|a, b| b.total_size_bytes.cmp(&a.total_size_bytes));
    for g in &mut groups { g.entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes)); }
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
fn delete_command(paths: Vec<String>, mode: String, sizes: Vec<u64>) -> Result<Vec<DeleteOutcome>, String> {
    let delete_mode = match mode.as_str() {
        "trash" => DeleteMode::Trash,
        "archive" => DeleteMode::Archive,
        "permanent" => DeleteMode::Permanent,
        other => return Err(format!("unknown delete mode: {other}")),
    };
    if paths.len() != sizes.len() { return Err("paths and sizes must be the same length".into()); }

    let mut results = Vec::with_capacity(paths.len());
    for (path_str, size) in paths.into_iter().zip(sizes) {
        let path = PathBuf::from(&path_str);
        match core_delete(&path, delete_mode, size) {
            Ok(res) => results.push(DeleteOutcome {
                path: path_str,
                freed_bytes: res.freed_bytes,
                archive_path: res.archive_path.map(|p| p.display().to_string()),
                error: None,
            }),
            Err(e) => results.push(DeleteOutcome { path: path_str, freed_bytes: 0, archive_path: None, error: Some(e.to_string()) }),
        }
    }
    Ok(results)
}

/// Finds all nodewipe backup archives (*-backup.tar.gz) in the given root.
#[tauri::command]
fn find_archives_command(root: String) -> Result<Vec<serde_json::Value>, String> {
    let archives: Vec<serde_json::Value> = WalkDir::new(&root)
        .max_depth(6)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.file_name().to_str().map(|n| n.ends_with("-backup.tar.gz")).unwrap_or(false)
        })
        .map(|e| {
            let path = e.path().to_string_lossy().to_string();
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            serde_json::json!({ "path": path, "size_bytes": size })
        })
        .collect();
    Ok(archives)
}

/// Restores a backup archive to its original location.
#[tauri::command]
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            scan_command,
            scan_grouped_command,
            delete_command,
            find_archives_command,
            restore_command,
            home_dir_command
        ])
        .run(tauri::generate_context!())
        .expect("error while running nodewipe GUI");
}
