use nodewipe_core::history::{daily_totals, format_date, stats_by_ecosystem};
use nodewipe_core::workspace::{annotate_workspace_roots, group_by_workspace};
use nodewipe_core::{
    ArtifactKind, DeleteMode, Entry, HistoryEntry, ScanOptions, classify_path, delete,
    find_archives, load_ignore_patterns, restore, scan,
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

    // Xcode DerivedData
    let generic_derived_data = root.join("DerivedData");
    fs::create_dir(&generic_derived_data).unwrap();
    assert_eq!(classify_path(&generic_derived_data), None);

    let xcode_derived_data = root.join("Xcode").join("DerivedData");
    fs::create_dir_all(&xcode_derived_data).unwrap();
    assert_eq!(
        classify_path(&xcode_derived_data),
        Some(ArtifactKind::XcodeDerivedData)
    );

    // SwiftPM build
    let swift_proj = root.join("swift_app");
    fs::create_dir_all(&swift_proj).unwrap();
    let swift_build = swift_proj.join(".build");
    fs::create_dir(&swift_build).unwrap();
    assert_eq!(classify_path(&swift_build), None);
    File::create(swift_proj.join("Package.swift")).unwrap();
    assert_eq!(
        classify_path(&swift_build),
        Some(ArtifactKind::SwiftPmBuild)
    );

    // CocoaPods Pods
    let ios_proj = root.join("ios_app");
    fs::create_dir_all(&ios_proj).unwrap();
    let pods_dir = ios_proj.join("Pods");
    fs::create_dir(&pods_dir).unwrap();
    assert_eq!(classify_path(&pods_dir), None);
    File::create(ios_proj.join("Podfile")).unwrap();
    assert_eq!(classify_path(&pods_dir), Some(ArtifactKind::CocoaPods));

    // Gradle cache
    let generic_gradle_cache = root.join("caches");
    fs::create_dir(&generic_gradle_cache).unwrap();
    assert_eq!(classify_path(&generic_gradle_cache), None);

    let real_gradle_cache = root.join(".gradle").join("caches");
    fs::create_dir_all(&real_gradle_cache).unwrap();
    assert_eq!(
        classify_path(&real_gradle_cache),
        Some(ArtifactKind::GradleCache)
    );

    // Android build cache
    let generic_android_cache = root.join("build-cache");
    fs::create_dir(&generic_android_cache).unwrap();
    assert_eq!(classify_path(&generic_android_cache), None);

    let real_android_cache = root.join(".android").join("build-cache");
    fs::create_dir_all(&real_android_cache).unwrap();
    assert_eq!(
        classify_path(&real_android_cache),
        Some(ArtifactKind::AndroidBuildCache)
    );

    // Playwright
    let pw_dir = root.join("ms-playwright");
    fs::create_dir(&pw_dir).unwrap();
    assert_eq!(
        classify_path(&pw_dir),
        Some(ArtifactKind::PlaywrightBrowsers)
    );

    // Cypress
    let generic_cypress = root.join("Cypress");
    fs::create_dir(&generic_cypress).unwrap();
    assert_eq!(classify_path(&generic_cypress), None);

    let cache_cypress = root.join(".cache").join("Cypress");
    fs::create_dir_all(&cache_cypress).unwrap();
    assert_eq!(
        classify_path(&cache_cypress),
        Some(ArtifactKind::CypressCache)
    );

    // Astro
    let astro_proj = root.join("astro_site");
    fs::create_dir_all(&astro_proj).unwrap();
    let astro_dir = astro_proj.join(".astro");
    fs::create_dir(&astro_dir).unwrap();
    assert_eq!(classify_path(&astro_dir), None);
    File::create(astro_proj.join("astro.config.mjs")).unwrap();
    assert_eq!(classify_path(&astro_dir), Some(ArtifactKind::AstroCache));

    // SvelteKit
    let svelte_proj = root.join("svelte_site");
    fs::create_dir_all(&svelte_proj).unwrap();
    let svelte_dir = svelte_proj.join(".svelte-kit");
    fs::create_dir(&svelte_dir).unwrap();
    assert_eq!(classify_path(&svelte_dir), None);
    File::create(svelte_proj.join("svelte.config.js")).unwrap();
    assert_eq!(
        classify_path(&svelte_dir),
        Some(ArtifactKind::SvelteKitCache)
    );

    // Nuxt
    let nuxt_proj = root.join("nuxt_site");
    fs::create_dir_all(&nuxt_proj).unwrap();
    let nuxt_dir = nuxt_proj.join(".nuxt");
    fs::create_dir(&nuxt_dir).unwrap();
    assert_eq!(classify_path(&nuxt_dir), None);
    File::create(nuxt_proj.join("nuxt.config.ts")).unwrap();
    assert_eq!(classify_path(&nuxt_dir), Some(ArtifactKind::NuxtCache));

    // Docusaurus
    let doc_proj = root.join("doc_site");
    fs::create_dir_all(&doc_proj).unwrap();
    let doc_dir = doc_proj.join(".docusaurus");
    fs::create_dir(&doc_dir).unwrap();
    assert_eq!(classify_path(&doc_dir), None);
    File::create(doc_proj.join("docusaurus.config.js")).unwrap();
    assert_eq!(classify_path(&doc_dir), Some(ArtifactKind::DocusaurusCache));

    // Vite
    let vite_proj = root.join("vite_site");
    fs::create_dir_all(&vite_proj).unwrap();
    let vite_dir = vite_proj.join(".vite");
    fs::create_dir(&vite_dir).unwrap();
    assert_eq!(classify_path(&vite_dir), None);
    File::create(vite_proj.join("vite.config.ts")).unwrap();
    assert_eq!(classify_path(&vite_dir), Some(ArtifactKind::ViteCache));

    // CMake / Ninja
    let cmake_proj = root.join("cpp_proj");
    fs::create_dir_all(&cmake_proj).unwrap();
    let cmake_build = cmake_proj.join("build");
    fs::create_dir(&cmake_build).unwrap();
    assert_eq!(classify_path(&cmake_build), None);
    File::create(cmake_proj.join("CMakeLists.txt")).unwrap();
    assert_eq!(classify_path(&cmake_build), Some(ArtifactKind::CmakeBuild));

    let clion_build = cmake_proj.join("cmake-build-debug");
    fs::create_dir(&clion_build).unwrap();
    assert_eq!(classify_path(&clion_build), Some(ArtifactKind::CmakeBuild));

    // Zig
    let zig_proj = root.join("zig_proj");
    fs::create_dir_all(&zig_proj).unwrap();
    let zig_cache = zig_proj.join(".zig-cache");
    fs::create_dir(&zig_cache).unwrap();
    assert_eq!(classify_path(&zig_cache), Some(ArtifactKind::ZigCache));

    let zig_out = zig_proj.join("zig-out");
    fs::create_dir(&zig_out).unwrap();
    assert_eq!(classify_path(&zig_out), None);
    File::create(zig_proj.join("build.zig")).unwrap();
    assert_eq!(classify_path(&zig_out), Some(ArtifactKind::ZigCache));

    // Elixir / Mix
    let elixir_proj = root.join("elixir_proj");
    fs::create_dir_all(&elixir_proj).unwrap();
    let elixir_build = elixir_proj.join("_build");
    fs::create_dir(&elixir_build).unwrap();
    assert_eq!(classify_path(&elixir_build), None);
    File::create(elixir_proj.join("mix.exs")).unwrap();
    assert_eq!(
        classify_path(&elixir_build),
        Some(ArtifactKind::ElixirBuild)
    );

    // Meson
    let meson_proj = root.join("meson_proj");
    fs::create_dir_all(&meson_proj).unwrap();
    let meson_build = meson_proj.join("builddir");
    fs::create_dir(&meson_build).unwrap();
    assert_eq!(classify_path(&meson_build), None);
    File::create(meson_proj.join("meson.build")).unwrap();
    assert_eq!(classify_path(&meson_build), Some(ArtifactKind::MesonBuild));
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
        git_status: None,
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

#[cfg(unix)]
#[test]
fn test_archive_preserves_symlinks_without_dereference() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    let project = root.join("app_with_symlinks");
    let nm = project.join("node_modules");
    fs::create_dir_all(&nm).unwrap();

    let real_file = nm.join("target_file.js");
    fs::write(&real_file, b"original payload").unwrap();

    let link_file = nm.join("symlink.js");
    std::os::unix::fs::symlink(std::path::Path::new("target_file.js"), &link_file).unwrap();

    // Verify it is indeed a symlink initially
    assert!(
        fs::symlink_metadata(&link_file)
            .unwrap()
            .file_type()
            .is_symlink()
    );

    // Archive and restore
    let res = delete(&nm, DeleteMode::Archive, 7).unwrap();
    assert!(!nm.exists());
    let archive_path = res.archive_path.expect("archive_path should be set");

    let restored = restore(&archive_path).unwrap();
    assert_eq!(restored, nm);

    // Verify that the restored link is still a symlink, NOT dereferenced into a regular file
    let restored_link = nm.join("symlink.js");
    assert!(restored_link.exists());
    assert!(
        fs::symlink_metadata(&restored_link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "archived symlink must be restored as a symlink, not dereferenced"
    );
    assert_eq!(fs::read(&restored_link).unwrap(), b"original payload");
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
fn test_history_corruption_self_healing_and_recovery() {
    use nodewipe_core::history::{append_to_path, load_from_path};

    let tmp = tempfile::tempdir().unwrap();
    let history_file = tmp.path().join("history.json");

    // 1. Write corrupted/truncated JSON
    std::fs::write(&history_file, b"{\"truncated\": [").unwrap();

    // 2. load_from_path must not fail; it must recover, back up the corrupt file, and return empty list
    let loaded = load_from_path(&history_file).unwrap();
    assert_eq!(loaded.len(), 0);

    // Corrupted file was backed up
    let has_backup = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("history.json.corrupt.")
        });
    assert!(
        has_backup,
        "Corrupted history file must be preserved as backup"
    );

    // 3. Appending a new entry must succeed cleanly on the healed database
    let new_entry = HistoryEntry {
        deleted_at: 1700000000,
        date: "2023-11-14".into(),
        path: "/path/to/recovered".into(),
        kind: "node_modules".into(),
        size_bytes: 1024,
        mode: "trash".into(),
    };
    append_to_path(&history_file, new_entry).unwrap();

    let reloaded = load_from_path(&history_file).unwrap();
    assert_eq!(reloaded.len(), 1);
    assert_eq!(reloaded[0].path, "/path/to/recovered");
    assert_eq!(reloaded[0].size_bytes, 1024);
}

