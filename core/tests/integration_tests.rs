use nodewipe_core::history::{daily_totals, format_date, stats_by_ecosystem};
use nodewipe_core::workspace::{annotate_workspace_roots, group_by_workspace};
use nodewipe_core::{
    ArtifactKind, DeleteMode, HistoryEntry, ScanOptions, classify_path, delete,
    load_ignore_patterns, restore, scan,
};
use std::fs::{self, File};
use std::io::Write;
use tempfile::tempdir;

#[test]
fn test_classify_path_marker_rules() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // node_modules requires no marker
    let nm = root.join("node_modules");
    fs::create_dir(&nm).unwrap();
    assert_eq!(classify_path(&nm), Some(ArtifactKind::NodeModules));

    // target without Cargo.toml should be None
    let target = root.join("target");
    fs::create_dir(&target).unwrap();
    assert_eq!(classify_path(&target), None);

    // target with sibling Cargo.toml should be RustTarget
    File::create(root.join("Cargo.toml")).unwrap();
    assert_eq!(classify_path(&target), Some(ArtifactKind::RustTarget));

    // venv without pyvenv.cfg should be None
    let venv = root.join("venv");
    fs::create_dir(&venv).unwrap();
    assert_eq!(classify_path(&venv), None);

    // venv with pyvenv.cfg inside should be PythonVenv
    File::create(venv.join("pyvenv.cfg")).unwrap();
    assert_eq!(classify_path(&venv), Some(ArtifactKind::PythonVenv));

    // dist with pyproject.toml should be GenericDist
    let py_project = root.join("py_proj");
    fs::create_dir(&py_project).unwrap();
    File::create(py_project.join("pyproject.toml")).unwrap();
    let dist = py_project.join("dist");
    fs::create_dir(&dist).unwrap();
    assert_eq!(classify_path(&dist), Some(ArtifactKind::GenericDist));

    // HuggingFace hub cache
    let generic_hub = root.join("hub");
    fs::create_dir(&generic_hub).unwrap();
    assert_eq!(classify_path(&generic_hub), None);

    let hf_dir = root.join("huggingface");
    let hf_hub = hf_dir.join("hub");
    fs::create_dir_all(&hf_hub).unwrap();
    assert_eq!(classify_path(&hf_hub), Some(ArtifactKind::HuggingFaceCache));

    let custom_hub = root.join("custom_ai").join("hub");
    fs::create_dir_all(&custom_hub).unwrap();
    File::create(custom_hub.join("version.txt")).unwrap();
    assert_eq!(
        classify_path(&custom_hub),
        Some(ArtifactKind::HuggingFaceCache)
    );

    // Ollama models
    let generic_models = root.join("models");
    fs::create_dir(&generic_models).unwrap();
    assert_eq!(classify_path(&generic_models), None);

    let ollama_models = root.join(".ollama").join("models");
    fs::create_dir_all(&ollama_models).unwrap();
    assert_eq!(
        classify_path(&ollama_models),
        Some(ArtifactKind::OllamaModels)
    );

    // PyTorch cache
    let generic_torch = root.join("torch");
    fs::create_dir(&generic_torch).unwrap();
    assert_eq!(classify_path(&generic_torch), None);

    let cache_torch = root.join(".cache").join("torch");
    fs::create_dir_all(&cache_torch).unwrap();
    assert_eq!(classify_path(&cache_torch), Some(ArtifactKind::TorchCache));

    // Triton cache
    let generic_cache = root.join("cache");
    fs::create_dir(&generic_cache).unwrap();
    assert_eq!(classify_path(&generic_cache), None);

    let triton_cache = root.join(".triton").join("cache");
    fs::create_dir_all(&triton_cache).unwrap();
    assert_eq!(
        classify_path(&triton_cache),
        Some(ArtifactKind::TritonCache)
    );
}

#[test]
fn test_scan_and_ignore() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    let proj1 = root.join("proj1");
    fs::create_dir_all(proj1.join("node_modules")).unwrap();
    File::create(proj1.join("node_modules").join("pkg.js")).unwrap();

    let proj2 = root.join("ignored_proj");
    fs::create_dir_all(proj2.join("node_modules")).unwrap();
    File::create(proj2.join("node_modules").join("pkg.js")).unwrap();

    // Create .nodewipeignore in root
    let mut ignore_file = File::create(root.join(".nodewipeignore")).unwrap();
    writeln!(ignore_file, "ignored_proj").unwrap();

    let ignore_patterns = load_ignore_patterns(root);
    let opts = ScanOptions {
        root: root.to_path_buf(),
        ignore_patterns,
        ..Default::default()
    };

    let entries = scan(&opts).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, ArtifactKind::NodeModules);
    assert!(entries[0].path.starts_with(&proj1));
}

