use flate2::Compression;
use flate2::write::GzEncoder;
use nodewipe_core::{
    ArtifactKind, DedupeOptions, DeleteMode, ScanOptions, dedupe_paths, delete, restore, scan,
    supports_reflink,
};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;
use tempfile::tempdir;

#[test]
fn test_stress_concurrent_archive_and_restore_cycle() {
    const NUM_THREADS: usize = 20;
    let temp_root = tempdir().unwrap();
    let root_path = temp_root.path().to_path_buf();
    let barrier = Arc::new(Barrier::new(NUM_THREADS));
    let mut handles = Vec::new();

    for thread_id in 0..NUM_THREADS {
        let b = Arc::clone(&barrier);
        let project_dir = root_path.join(format!("proj_{thread_id}"));
        let nm = project_dir.join("node_modules");

        fs::create_dir_all(&nm).unwrap();
        fs::write(project_dir.join("package.json"), r#"{"name": "test-pkg"}"#).unwrap();

        // Write nested files inside node_modules
        let sub = nm.join("some-pkg");
        fs::create_dir_all(&sub).unwrap();
        let payload = format!("content from thread {thread_id}\n").repeat(200);
        fs::write(sub.join("index.js"), &payload).unwrap();

        handles.push(thread::spawn(move || {
            // Wait for all threads to initialize their directories
            b.wait();

            // 1. Delete in Archive mode concurrently
            let res = delete(&nm, DeleteMode::Archive, 5000).expect("Archive delete must succeed");
            assert!(!nm.exists(), "Original directory must be removed");
            let archive_path = res.archive_path.expect("archive_path must be populated");
            assert!(
                archive_path.exists(),
                "Backup archive or snapshot must exist at {}",
                archive_path.display()
            );

            // Wait before restoring concurrently
            b.wait();

            // 2. Restore archive concurrently
            let restored = restore(&archive_path).expect("Restore must succeed");
            assert_eq!(restored, nm, "Restored path must match original path");
            assert!(nm.exists(), "Restored directory must exist");

            // Verify payload integrity
            let read_payload = fs::read_to_string(sub.join("index.js")).unwrap();
            assert_eq!(read_payload, payload, "Payload content must match exactly");
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn test_stress_malformed_corrupt_tar_and_gzip_fuzzing() {
    let temp = tempdir().unwrap();
    let root = temp.path();

    // 1. Completely empty archive file
    let empty_archive = root.join("empty-backup.tar.gz");
    fs::write(&empty_archive, b"").unwrap();
    assert!(
        restore(&empty_archive).is_err(),
        "Empty archive must fail restore"
    );

    // 2. Truncated gzip stream (invalid header)
    let truncated_archive = root.join("truncated-backup.tar.gz");
    fs::write(&truncated_archive, b"\x1f\x8b\x08\x00corrupted").unwrap();
    assert!(
        restore(&truncated_archive).is_err(),
        "Truncated gzip header must fail restore"
    );

    // 3. Valid Gzip containing random garbage (not a tar)
    let garbage_archive = root.join("garbage-backup.tar.gz");
    {
        let file = fs::File::create(&garbage_archive).unwrap();
        let mut gz = GzEncoder::new(file, Compression::default());
        gz.write_all(b"This is definitely not a tar archive, just plain text")
            .unwrap();
        gz.finish().unwrap();
    }
    assert!(
        restore(&garbage_archive).is_err(),
        "Non-tar gzip must fail restore"
    );

    // 4. Hostile Tar: Path Traversal (escaping to ../../../evil)
    let traversal_archive = root.join("traversal-backup.tar.gz");
    {
        let file = fs::File::create(&traversal_archive).unwrap();
        let mut gz = GzEncoder::new(file, Compression::default());

        let mut header = tar::Header::new_gnu();
        let data = b"ATTACK_PAYLOAD";
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        let name_bytes = b"node_modules/../../../../../../tmp/evil_pwned.txt\0";
        header.as_mut_bytes()[..name_bytes.len()].copy_from_slice(name_bytes);
        header.set_cksum();

        let mut raw_tar = Vec::new();
        raw_tar.extend_from_slice(header.as_bytes());
        raw_tar.extend_from_slice(data);
        let pad = 512 - (data.len() % 512);
        if pad < 512 {
            raw_tar.extend(std::iter::repeat_n(0, pad));
        }
        raw_tar.extend(std::iter::repeat_n(0, 1024));

        gz.write_all(&raw_tar).unwrap();
        gz.finish().unwrap();
    }
    let res = restore(&traversal_archive);
    assert!(
        res.is_err(),
        "Archive containing path traversal must be rejected"
    );
    assert!(
        !Path::new("/tmp/evil_pwned.txt").exists(),
        "Traversal must not create file outside artifact"
    );

    // 5. Hostile Tar: Multiple roots (sneak file into parent directory)
    let multi_root_archive = root.join("multiroot-backup.tar.gz");
    {
        let file = fs::File::create(&multi_root_archive).unwrap();
        let gz = GzEncoder::new(file, Compression::default());
        let mut tar_builder = tar::Builder::new(gz);

        // Entry 1: legitimate node_modules file
        let mut h1 = tar::Header::new_gnu();
        let d1 = b"LEGIT";
        h1.set_size(d1.len() as u64);
        h1.set_mode(0o644);
        h1.set_cksum();
        tar_builder
            .append_data(&mut h1, "node_modules/index.js", &d1[..])
            .unwrap();

        // Entry 2: sneaky package.json overwrite attempt in parent
        let mut h2 = tar::Header::new_gnu();
        let d2 = b"EVIL_CONFIG";
        h2.set_size(d2.len() as u64);
        h2.set_mode(0o644);
        h2.set_cksum();
        tar_builder
            .append_data(&mut h2, "package.json", &d2[..])
            .unwrap();

        let gz = tar_builder.into_inner().unwrap();
        gz.finish().unwrap();
    }
    let multi_res = restore(&multi_root_archive);
    assert!(
        multi_res.is_err(),
        "Archive containing entries outside primary root must be rejected"
    );
}

#[test]
fn test_stress_deeply_nested_unicode_and_hostile_characters() {
    let temp = tempdir().unwrap();
    let root = temp.path();

    // Build 40-level deep hierarchy with special characters
    let mut current = root.to_path_buf();
    for i in 0..40 {
        current = current.join(format!("lvl_{i}_🦀"));
    }
    let nm = current.join("node_modules");
    fs::create_dir_all(&nm).unwrap();
    fs::write(current.join("package.json"), r#"{"name": "deep-pkg"}"#).unwrap();

    // Create files with hostile characters
    let hostile_names = [
        "regular.js",
        "with spaces and tabs\t.txt",
        "emoji_🚀_📦_🦀_日本語.dat",
        "$DOLLAR_VAR.env",
        "--leading-dash.config",
        "quoted_\"double\"_'single'.json",
        "symbols_!@#%^&*()-_+=~.log",
    ];

    let mut total_expected_bytes = 0u64;
    for (i, name) in hostile_names.iter().enumerate() {
        let file_path = nm.join(name);
        let payload = vec![(i % 255) as u8; 512 + i * 100];
        total_expected_bytes += payload.len() as u64;
        fs::write(&file_path, &payload).unwrap();
    }

    // Scan deep directory
    let opts = ScanOptions {
        root: root.to_path_buf(),
        ..Default::default()
    };
    let entries = scan(&opts).expect("Scan on deep hierarchy must succeed");
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.kind, ArtifactKind::NodeModules);
    assert_eq!(entry.path, nm);
    assert!(
        entry.size_bytes >= total_expected_bytes,
        "Reported size ({}) must be >= expected payload ({})",
        entry.size_bytes,
        total_expected_bytes
    );

    // Archive & Restore deep hierarchy with hostile names
    let archive_res = delete(&nm, DeleteMode::Archive, entry.size_bytes).unwrap();
    assert!(!nm.exists());

    let archive_path = archive_res
        .archive_path
        .expect("archive_path must be populated");
    assert!(archive_path.exists());

    let restore_res = restore(&archive_path);
    assert!(restore_res.is_ok(), "Deep restore must succeed");
    assert!(nm.exists());

    for name in hostile_names.iter() {
        assert!(
            nm.join(name).exists(),
            "Hostile filename {} must survive archive/restore cycle",
            name
        );
    }
}

#[test]
fn test_stress_circular_and_escaping_symlink_containment() {
    let temp = tempdir().unwrap();
    let root = temp.path();

    let project = root.join("my-app");
    let nm = project.join("node_modules");
    fs::create_dir_all(&nm).unwrap();
    fs::write(project.join("package.json"), r#"{"name": "app"}"#).unwrap();

    let outside_canary = root.join("canary_important_file.txt");
    fs::write(&outside_canary, b"DO_NOT_TOUCH").unwrap();

    // Create circular and escaping symlinks inside node_modules
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        // Circular loop: node_modules/self_loop -> .
        let _ = symlink(".", nm.join("self_loop"));
        // Circular loop: node_modules/parent_loop -> ..
        let _ = symlink("..", nm.join("parent_loop"));
        // Escaping symlink pointing out-of-tree
        let _ = symlink(&outside_canary, nm.join("escaping_canary"));
    }

    // Normal payload
    fs::write(nm.join("index.js"), b"console.log('hi');").unwrap();

    // Scan must terminate rapidly without getting trapped in loops
    let opts = ScanOptions {
        root: root.to_path_buf(),
        follow_symlinks: false,
        ..Default::default()
    };
    let entries = scan(&opts).expect("Scan must safely handle symlink loops");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, ArtifactKind::NodeModules);

    // Permanent delete of node_modules
    let del_res = delete(&nm, DeleteMode::Permanent, 100);
    assert!(del_res.is_ok(), "Permanent delete must succeed");
    assert!(!nm.exists(), "node_modules must be removed");

    // CRITICAL: canary file outside MUST NOT be modified or deleted
    assert!(
        outside_canary.exists(),
        "External canary file pointed to by symlink must NOT be deleted"
    );
    assert_eq!(fs::read_to_string(&outside_canary).unwrap(), "DO_NOT_TOUCH");
}

#[test]
fn test_stress_concurrent_reflink_probe_collision_resilience() {
    const THREADS: usize = 25;
    let temp = tempdir().unwrap();
    let dir = Arc::new(temp.path().to_path_buf());
    let barrier = Arc::new(Barrier::new(THREADS));
    let mut handles = Vec::new();

    for id in 0..THREADS {
        let b = Arc::clone(&barrier);
        let d = Arc::clone(&dir);
        handles.push(thread::spawn(move || {
            // Also write regular files to same directory concurrently
            let canary = d.join(format!("canary_{id}.txt"));
            fs::write(&canary, b"data").unwrap();

            b.wait();

            // Run reflink probe 10 times in tight loop under contention
            for _ in 0..10 {
                let _ = supports_reflink(&d);
            }

            // Verify our canary was never touched or deleted by supports_reflink
            assert!(
                canary.exists(),
                "Pre-existing file must not be affected by supports_reflink"
            );
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn test_stress_dedupe_contention_and_safety() {
    let temp = tempdir().unwrap();
    let root = temp.path();

    let proj1 = root.join("proj_a");
    let proj2 = root.join("proj_b");
    fs::create_dir_all(&proj1).unwrap();
    fs::create_dir_all(&proj2).unwrap();

    // Create 30 identical files across both projects
    for i in 0..30 {
        let payload = format!("identical content chunk {i}\n").repeat(100);
        fs::write(proj1.join(format!("file_{i}.js")), &payload).unwrap();
        fs::write(proj2.join(format!("file_{i}.js")), &payload).unwrap();
    }

    // 1. Dry run deduplication
    let dry_opts = DedupeOptions {
        dry_run: true,
        allow_hardlinks: false,
        ..Default::default()
    };
    let dry_stats = dedupe_paths(&[proj1.clone(), proj2.clone()], &dry_opts)
        .expect("Dry run dedupe must succeed");
    assert_eq!(dry_stats.duplicate_files_found, 30);
    assert_eq!(dry_stats.duplicate_files_linked, 0);

    // 2. Real deduplication with default copy-on-write (reflink only)
    let reflink_opts = DedupeOptions {
        dry_run: false,
        allow_hardlinks: false,
        ..Default::default()
    };
    let reflink_stats = dedupe_paths(&[proj1.clone(), proj2.clone()], &reflink_opts)
        .expect("Reflink dedupe must succeed");
    assert_eq!(reflink_stats.duplicate_files_found, 30);

    // Verify all files in both projects still exist and have intact data
    for i in 0..30 {
        let f1 = proj1.join(format!("file_{i}.js"));
        let f2 = proj2.join(format!("file_{i}.js"));
        assert!(f1.exists());
        assert!(f2.exists());
        assert_eq!(
            fs::read_to_string(&f1).unwrap(),
            fs::read_to_string(&f2).unwrap()
        );
    }
}