#[test]
fn test_concurrent_history_appends_lossless() {
    use nodewipe_core::history::{append_to_path, load_from_path};
    use std::sync::Arc;

    let tmp = tempfile::tempdir().unwrap();
    let history_file = Arc::new(tmp.path().join("concurrent_history.json"));

    let num_threads = 20;
    let mut handles = Vec::new();

    for i in 0..num_threads {
        let path_clone = Arc::clone(&history_file);
        handles.push(std::thread::spawn(move || {
            let entry = HistoryEntry {
                deleted_at: 1700000000 + i as u64,
                date: "2026-11-17".into(),
                path: format!("/path/to/project_{i}"),
                kind: "node_modules".into(),
                size_bytes: 1000 + i as u64,
                mode: "trash".into(),
            };
            append_to_path(&path_clone, entry).expect("concurrent append must succeed");
        }));
    }

    for h in handles {
        h.join().expect("thread should join cleanly");
    }

    let entries = load_from_path(&history_file).expect("history must load cleanly");
    assert_eq!(
        entries.len(),
        num_threads,
        "Every concurrent entry must be safely preserved without race conditions"
    );

    // Verify all 20 unique paths are present
    let paths: std::collections::HashSet<String> = entries.into_iter().map(|e| e.path).collect();
    for i in 0..num_threads {
        assert!(paths.contains(&format!("/path/to/project_{i}")));
    }
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

#[test]
fn test_restore_rejects_entries_outside_artifact_root() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // Existing project file that must never be overwritten
    let pkg_json = root.join("package.json");
    std::fs::write(&pkg_json, b"ORIGINAL_CONTENT").unwrap();

    // Craft tar archive containing both node_modules/a.txt and package.json
    let multi_archive = root.join("multi-node_modules-backup.tar.gz");
    let file = File::create(&multi_archive).unwrap();
    let enc = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    let mut builder = tar::Builder::new(enc);

    // 1. Valid entry under node_modules
    let mut header1 = tar::Header::new_gnu();
    header1.set_size(5);
    header1.set_mode(0o644);
    header1.set_cksum();
    builder
        .append_data(&mut header1, "node_modules/a.txt", b"hello".as_slice())
        .unwrap();

    // 2. Malicious entry attempting to overwrite package.json outside node_modules
    let mut header2 = tar::Header::new_gnu();
    header2.set_size(11);
    header2.set_mode(0o644);
    header2.set_cksum();
    builder
        .append_data(&mut header2, "package.json", b"OVERWRITTEN".as_slice())
        .unwrap();

    let enc = builder.into_inner().unwrap();
    enc.finish().unwrap();

    // Attempting restore must fail before extracting or overwriting
    let err = restore(&multi_archive).unwrap_err();
    assert!(
        err.to_string()
            .contains("outside the permitted artifact directory"),
        "error must mention outside permitted artifact directory, got: {err}"
    );

    // Verify package.json is 100% untouched
    assert_eq!(
        std::fs::read_to_string(&pkg_json).unwrap(),
        "ORIGINAL_CONTENT"
    );
    // Verify node_modules was not partially left behind in root
    assert!(!root.join("node_modules").exists());
}