#[test]
fn test_is_ignored_outside_root_does_not_panic() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let ignore_patterns = load_ignore_patterns(root);

    // Testing a path outside the ignore root should safely return false instead of panicking
    let outside_path = std::path::Path::new("/some/random/outside/path");
    assert!(!nodewipe_core::ignorefile::is_ignored(
        outside_path,
        &ignore_patterns
    ));

    // Even if Gitignore has a relative root like ".", an absolute path outside should not panic
    let dot_ignore = nodewipe_core::load_ignore_patterns(std::path::Path::new("."));
    assert!(!nodewipe_core::ignorefile::is_ignored(
        outside_path,
        &dot_ignore
    ));

    // Empty ignore
    let empty = nodewipe_core::Gitignore::empty();
    assert!(!nodewipe_core::ignorefile::is_ignored(outside_path, &empty));
}

#[test]
fn test_workspace_detection() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // Cargo monorepo
    let cargo_root = root.join("cargo_mono");
    fs::create_dir_all(&cargo_root).unwrap();
    let mut cargo_toml = File::create(cargo_root.join("Cargo.toml")).unwrap();
    writeln!(cargo_toml, "[workspace]\nmembers = [\"app\"]").unwrap();

    let sub_app = cargo_root.join("app");
    fs::create_dir_all(sub_app.join("target")).unwrap();
    File::create(cargo_root.join("target").join("dummy")).ok();

    let mut entries = vec![nodewipe_core::Entry {
        path: sub_app.join("target"),
        size_bytes: 1024,
        kind: ArtifactKind::RustTarget,
        package_manager: None,
        last_modified: None,
        workspace_root: None,
    }];

    annotate_workspace_roots(&mut entries, root);
    assert_eq!(entries[0].workspace_root, Some(cargo_root.clone()));

    let groups = group_by_workspace(entries);
    assert!(groups.contains_key(&Some(cargo_root)));
}

#[test]
fn test_archive_and_restore_cycle() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    let project = root.join("my_app");
    let nm = project.join("node_modules");
    fs::create_dir_all(&nm).unwrap();
    let test_file = nm.join("hello.txt");
    fs::write(&test_file, b"content").unwrap();

    // Delete with Archive mode
    let res = delete(&nm, DeleteMode::Archive, 7).unwrap();
    assert!(!nm.exists());
    let archive_path = res.archive_path.expect("archive_path should be set");
    assert!(archive_path.exists());

    // Restore from archive
    let restored = restore(&archive_path).unwrap();
    assert_eq!(restored, nm);
    assert!(nm.exists());
    assert_eq!(fs::read(nm.join("hello.txt")).unwrap(), b"content");
}

#[test]
fn test_history_stats_and_formatting() {
    assert_eq!(format_date(1700000000), "2023-11-14");

    let entries = vec![
        HistoryEntry {
            deleted_at: 1700000000,
            date: "2023-11-14".into(),
            path: "/path/to/target".into(),
            kind: "rust_target".into(),
            size_bytes: 500,
            mode: "trash".into(),
        },
        HistoryEntry {
            deleted_at: 1700001000,
            date: "2023-11-14".into(),
            path: "/path/to/venv".into(),
            kind: "venv".into(),
            size_bytes: 200,
            mode: "archive".into(),
        },
        HistoryEntry {
            deleted_at: 1700086400,
            date: "2023-11-15".into(),
            path: "/path/to/target2".into(),
            kind: "rust_target".into(),
            size_bytes: 300,
            mode: "permanent".into(),
        },
    ];

    let eco_stats = stats_by_ecosystem(&entries);
    assert_eq!(eco_stats.len(), 2);
    assert_eq!(eco_stats[0].kind, "rust_target");
    assert_eq!(eco_stats[0].total_bytes, 800);
    assert_eq!(eco_stats[0].count, 2);

    let days = daily_totals(&entries);
    assert_eq!(days.len(), 2);
    assert_eq!(days[0].date, "2023-11-14");
    assert_eq!(days[0].bytes_freed, 700);
}

#[test]
fn test_restore_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // Craft raw tar header containing '..' path traversal
    let evil_archive = root.join("evil-node_modules-backup.tar.gz");
    let file = File::create(&evil_archive).unwrap();
    let mut enc = flate2::write::GzEncoder::new(file, flate2::Compression::fast());

    // A standard POSIX ustar header is 512 bytes
    let mut header_bytes = [0u8; 512];
    let path_str = b"node_modules/../../evil.txt";
    header_bytes[..path_str.len()].copy_from_slice(path_str);
    // Mode
    header_bytes[100..107].copy_from_slice(b"0000644");
    // Size (5 bytes in octal)
    header_bytes[124..135].copy_from_slice(b"00000000005");
    // Checksum calculation
    // Fill checksum field with spaces
    header_bytes[148..156].copy_from_slice(b"        ");
    let cksum: u32 = header_bytes.iter().map(|&b| b as u32).sum();
    let cksum_octal = format!("{:06o}\0 ", cksum);
    header_bytes[148..156].copy_from_slice(cksum_octal.as_bytes());

    enc.write_all(&header_bytes).unwrap();
    // Payload (5 bytes padded to 512 bytes)
    let mut payload = [0u8; 512];
    payload[..5].copy_from_slice(b"pwned");
    enc.write_all(&payload).unwrap();
    // Tar end markers (1024 zeroes)
    enc.write_all(&[0u8; 1024]).unwrap();
    enc.finish().unwrap();

    let err = restore(&evil_archive).unwrap_err();
    assert!(err.to_string().contains("parent directory component '..'"));
}

