use nodewipe_core::{annotate_workspace_roots, delete as core_delete, scan as core_scan, DeleteMode, Entry, ScanOptions};
use std::path::PathBuf;

/// Scans `root` and returns every discovered `node_modules` directory,
/// annotated with its workspace/monorepo root (if any).
///
/// This is a thin wrapper: all scanning/exclude-list logic lives in
/// `nodewipe-core`, so the GUI and CLI can never drift out of sync.
#[tauri::command]
fn scan_command(root: String) -> Result<Vec<Entry>, String> {
    let opts = ScanOptions {
        root: PathBuf::from(root),
        ..Default::default()
    };

    let mut entries = core_scan(&opts).map_err(|e| e.to_string())?;
    annotate_workspace_roots(&mut entries, &opts.root);
    entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    Ok(entries)
}

#[derive(serde::Serialize)]
struct DeleteOutcome {
    path: String,
    freed_bytes: u64,
    archive_path: Option<String>,
    error: Option<String>,
}

/// Deletes each path in `paths` using `mode` ("trash" | "archive" | "permanent").
/// Continues past individual failures and reports them per-path rather than
/// aborting the whole batch, so one locked file doesn't block the rest.
#[tauri::command]
fn delete_command(paths: Vec<String>, mode: String, sizes: Vec<u64>) -> Result<Vec<DeleteOutcome>, String> {
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
        match core_delete(&path, delete_mode, size) {
            Ok(res) => results.push(DeleteOutcome {
                path: path_str,
                freed_bytes: res.freed_bytes,
                archive_path: res.archive_path.map(|p| p.display().to_string()),
                error: None,
            }),
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

#[tauri::command]
fn home_dir_command() -> Result<String, String> {
    dirs_home().ok_or_else(|| "could not determine home directory".to_string())
}

// Minimal home-dir lookup without pulling in the `dirs` crate just for this.
fn dirs_home() -> Option<String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|s| s.to_string_lossy().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![scan_command, delete_command, home_dir_command])
        .run(tauri::generate_context!())
        .expect("error while running nodewipe GUI");
}