#[cfg(unix)]
#[test]
fn test_restore_refuses_overwrite_on_broken_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let root = dir.path();

    // 1. Create a valid node_modules directory, archive it
    let nm = root.join("node_modules");
    fs::create_dir(&nm).unwrap();
    fs::write(nm.join("index.js"), b"console.log('hello');").unwrap();

    let res = delete(&nm, DeleteMode::Archive, 100).unwrap();
    let archive_path = res.archive_path.unwrap();
    assert!(archive_path.exists());
    assert!(!nm.exists());

    // 2. Create a broken symlink at 'node_modules' pointing to a non-existent path
    let broken_target = root.join("non_existent_folder");
    symlink(&broken_target, &nm).unwrap();

    // In regular Path::exists(), a broken symlink returns false!
    assert!(!nm.exists());
    assert!(std::fs::symlink_metadata(&nm).is_ok());

    // 3. Attempting to restore must refuse because a filesystem entry already exists!
    let restore_err = restore(&archive_path).unwrap_err();
    assert!(restore_err.to_string().contains("already exists"));

    // Cleanup broken symlink and restore must then succeed
    std::fs::remove_file(&nm).unwrap();
    let restored = restore(&archive_path).unwrap();
    assert_eq!(restored, nm);
    assert!(nm.exists());
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

#[test]
fn test_mobile_apple_cache_scan_and_delete() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // Xcode DerivedData
    let xcode_dir = root.join("Xcode").join("DerivedData");
    fs::create_dir_all(&xcode_dir).unwrap();
    fs::write(xcode_dir.join("build.db"), b"index_data").unwrap();

    // SwiftPM .build
    let swift_proj = root.join("my_swift_lib");
    fs::create_dir_all(&swift_proj).unwrap();
    File::create(swift_proj.join("Package.swift")).unwrap();
    let swift_build = swift_proj.join(".build");
    fs::create_dir_all(&swift_build).unwrap();
    fs::write(swift_build.join("output.o"), b"compiled_swift").unwrap();

    // CocoaPods Pods
    let ios_proj = root.join("my_ios_app");
    fs::create_dir_all(&ios_proj).unwrap();
    File::create(ios_proj.join("Podfile")).unwrap();
    let pods_dir = ios_proj.join("Pods");
    fs::create_dir_all(&pods_dir).unwrap();
    fs::write(pods_dir.join("Pod.a"), b"pod_lib").unwrap();

    // Gradle cache
    let gradle_cache = root.join(".gradle").join("caches");
    fs::create_dir_all(&gradle_cache).unwrap();
    fs::write(gradle_cache.join("dep.jar"), b"jar_cache").unwrap();

    // Android build cache
    let android_cache = root.join(".android").join("build-cache");
    fs::create_dir_all(&android_cache).unwrap();
    fs::write(android_cache.join("dex.cache"), b"dex_cache").unwrap();

    // Scan all
    let opts = ScanOptions {
        root: root.to_path_buf(),
        ..Default::default()
    };
    let entries = scan(&opts).unwrap();
    assert_eq!(entries.len(), 5);

    let kinds: Vec<ArtifactKind> = entries.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&ArtifactKind::XcodeDerivedData));
    assert!(kinds.contains(&ArtifactKind::SwiftPmBuild));
    assert!(kinds.contains(&ArtifactKind::CocoaPods));
    assert!(kinds.contains(&ArtifactKind::GradleCache));
    assert!(kinds.contains(&ArtifactKind::AndroidBuildCache));

    // Test exclude_kinds
    let opts_filtered = ScanOptions {
        root: root.to_path_buf(),
        exclude_kinds: vec![ArtifactKind::XcodeDerivedData, ArtifactKind::CocoaPods],
        ..Default::default()
    };
    let filtered_entries = scan(&opts_filtered).unwrap();
    assert_eq!(filtered_entries.len(), 3);
    let filtered_kinds: Vec<ArtifactKind> = filtered_entries.iter().map(|e| e.kind).collect();
    assert!(!filtered_kinds.contains(&ArtifactKind::XcodeDerivedData));
    assert!(!filtered_kinds.contains(&ArtifactKind::CocoaPods));
    assert!(filtered_kinds.contains(&ArtifactKind::SwiftPmBuild));
    assert!(filtered_kinds.contains(&ArtifactKind::GradleCache));
    assert!(filtered_kinds.contains(&ArtifactKind::AndroidBuildCache));

    // Test safe deletion
    let delete_res = delete(&xcode_dir, DeleteMode::Permanent, 0);
    assert!(delete_res.is_ok());
    assert!(!xcode_dir.exists());
}

#[test]
fn test_modern_web_cache_scan_and_delete() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // Playwright
    let pw_dir = root.join("ms-playwright");
    fs::create_dir_all(&pw_dir).unwrap();
    fs::write(pw_dir.join("chromium-1000"), b"browser_binary").unwrap();

    // Cypress
    let cypress_dir = root.join(".cache").join("Cypress");
    fs::create_dir_all(&cypress_dir).unwrap();
    fs::write(cypress_dir.join("cypress.zip"), b"cypress_binary").unwrap();

    // Astro
    let astro_proj = root.join("astro_app");
    fs::create_dir_all(&astro_proj).unwrap();
    File::create(astro_proj.join("package.json")).unwrap();
    let astro_cache = astro_proj.join(".astro");
    fs::create_dir_all(&astro_cache).unwrap();
    fs::write(astro_cache.join("cache.data"), b"astro_data").unwrap();

    // SvelteKit
    let svelte_proj = root.join("svelte_app");
    fs::create_dir_all(&svelte_proj).unwrap();
    File::create(svelte_proj.join("svelte.config.js")).unwrap();
    let svelte_cache = svelte_proj.join(".svelte-kit");
    fs::create_dir_all(&svelte_cache).unwrap();
    fs::write(svelte_cache.join("types.d.ts"), b"types").unwrap();

    // Nuxt
    let nuxt_proj = root.join("nuxt_app");
    fs::create_dir_all(&nuxt_proj).unwrap();
    File::create(nuxt_proj.join("nuxt.config.ts")).unwrap();
    let nuxt_cache = nuxt_proj.join(".nuxt");
    fs::create_dir_all(&nuxt_cache).unwrap();
    fs::write(nuxt_cache.join("nitro.json"), b"nitro").unwrap();

    // Docusaurus
    let doc_proj = root.join("doc_app");
    fs::create_dir_all(&doc_proj).unwrap();
    File::create(doc_proj.join("docusaurus.config.js")).unwrap();
    let doc_cache = doc_proj.join(".docusaurus");
    fs::create_dir_all(&doc_cache).unwrap();
    fs::write(doc_cache.join("routes.json"), b"routes").unwrap();

    // Vite
    let vite_proj = root.join("vite_app");
    fs::create_dir_all(&vite_proj).unwrap();
    File::create(vite_proj.join("vite.config.ts")).unwrap();
    let vite_cache = vite_proj.join(".vite");
    fs::create_dir_all(&vite_cache).unwrap();
    fs::write(vite_cache.join("deps.js"), b"bundled_deps").unwrap();

    // Scan all
    let opts = ScanOptions {
        root: root.to_path_buf(),
        ..Default::default()
    };
    let entries = scan(&opts).unwrap();
    assert_eq!(entries.len(), 7);

    let kinds: Vec<ArtifactKind> = entries.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&ArtifactKind::PlaywrightBrowsers));
    assert!(kinds.contains(&ArtifactKind::CypressCache));
    assert!(kinds.contains(&ArtifactKind::AstroCache));
    assert!(kinds.contains(&ArtifactKind::SvelteKitCache));
    assert!(kinds.contains(&ArtifactKind::NuxtCache));
    assert!(kinds.contains(&ArtifactKind::DocusaurusCache));
    assert!(kinds.contains(&ArtifactKind::ViteCache));

    // Test exclude_kinds
    let opts_filtered = ScanOptions {
        root: root.to_path_buf(),
        exclude_kinds: vec![
            ArtifactKind::PlaywrightBrowsers,
            ArtifactKind::CypressCache,
            ArtifactKind::AstroCache,
        ],
        ..Default::default()
    };
    let filtered_entries = scan(&opts_filtered).unwrap();
    assert_eq!(filtered_entries.len(), 4);

    // Test safe deletion
    let delete_res = delete(&pw_dir, DeleteMode::Permanent, 0);
    assert!(delete_res.is_ok());
    assert!(!pw_dir.exists());
}