#[cfg(unix)]
#[test]
fn test_symlink_delete_does_not_wipe_target() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let root = dir.path();

    let real_dir = root.join("important_real_dir");
    fs::create_dir(&real_dir).unwrap();
    let canary = real_dir.join("precious_data.txt");
    fs::write(&canary, b"do not delete").unwrap();

    let fake_nm = root.join("node_modules");
    symlink(&real_dir, &fake_nm).unwrap();

    // Delete fake_nm with Permanent mode
    let res = delete(&fake_nm, DeleteMode::Permanent, 0);
    assert!(res.is_ok());

    // Symlink should be gone, but the real target and files MUST remain intact!
    assert!(!fake_nm.exists());
    assert!(real_dir.exists());
    assert!(canary.exists());
    assert_eq!(fs::read_to_string(canary).unwrap(), "do not delete");
}

#[test]
fn test_delete_refuses_root_or_non_artifact() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // Trying to delete a normal directory that isn't an artifact should fail
    let regular_dir = root.join("my_documents");
    fs::create_dir(&regular_dir).unwrap();
    let res = delete(&regular_dir, DeleteMode::Permanent, 0);
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains(
        "refusing to delete a directory that doesn't look like a known disposable artifact"
    ));

    // Root directory check
    let res_root = delete(std::path::Path::new("/"), DeleteMode::Permanent, 0);
    assert!(res_root.is_err());
}

#[test]
fn test_ai_ml_cache_scan_and_delete() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // Setup HF cache
    let hf_hub = root.join("huggingface").join("hub");
    fs::create_dir_all(&hf_hub).unwrap();
    fs::write(hf_hub.join("model.safetensors"), b"fake_weights").unwrap();

    // Setup Ollama models
    let ollama_models = root.join(".ollama").join("models");
    fs::create_dir_all(&ollama_models).unwrap();
    fs::write(ollama_models.join("sha256_blob"), b"fake_blob").unwrap();

    // Setup PyTorch cache
    let torch_cache = root.join(".cache").join("torch");
    fs::create_dir_all(&torch_cache).unwrap();
    fs::write(torch_cache.join("checkpoint.pt"), b"fake_checkpoint").unwrap();

    // Setup Triton cache
    let triton_cache = root.join(".triton").join("cache");
    fs::create_dir_all(&triton_cache).unwrap();
    fs::write(triton_cache.join("kernel.ptx"), b"fake_kernel").unwrap();

    // Scan all
    let opts = ScanOptions {
        root: root.to_path_buf(),
        ..Default::default()
    };
    let entries = scan(&opts).unwrap();
    assert_eq!(entries.len(), 4);

    let kinds: Vec<ArtifactKind> = entries.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&ArtifactKind::HuggingFaceCache));
    assert!(kinds.contains(&ArtifactKind::OllamaModels));
    assert!(kinds.contains(&ArtifactKind::TorchCache));
    assert!(kinds.contains(&ArtifactKind::TritonCache));

    // Test exclude_kinds
    let opts_filtered = ScanOptions {
        root: root.to_path_buf(),
        exclude_kinds: vec![ArtifactKind::HuggingFaceCache, ArtifactKind::OllamaModels],
        ..Default::default()
    };
    let filtered_entries = scan(&opts_filtered).unwrap();
    assert_eq!(filtered_entries.len(), 2);
    let filtered_kinds: Vec<ArtifactKind> = filtered_entries.iter().map(|e| e.kind).collect();
    assert!(!filtered_kinds.contains(&ArtifactKind::HuggingFaceCache));
    assert!(!filtered_kinds.contains(&ArtifactKind::OllamaModels));
    assert!(filtered_kinds.contains(&ArtifactKind::TorchCache));
    assert!(filtered_kinds.contains(&ArtifactKind::TritonCache));

    // Test safe deletion of one of the AI caches
    let delete_res = delete(&triton_cache, DeleteMode::Permanent, 0);
    assert!(delete_res.is_ok());
    assert!(!triton_cache.exists());
}
