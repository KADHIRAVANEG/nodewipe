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