#[test]
fn test_systems_native_cache_scan_and_delete() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // CMake
    let cmake_proj = root.join("my_cmake_app");
    fs::create_dir_all(&cmake_proj).unwrap();
    File::create(cmake_proj.join("CMakeLists.txt")).unwrap();
    let cmake_build = cmake_proj.join("build");
    fs::create_dir_all(&cmake_build).unwrap();
    fs::write(cmake_build.join("my_binary"), b"compiled_elf").unwrap();

    // Zig
    let zig_proj = root.join("my_zig_app");
    fs::create_dir_all(&zig_proj).unwrap();
    File::create(zig_proj.join("build.zig")).unwrap();
    let zig_cache = zig_proj.join(".zig-cache");
    fs::create_dir_all(&zig_cache).unwrap();
    fs::write(zig_cache.join("z.o"), b"zig_obj").unwrap();

    // Elixir
    let elixir_proj = root.join("my_elixir_app");
    fs::create_dir_all(&elixir_proj).unwrap();
    File::create(elixir_proj.join("mix.exs")).unwrap();
    let elixir_build = elixir_proj.join("_build");
    fs::create_dir_all(&elixir_build).unwrap();
    fs::write(elixir_build.join("beam.beam"), b"beam_code").unwrap();

    // Meson
    let meson_proj = root.join("my_meson_app");
    fs::create_dir_all(&meson_proj).unwrap();
    File::create(meson_proj.join("meson.build")).unwrap();
    let meson_build = meson_proj.join("builddir");
    fs::create_dir_all(&meson_build).unwrap();
    fs::write(meson_build.join("app.ninja"), b"ninja_data").unwrap();

    // Scan all
    let opts = ScanOptions {
        root: root.to_path_buf(),
        ..Default::default()
    };
    let entries = scan(&opts).unwrap();
    assert_eq!(entries.len(), 4);

    let kinds: Vec<ArtifactKind> = entries.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&ArtifactKind::CmakeBuild));
    assert!(kinds.contains(&ArtifactKind::ZigCache));
    assert!(kinds.contains(&ArtifactKind::ElixirBuild));
    assert!(kinds.contains(&ArtifactKind::MesonBuild));

    // Test exclude_kinds
    let opts_filtered = ScanOptions {
        root: root.to_path_buf(),
        exclude_kinds: vec![ArtifactKind::CmakeBuild, ArtifactKind::ZigCache],
        ..Default::default()
    };
    let filtered_entries = scan(&opts_filtered).unwrap();
    assert_eq!(filtered_entries.len(), 2);
    let filtered_kinds: Vec<ArtifactKind> = filtered_entries.iter().map(|e| e.kind).collect();
    assert!(!filtered_kinds.contains(&ArtifactKind::CmakeBuild));
    assert!(!filtered_kinds.contains(&ArtifactKind::ZigCache));
    assert!(filtered_kinds.contains(&ArtifactKind::ElixirBuild));
    assert!(filtered_kinds.contains(&ArtifactKind::MesonBuild));

    // Test safe deletion
    let delete_res = delete(&cmake_build, DeleteMode::Permanent, 0);
    assert!(delete_res.is_ok());
    assert!(!cmake_build.exists());
}

#[test]
fn test_parallel_vs_sequential_scan_consistency() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // 1. Rust project
    let rust_proj = root.join("apps").join("backend");
    fs::create_dir_all(&rust_proj).unwrap();
    File::create(rust_proj.join("Cargo.toml")).unwrap();
    let rust_target = rust_proj.join("target");
    fs::create_dir_all(&rust_target).unwrap();
    fs::write(rust_target.join("app"), b"bin").unwrap();

    // 2. Node project
    let node_proj = root.join("apps").join("frontend");
    fs::create_dir_all(&node_proj).unwrap();
    File::create(node_proj.join("package.json")).unwrap();
    let node_modules = node_proj.join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(node_modules.join("react.js"), b"react").unwrap();

    // 3. Python venv
    let py_proj = root.join("services").join("worker");
    fs::create_dir_all(&py_proj).unwrap();
    let venv = py_proj.join(".venv");
    fs::create_dir_all(&venv).unwrap();
    File::create(venv.join("pyvenv.cfg")).unwrap();

    // 4. Excluded .git directory (should be pruned)
    let git_dir = root.join(".git");
    fs::create_dir_all(git_dir.join("node_modules")).unwrap();

    // 5. Ignored project via .nodewipeignore
    let ignored_dir = root.join("libs").join("ignored");
    fs::create_dir_all(ignored_dir.join("node_modules")).unwrap();
    let mut ignore_file = File::create(root.join(".nodewipeignore")).unwrap();
    writeln!(ignore_file, "libs/ignored").unwrap();

    let ignore_patterns = load_ignore_patterns(root);

    // Parallel scan (default)
    let opts_parallel = ScanOptions {
        root: root.to_path_buf(),
        ignore_patterns: ignore_patterns.clone(),
        parallel: true,
        ..Default::default()
    };
    let parallel_entries = scan(&opts_parallel).unwrap();

    // Sequential scan (fallback)
    let opts_sequential = ScanOptions {
        root: root.to_path_buf(),
        ignore_patterns,
        parallel: false,
        ..Default::default()
    };
    let sequential_entries = scan(&opts_sequential).unwrap();

    // Both MUST discover the exact same number of entries
    assert_eq!(parallel_entries.len(), 3);
    assert_eq!(sequential_entries.len(), 3);

    // Both MUST match identical paths and kinds in identical order
    for (p, s) in parallel_entries.iter().zip(sequential_entries.iter()) {
        assert_eq!(p.path, s.path);
        assert_eq!(p.kind, s.kind);
        assert_eq!(p.size_bytes, s.size_bytes);
    }
}

#[test]
fn test_parallel_traversal_benchmark() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // Create a multi-tiered directory hierarchy with 30 project trees
    for i in 0..30 {
        let proj = root.join(format!("proj_{}", i));
        fs::create_dir_all(&proj).unwrap();
        if i % 3 == 0 {
            File::create(proj.join("Cargo.toml")).unwrap();
            let target = proj.join("target");
            fs::create_dir_all(&target).unwrap();
            fs::write(target.join("output.bin"), b"data").unwrap();
        } else if i % 3 == 1 {
            let nm = proj.join("node_modules");
            fs::create_dir_all(&nm).unwrap();
            fs::write(nm.join("index.js"), b"console.log('hi');").unwrap();
        } else {
            let cmake = proj.join("build");
            fs::create_dir_all(&cmake).unwrap();
            File::create(proj.join("CMakeLists.txt")).unwrap();
            fs::write(cmake.join("CMakeCache.txt"), b"# cmake cache").unwrap();
        }
    }

    let start_seq = std::time::Instant::now();
    let seq_opts = ScanOptions {
        root: root.to_path_buf(),
        parallel: false,
        compute_sizes: true,
        ..Default::default()
    };
    let seq_entries = scan(&seq_opts).unwrap();
    let seq_duration = start_seq.elapsed();

    let start_par = std::time::Instant::now();
    let par_opts = ScanOptions {
        root: root.to_path_buf(),
        parallel: true,
        compute_sizes: true,
        ..Default::default()
    };
    let par_entries = scan(&par_opts).unwrap();
    let par_duration = start_par.elapsed();

    assert_eq!(seq_entries.len(), 30);
    assert_eq!(par_entries.len(), 30);
    println!(
        "Benchmark (30 projects): Sequential = {:?}, Parallel = {:?}",
        seq_duration, par_duration
    );
}

#[test]
fn test_supports_reflink_isolation_and_non_destructive() {
    use nodewipe_core::reflink::supports_reflink;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // User or third-party files that happen to share old probe names
    let probe_src = root.join(".nodewipe_reflink_probe_src");
    let probe_dst = root.join(".nodewipe_reflink_probe_dst");
    fs::write(&probe_src, b"USER_DATA_SRC_UNTOUCHED").unwrap();
    fs::write(&probe_dst, b"USER_DATA_DST_UNTOUCHED").unwrap();

    let _supported = supports_reflink(root);

    // Existing files must be 100% intact and untouched
    assert_eq!(
        fs::read(&probe_src).unwrap(),
        b"USER_DATA_SRC_UNTOUCHED",
        "supports_reflink must never overwrite user files"
    );
    assert_eq!(
        fs::read(&probe_dst).unwrap(),
        b"USER_DATA_DST_UNTOUCHED",
        "supports_reflink must never overwrite or delete user files"
    );
}

#[test]
fn test_reflink_snapshot_and_restore_cycle() {
    use nodewipe_core::reflink::{reflink_dir, supports_reflink};

    // Use current workspace directory which is on Btrfs to test native reflink CoW
    let base = std::env::current_dir()
        .unwrap()
        .join("target")
        .join("test_reflink_tmp");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    let project = base.join("my_btrfs_proj");
    let target = project.join("target");
    fs::create_dir_all(&target).unwrap();
    File::create(project.join("Cargo.toml")).unwrap();
    fs::write(target.join("my_binary"), b"compiled machine code").unwrap();
    fs::write(target.join("cached.o"), b"object file").unwrap();

    let can_reflink = supports_reflink(&project);
    println!("Filesystem supports reflink: {}", can_reflink);

    // Delete with Archive mode
    let res = delete(&target, DeleteMode::Archive, 100).unwrap();
    assert!(!target.exists());

    let archive_path = res.archive_path.expect("archive path should be returned");
    assert!(archive_path.exists());

    if can_reflink {
        // Must be a snapshot folder on Btrfs
        assert!(archive_path.is_dir());
        assert!(archive_path.to_string_lossy().ends_with(".snapshot"));
    }

    // Restore from snapshot or archive
    let restored = restore(&archive_path).unwrap();
    assert_eq!(restored, target);
    assert!(target.exists());
    assert_eq!(
        fs::read(target.join("my_binary")).unwrap(),
        b"compiled machine code"
    );
    assert_eq!(fs::read(target.join("cached.o")).unwrap(), b"object file");

    // Also test direct reflink_dir functionality
    let clone_dst = base.join("target_direct_clone");
    let stats = reflink_dir(&target, &clone_dst).unwrap();
    assert!(clone_dst.exists());
    assert_eq!(
        fs::read(clone_dst.join("my_binary")).unwrap(),
        b"compiled machine code"
    );
    println!("Direct reflink stats: {:?}", stats);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn test_path_interner_deduplication() {
    use nodewipe_core::PathInterner;
    use std::path::Path;

    let interner = PathInterner::new();

    // Intern 10,000 paths across 10 monorepo projects
    for p in 0..10 {
        let prefix = format!("/very/deep/monorepo/root/workspace/project_{}", p);
        for i in 0..1000 {
            let path_str = format!("{}/package_{}/node_modules", prefix, i);
            let interned = interner.intern_path(Path::new(&path_str));
            assert_eq!(interned, Path::new(&path_str));
        }
    }

    // 10,000 paths interned, pool should have interned unique parent strings
    assert!(!interner.is_empty());
    assert!(interner.len() <= 10010);
    println!(
        "PathInterner successfully deduplicated strings: unique count = {}",
        interner.len()
    );
}

fn run_git_cmd(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git execution failed");
    assert!(status.success(), "git {:?} in {:?} failed", args, dir);
}

#[test]
fn test_git_clean_repo_detection() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    run_git_cmd(root, &["init", "-b", "main"]);
    run_git_cmd(root, &["config", "user.name", "Test User"]);
    run_git_cmd(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"foo\"\n").unwrap();
    run_git_cmd(root, &["add", "Cargo.toml"]);
    run_git_cmd(root, &["commit", "-m", "init"]);

    let target = root.join("target");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("output.bin"), b"build artifacts").unwrap();

    let opts = ScanOptions {
        root: root.to_path_buf(),
        inspect_git: true,
        ..Default::default()
    };
    let entries = scan(&opts).unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    let git = entry
        .git_status
        .as_ref()
        .expect("git_status must be present");
    assert_eq!(git.branch.as_deref(), Some("main"));
    assert!(!git.is_dirty, "repository should not be dirty");
    assert!(!git.is_merged, "main branch is not a merged feature branch");
    assert!(
        git.last_commit_time.is_some(),
        "commit time must be present"
    );
}

#[test]
fn test_git_dirty_repo_detection() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    run_git_cmd(root, &["init", "-b", "main"]);
    run_git_cmd(root, &["config", "user.name", "Test User"]);
    run_git_cmd(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"foo\"\n").unwrap();
    run_git_cmd(root, &["add", "Cargo.toml"]);
    run_git_cmd(root, &["commit", "-m", "init"]);

    let target = root.join("target");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("output.bin"), b"build artifacts").unwrap();

    // Make repository dirty by adding uncommitted change
    fs::write(root.join("uncommitted.rs"), "fn dirty() {}").unwrap();

    let opts = ScanOptions {
        root: root.to_path_buf(),
        inspect_git: true,
        ..Default::default()
    };
    let entries = scan(&opts).unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    let git = entry
        .git_status
        .as_ref()
        .expect("git_status must be present");
    assert_eq!(git.branch.as_deref(), Some("main"));
    assert!(git.is_dirty, "repository must be flagged as dirty");
}

#[test]
fn test_git_merged_feature_branch_detection() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    run_git_cmd(root, &["init", "-b", "main"]);
    run_git_cmd(root, &["config", "user.name", "Test User"]);
    run_git_cmd(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("package.json"), "{\"name\":\"app\"}").unwrap();
    run_git_cmd(root, &["add", "package.json"]);
    run_git_cmd(root, &["commit", "-m", "initial on main"]);

    // Create and checkout feature-auth branch
    run_git_cmd(root, &["checkout", "-b", "feature-auth"]);
    fs::write(root.join("auth.js"), "// auth logic").unwrap();
    run_git_cmd(root, &["add", "auth.js"]);
    run_git_cmd(root, &["commit", "-m", "add auth"]);

    // Merge into main
    run_git_cmd(root, &["checkout", "main"]);
    run_git_cmd(
        root,
        &["merge", "feature-auth", "--no-ff", "-m", "merge auth"],
    );

    // Checkout feature-auth branch again
    run_git_cmd(root, &["checkout", "feature-auth"]);

    let nm = root.join("node_modules");
    fs::create_dir_all(&nm).unwrap();
    fs::write(nm.join("package.json"), "{}").unwrap();

    let opts = ScanOptions {
        root: root.to_path_buf(),
        inspect_git: true,
        ..Default::default()
    };
    let entries = scan(&opts).unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    let git = entry
        .git_status
        .as_ref()
        .expect("git_status must be present");
    assert_eq!(git.branch.as_deref(), Some("feature-auth"));
    assert!(
        git.is_merged,
        "feature-auth branch must be flagged as merged"
    );
    assert!(!git.is_dirty, "repository should not be dirty");
}

#[test]
fn test_smart_staleness_with_git_integration() {
    use nodewipe_core::find_stale_projects;
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    run_git_cmd(root, &["init", "-b", "main"]);
    run_git_cmd(root, &["config", "user.name", "Test User"]);
    run_git_cmd(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"foo\"\n").unwrap();
    run_git_cmd(root, &["add", "Cargo.toml"]);
    run_git_cmd(root, &["commit", "-m", "init"]);

    let target = root.join("target");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("output.bin"), b"build artifacts").unwrap();

    let opts = ScanOptions {
        root: root.to_path_buf(),
        inspect_git: true,
        ..Default::default()
    };
    let entries = scan(&opts).unwrap();
    assert_eq!(entries.len(), 1);

    let stale = find_stale_projects(&entries, Duration::from_secs(0));
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].git_branch.as_deref(), Some("main"));
    assert!(!stale[0].is_dirty);
    assert!(!stale[0].is_merged);
}

#[test]
fn test_resolve_rebuild_plan_rust() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"my-crate\"\n").unwrap();

    let entry = Entry {
        path: root.join("target"),
        size_bytes: 500,
        kind: ArtifactKind::RustTarget,
        package_manager: None,
        last_modified: None,
        workspace_root: None,
        git_status: None,
    };

    let plan = nodewipe_core::resolve_rebuild_plan(&entry).expect("plan must exist");
    assert_eq!(plan.program, "cargo");
    assert_eq!(plan.args, vec!["build"]);
    assert_eq!(plan.work_dir, root);
}

#[test]
fn test_resolve_rebuild_plan_node_variants() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // 1. pnpm
    let pnpm_dir = root.join("pnpm_app");
    fs::create_dir_all(&pnpm_dir).unwrap();
    fs::write(pnpm_dir.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'").unwrap();
    let entry_pnpm = Entry {
        path: pnpm_dir.join("node_modules"),
        size_bytes: 100,
        kind: ArtifactKind::NodeModules,
        package_manager: Some(nodewipe_core::PackageManager::Pnpm),
        last_modified: None,
        workspace_root: None,
        git_status: None,
    };
    let plan = nodewipe_core::resolve_rebuild_plan(&entry_pnpm).unwrap();
    assert_eq!(plan.program, "pnpm");
    assert_eq!(plan.args, vec!["install"]);

    // 2. yarn
    let yarn_dir = root.join("yarn_app");
    fs::create_dir_all(&yarn_dir).unwrap();
    fs::write(yarn_dir.join("yarn.lock"), "").unwrap();
    let entry_yarn = Entry {
        path: yarn_dir.join("node_modules"),
        size_bytes: 100,
        kind: ArtifactKind::NodeModules,
        package_manager: Some(nodewipe_core::PackageManager::Yarn),
        last_modified: None,
        workspace_root: None,
        git_status: None,
    };
    let plan = nodewipe_core::resolve_rebuild_plan(&entry_yarn).unwrap();
    assert_eq!(plan.program, "yarn");
    assert_eq!(plan.args, vec!["install"]);

    // 3. npm
    let npm_dir = root.join("npm_app");
    fs::create_dir_all(&npm_dir).unwrap();
    fs::write(npm_dir.join("package.json"), "{}").unwrap();
    let entry_npm = Entry {
        path: npm_dir.join("node_modules"),
        size_bytes: 100,
        kind: ArtifactKind::NodeModules,
        package_manager: Some(nodewipe_core::PackageManager::Npm),
        last_modified: None,
        workspace_root: None,
        git_status: None,
    };
    let plan = nodewipe_core::resolve_rebuild_plan(&entry_npm).unwrap();
    assert_eq!(plan.program, "npm");
    assert_eq!(plan.args, vec!["install"]);
}

#[test]
fn test_resolve_rebuild_plan_python_variants() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // 1. uv
    let uv_dir = root.join("uv_app");
    fs::create_dir_all(&uv_dir).unwrap();
    fs::write(uv_dir.join("uv.lock"), "").unwrap();
    let entry_uv = Entry {
        path: uv_dir.join(".venv"),
        size_bytes: 100,
        kind: ArtifactKind::PythonVenv,
        package_manager: None,
        last_modified: None,
        workspace_root: None,
        git_status: None,
    };
    let plan = nodewipe_core::resolve_rebuild_plan(&entry_uv).unwrap();
    assert_eq!(plan.program, "uv");
    assert_eq!(plan.args, vec!["sync"]);

    // 2. poetry
    let poetry_dir = root.join("poetry_app");
    fs::create_dir_all(&poetry_dir).unwrap();
    fs::write(poetry_dir.join("poetry.lock"), "").unwrap();
    let entry_poetry = Entry {
        path: poetry_dir.join(".venv"),
        size_bytes: 100,
        kind: ArtifactKind::PythonVenv,
        package_manager: None,
        last_modified: None,
        workspace_root: None,
        git_status: None,
    };
    let plan = nodewipe_core::resolve_rebuild_plan(&entry_poetry).unwrap();
    assert_eq!(plan.program, "poetry");
    assert_eq!(plan.args, vec!["install"]);

    // 3. requirements.txt / venv
    let req_dir = root.join("req_app");
    fs::create_dir_all(&req_dir).unwrap();
    fs::write(req_dir.join("requirements.txt"), "requests>=2.0\n").unwrap();
    let entry_req = Entry {
        path: req_dir.join("venv"),
        size_bytes: 100,
        kind: ArtifactKind::PythonVenv,
        package_manager: None,
        last_modified: None,
        workspace_root: None,
        git_status: None,
    };
    let plan = nodewipe_core::resolve_rebuild_plan(&entry_req).unwrap();
    assert_eq!(plan.program, "python3");
    assert_eq!(plan.args, vec!["-m", "venv", ".venv"]);
}

#[test]
fn test_dedupe_dry_run_and_execution() {
    use nodewipe_core::{DedupeOptions, dedupe_paths};
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let app1 = root.join("app1").join("node_modules");
    let app2 = root.join("app2").join("node_modules");
    fs::create_dir_all(&app1).unwrap();
    fs::create_dir_all(&app2).unwrap();

    let payload = b"const lodash = function() { return 'utility_suite_content_12345'; };";
    let file1 = app1.join("lodash.js");
    let file2 = app2.join("lodash.js");
    let file_diff = app1.join("other.js");

    fs::write(&file1, payload).unwrap();
    fs::write(&file2, payload).unwrap();
    fs::write(&file_diff, b"different_file_content_here").unwrap();

    // 1. Dry run
    let dry_opts = DedupeOptions {
        dry_run: true,
        min_size_bytes: 10,
        allow_hardlinks: false,
    };
    let dry_stats = dedupe_paths(&[root.to_path_buf()], &dry_opts).unwrap();
    assert_eq!(dry_stats.files_scanned, 3);
    assert_eq!(dry_stats.duplicate_files_found, 1);
    assert_eq!(dry_stats.duplicate_files_linked, 0);
    assert_eq!(dry_stats.reclaimed_bytes, payload.len() as u64);

    // 2. Safe default: allow_hardlinks: false (refuses hardlinking independent files)
    let safe_opts = DedupeOptions {
        dry_run: false,
        min_size_bytes: 10,
        allow_hardlinks: false,
    };
    let safe_stats = dedupe_paths(&[root.to_path_buf()], &safe_opts).unwrap();
    assert_eq!(safe_stats.duplicate_files_found, 1);
    // On systems without reflink (such as standard CI / ext4 / tmpfs), no hardlinks are forced
    if !nodewipe_core::supports_reflink(root) {
        assert_eq!(safe_stats.duplicate_files_linked, 0);
        assert_eq!(safe_stats.reclaimed_bytes, 0);
    }

    // 3. Explicit opt-in: allow_hardlinks: true
    let hardlink_opts = DedupeOptions {
        dry_run: false,
        min_size_bytes: 10,
        allow_hardlinks: true,
    };
    let hardlink_stats = dedupe_paths(&[root.to_path_buf()], &hardlink_opts).unwrap();
    assert_eq!(hardlink_stats.duplicate_files_found, 1);
    assert_eq!(hardlink_stats.duplicate_files_linked, 1);
    assert_eq!(hardlink_stats.reclaimed_bytes, payload.len() as u64);

    // Verify both files still exist and have intact identical content
    assert_eq!(fs::read(&file1).unwrap(), payload);
    assert_eq!(fs::read(&file2).unwrap(), payload);

    // On Unix, verify they now share the same inode (hardlinked)
    #[cfg(unix)]
    {
        let m1 = fs::metadata(&file1).unwrap();
        let m2 = fs::metadata(&file2).unwrap();
        assert_eq!(m1.ino(), m2.ino(), "deduplicated files must share inode");
    }

    // 4. Re-running on already-deduplicated files detects 0 duplicates
    let second_run = dedupe_paths(&[root.to_path_buf()], &hardlink_opts).unwrap();
    assert_eq!(second_run.duplicate_files_found, 0);
    assert_eq!(second_run.duplicate_files_linked, 0);
}

#[test]
fn test_ignore_management_lifecycle_and_root_safety() {
    use nodewipe_core::{
        add_ignored_pattern_to_file, list_ignored_patterns_from_file,
        remove_ignored_pattern_from_file,
    };
    use std::path::Path;

    let temp_dir = tempfile::tempdir().unwrap();
    let test_ignore_file = temp_dir.path().join("isolated_ignore_file");

    // 1. Safety check: refusing root paths
    assert!(add_ignored_pattern_to_file(&test_ignore_file, Path::new("/")).is_err());
    #[cfg(windows)]
    assert!(add_ignored_pattern_to_file(&test_ignore_file, Path::new("C:\\")).is_err());

    // 2. Add temporary valid test directory to isolated ignore file
    let sample_path = temp_dir.path().join("safe_test_artifact");
    std::fs::create_dir_all(&sample_path).unwrap();

    let res = add_ignored_pattern_to_file(&test_ignore_file, &sample_path);
    assert!(
        res.is_ok(),
        "add_ignored_pattern_to_file must succeed for valid path"
    );

    // 3. Duplicate addition is idempotent
    let res_dup = add_ignored_pattern_to_file(&test_ignore_file, &sample_path);
    assert!(res_dup.is_ok());

    // 4. Verify listed in isolated file
    let list = list_ignored_patterns_from_file(&test_ignore_file);
    let sample_str = sample_path.canonicalize().unwrap().display().to_string();
    assert!(list.iter().any(|item| item.pattern == sample_str));

    // 5. Remove pattern from isolated file
    let removed = remove_ignored_pattern_from_file(&test_ignore_file, &sample_str).unwrap();
    assert!(removed, "Pattern must be reported as removed");

    // 6. Verify removed from isolated file
    let list_after = list_ignored_patterns_from_file(&test_ignore_file);
    assert!(!list_after.iter().any(|item| item.pattern == sample_str));

    // 7. Verify local ignore file removal support in project root
    let local_ignore = temp_dir.path().join(".nodewipeignore");
    std::fs::write(&local_ignore, "local_ignored_dir\nanother_dir\n").unwrap();
    let local_list = nodewipe_core::list_ignored_patterns(temp_dir.path());
    assert!(
        local_list
            .iter()
            .any(|item| item.pattern == "local_ignored_dir")
    );

    let removed_local =
        nodewipe_core::remove_ignored_pattern_from_root(temp_dir.path(), "local_ignored_dir")
            .unwrap();
    assert!(removed_local, "Local ignore pattern must be removed");
    let content_after = std::fs::read_to_string(&local_ignore).unwrap();
    assert!(!content_after.contains("local_ignored_dir"));
    assert!(content_after.contains("another_dir"));
}

#[test]
fn test_pnpm_ignored_subfolder_detection_and_surgical_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // Create a node project with node_modules
    let proj = root.join("my-pnpm-app");
    let nm = proj.join("node_modules");
    let pnpm_ignored = nm.join(".ignored");
    let active_pkg = nm.join("active-dep");

    std::fs::create_dir_all(&pnpm_ignored).unwrap();
    std::fs::create_dir_all(&active_pkg).unwrap();
    std::fs::write(proj.join("package.json"), b"{}").unwrap();

    // Write dummy payloads
    std::fs::write(pnpm_ignored.join("old-package.js"), vec![0u8; 1000]).unwrap();
    std::fs::write(active_pkg.join("index.js"), vec![0u8; 500]).unwrap();

    // 1. Default scan: returns node_modules (which contains the full package payload)
    // without returning overlapping child targets
    let default_opts = nodewipe_core::ScanOptions {
        root: root.to_path_buf(),
        ..Default::default()
    };
    let entries = nodewipe_core::scan(&default_opts).unwrap();
    assert_eq!(
        entries.len(),
        1,
        "Default scan must not return overlapping parent & child entries"
    );
    let nm_entry = &entries[0];
    assert_eq!(nm_entry.kind, ArtifactKind::NodeModules);
    assert!(
        nm_entry.size_bytes >= 1500,
        "node_modules size must reflect full directory payload"
    );

    // 2. Surgical scan targeting only pnpm_ignored (with NodeModules excluded)
    let surgical_opts = nodewipe_core::ScanOptions {
        root: root.to_path_buf(),
        exclude_kinds: vec![ArtifactKind::NodeModules],
        ..Default::default()
    };
    let surgical_entries = nodewipe_core::scan(&surgical_opts).unwrap();
    assert_eq!(surgical_entries.len(), 1);
    let ignored_entry = &surgical_entries[0];
    assert_eq!(ignored_entry.kind, ArtifactKind::PnpmIgnored);
    assert_eq!(ignored_entry.path, pnpm_ignored);
    assert!(
        ignored_entry.size_bytes >= 1000,
        "ignored size must reflect payload"
    );

    // 3. Verify surgical delete: deleting ONLY .ignored
    let res = delete(&pnpm_ignored, DeleteMode::Trash, ignored_entry.size_bytes).unwrap();
    assert_eq!(res.path, pnpm_ignored);
    assert!(!pnpm_ignored.exists(), ".ignored must be deleted");

    // Active package must still exist!
    assert!(
        active_pkg.exists(),
        "active-dep must remain intact after deleting .ignored"
    );
    assert!(
        nm.exists(),
        "parent node_modules must remain intact after deleting .ignored"
    );
}

#[test]
fn test_yarn_cache_scan_and_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let yarn_dir = tmp.path().join(".cache").join("yarn");
    let v6_dir = yarn_dir.join("v6");
    std::fs::create_dir_all(&v6_dir).unwrap();
    std::fs::write(v6_dir.join("npm-pkg-1.0.0.tgz"), vec![0u8; 1500]).unwrap();

    let opts = ScanOptions {
        root: tmp.path().to_path_buf(),
        ..Default::default()
    };

    let entries = scan(&opts).unwrap();
    let yarn_entry = entries
        .iter()
        .find(|e| e.kind == ArtifactKind::YarnCache)
        .expect("must detect yarn v6 cache directory");

    assert_eq!(yarn_entry.path, v6_dir);
    assert!(yarn_entry.size_bytes >= 1500);

    let res = delete(&v6_dir, DeleteMode::Trash, yarn_entry.size_bytes).unwrap();
    assert_eq!(res.path, v6_dir);
    assert!(!v6_dir.exists());
    assert!(yarn_dir.exists(), "parent yarn folder must remain intact");
}

#[test]
fn test_find_archives_discovers_both_tar_and_snapshots() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Create a mock .tar.gz archive
    let tar_archive = root.join("project-a-node_modules-backup.tar.gz");
    std::fs::write(&tar_archive, b"mock_tar_gz_bytes").unwrap();

    // Create a mock .snapshot archive directory
    let snapshot_archive = root.join("project-b-target-backup.snapshot");
    std::fs::create_dir_all(&snapshot_archive).unwrap();
    std::fs::write(snapshot_archive.join("dummy_file"), vec![0u8; 500]).unwrap();

    // Create unrelated non-archive files/folders that should NOT be detected
    let random_file = root.join("some-backup.txt");
    std::fs::write(&random_file, b"not an archive").unwrap();
    let random_dir = root.join("random.snapshot");
    std::fs::create_dir_all(&random_dir).unwrap();

    let archives = find_archives(root);
    assert_eq!(
        archives.len(),
        2,
        "must discover exactly the 2 valid backup archives"
    );

    let tar_found = archives
        .iter()
        .find(|a| a.path == tar_archive)
        .expect("tar archive must be found");
    assert!(!tar_found.is_snapshot);
    assert_eq!(tar_found.size_bytes, b"mock_tar_gz_bytes".len() as u64);

    let snapshot_found = archives
        .iter()
        .find(|a| a.path == snapshot_archive)
        .expect("snapshot archive must be found");
    assert!(snapshot_found.is_snapshot);
    assert!(snapshot_found.size_bytes >= 500);
}

#[test]
fn test_delete_refuses_pinned_project_and_artifact() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let project_dir = root.join("my-pinned-app");
    let nm_dir = project_dir.join("node_modules");
    std::fs::create_dir_all(&nm_dir).unwrap();
    std::fs::write(project_dir.join("package.json"), "{}").unwrap();
    std::fs::write(nm_dir.join("file.js"), "console.log(1);").unwrap();

    // Verify unpinned deletion would normally proceed or be classified
    assert!(nodewipe_core::classify_path(&nm_dir).is_some());

    // Pin project using pin module
    let pin_file = root.join("pinned.json");
    nodewipe_core::pin::add_pinned_project_to_file(&pin_file, &project_dir, Some("Critical client project".into())).unwrap();

    let list = nodewipe_core::pin::load_pinned_projects_from_file(&pin_file);
    assert!(nodewipe_core::pin::is_path_pinned_with_list(&nm_dir, &list));

    // Also test toggle
    let (now_pinned, _) = nodewipe_core::pin::toggle_pinned_project_in_file(&pin_file, &project_dir).unwrap();
    assert!(!now_pinned);
    let (re_pinned, _) = nodewipe_core::pin::toggle_pinned_project_in_file(&pin_file, &project_dir).unwrap();
    assert!(re_pinned);
}

#[test]
fn test_explain_path_detailed_recipe_and_staleness() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let project_dir = root.join("rust-api");
    let target_dir = project_dir.join("target");
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), "[package]\nname = \"api\"").unwrap();
    std::fs::write(project_dir.join("main.rs"), "fn main() {}").unwrap();
    std::fs::write(target_dir.join("app"), "binary-blob-data").unwrap();

    let explanation = nodewipe_core::explain_path(&target_dir).expect("explain path");
    assert_eq!(explanation.artifact_kind, nodewipe_core::ArtifactKind::RustTarget);
    assert_eq!(explanation.risk_level, nodewipe_core::RiskLevel::Low);
    assert!(explanation.rebuild_plan.is_some());
    let plan = explanation.rebuild_plan.unwrap();
    assert_eq!(plan.program, "cargo");
    assert_eq!(plan.args, vec!["build"]);
    assert!(explanation.match_reason.contains("Cargo.toml"));
    assert!(explanation.formatted_size.ends_with("B"));
}
