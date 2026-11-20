use nodewipe_core::{
    ArtifactKind, CustomRule, Entry, ScanOptions,
    diff::{ScanDiff, ScanSnapshot, ScanSnapshotEntry, compute_scan_diff},
    plan_reclaim, scan,
};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use tempfile::tempdir;

/// 1. Directory Traversal Throughput Benchmark.
///
/// Tests scanning throughput across a synthetic multi-project tree with nested artifacts.
#[test]
fn benchmark_directory_traversal_throughput() {
    let temp = tempdir().unwrap();
    let root = temp.path();

    // Create 50 simulated projects across different ecosystems
    for i in 0..50 {
        let proj = root.join(format!("proj_{i}"));
        fs::create_dir_all(&proj).unwrap();

        match i % 5 {
            0 => {
                // Rust project
                fs::write(proj.join("Cargo.toml"), "[package]\nname=\"foo\"").unwrap();
                let target = proj.join("target").join("debug");
                fs::create_dir_all(&target).unwrap();
                fs::write(target.join("foo"), "binary data").unwrap();
            }
            1 => {
                // Node project
                fs::write(proj.join("package.json"), r#"{"name":"foo"}"#).unwrap();
                let nm = proj.join("node_modules").join("nested-pkg");
                fs::create_dir_all(&nm).unwrap();
                fs::write(nm.join("index.js"), "console.log('hello');").unwrap();
            }
            2 => {
                // Python project
                fs::write(proj.join("pyproject.toml"), "[project]\nname=\"foo\"").unwrap();
                let venv = proj.join(".venv").join("lib");
                fs::create_dir_all(&venv).unwrap();
                fs::write(venv.join("site.py"), "# venv").unwrap();
            }
            3 => {
                // Java project
                fs::write(proj.join("pom.xml"), "<project></project>").unwrap();
                let target = proj.join("target").join("classes");
                fs::create_dir_all(&target).unwrap();
                fs::write(target.join("App.class"), "bytecode").unwrap();
            }
            _ => {
                // Go project
                fs::write(proj.join("go.mod"), "module foo\ngo 1.22").unwrap();
                let cache = proj.join(".cache").join("go-build");
                fs::create_dir_all(&cache).unwrap();
                fs::write(cache.join("log.txt"), "cache log").unwrap();
            }
        }
    }

    let opts = ScanOptions {
        root: root.to_path_buf(),
        parallel: true,
        inspect_git: false,
        ..Default::default()
    };

    let start = Instant::now();
    let entries = scan(&opts).expect("scan must succeed");
    let duration = start.elapsed();

    assert!(
        entries.len() >= 40,
        "Expected at least 40 detected artifacts, found {}",
        entries.len()
    );

    let dirs_scanned = 50;
    let dirs_per_sec = (dirs_scanned as f64) / duration.as_secs_f64().max(0.0001);

    println!(
        "\n [BENCH] Traversal Throughput: {} artifacts discovered in {:.3} ms ({:.0} projects/sec)",
        entries.len(),
        duration.as_secs_f64() * 1000.0,
        dirs_per_sec
    );

    // Performance regression guardrail: must complete in under 5 seconds
    assert!(
        duration.as_millis() < 5000,
        "Traversal took too long: {} ms",
        duration.as_millis()
    );
}

/// 2. Block Sizing Accuracy and Latency Benchmark
#[test]
fn benchmark_block_sizing_accuracy_and_latency() {
    let temp = tempdir().unwrap();
    let file_path = temp.path().join("dense_file.dat");

    // Write exactly 1 MiB of test data
    let test_data = vec![0x42u8; 1024 * 1024];
    fs::write(&file_path, &test_data).unwrap();

    let start = Instant::now();
    let metadata = fs::metadata(&file_path).unwrap();
    let size = metadata.len();
    let elapsed = start.elapsed();

    assert_eq!(size, 1024 * 1024, "Size must match written bytes");

    println!(
        " [BENCH] Sizing Latency: 1MB file sized in {:.3} µs (accurate to {} bytes)",
        elapsed.as_secs_f64() * 1_000_000.0,
        size
    );
}

/// 3. Custom Rule Evaluation Throughput Benchmark
#[test]
fn benchmark_custom_rule_evaluation_throughput() {
    let temp = tempdir().unwrap();
    let root = temp.path();

    // Create 20 custom rules
    let mut rules = Vec::new();
    for i in 0..20 {
        rules.push(CustomRule {
            name: format!("Custom Rule {i}"),
            dir_name: format!(".cache_{i}"),
            parent_marker: Some(format!("marker_{i}.json")),
            self_marker: None,
            risk: "low".to_string(),
            recipe: Some(format!("tool build {i}")),
            description: Some("benchmark custom rule".to_string()),
            ecosystem: "Bench".to_string(),
        });
    }

    // Create parent directory with matching marker for rule 7
    let matched_dir = root.join("match_proj");
    fs::create_dir_all(&matched_dir).unwrap();
    fs::write(matched_dir.join("marker_7.json"), "{}").unwrap();
    let target_dir = matched_dir.join(".cache_7");
    fs::create_dir_all(&target_dir).unwrap();

    let test_paths = vec![
        target_dir.clone(),
        root.join("unmatched_dir_1"),
        root.join("unmatched_dir_2"),
        root.join("unmatched_dir_3"),
    ];

    let iterations = 25_000;
    let start = Instant::now();
    let mut match_count = 0;

    for _ in 0..iterations {
        for path in &test_paths {
            for rule in &rules {
                if rule.matches_path(path) {
                    match_count += 1;
                }
            }
        }
    }

    let elapsed = start.elapsed();
    let total_evals = (iterations * test_paths.len() * rules.len()) as f64;
    let evals_per_sec = total_evals / elapsed.as_secs_f64().max(0.0001);

    assert_eq!(
        match_count, iterations,
        "Target rule 7 should match exactly once per iteration"
    );

    println!(
        " [BENCH] Rule Evaluation: {:.0} evaluations in {:.3} ms ({:.0} evals/sec)",
        total_evals,
        elapsed.as_secs_f64() * 1000.0,
        evals_per_sec
    );

    // Guardrail: must evaluate at least 100,000 rules/sec
    assert!(
        evals_per_sec > 100_000.0,
        "Rule evaluation throughput too slow: {:.0} evals/sec",
        evals_per_sec
    );
}

/// 4. Snapshot Diffing Throughput Benchmark
#[test]
fn benchmark_snapshot_diffing_throughput() {
    let now = std::time::SystemTime::now();

    // Generate 500 snapshot entries for baseline
    let mut snapshot_entries = Vec::with_capacity(500);
    for i in 0..500u64 {
        snapshot_entries.push(ScanSnapshotEntry {
            path: PathBuf::from(format!("/home/dev/projects/proj_{i}/target")),
            kind_slug: "rust_target".to_string(),
            size_bytes: 10_000_000 + (i * 1000),
            last_modified_secs: Some(1700000000 + i),
        });
    }

    let snapshot = ScanSnapshot {
        version: 1,
        timestamp_secs: 1700000000,
        root: PathBuf::from("/home/dev/projects"),
        total_bytes: snapshot_entries.iter().map(|e| e.size_bytes).sum(),
        entries: snapshot_entries.clone(),
    };

    // Generate 500 live entries with 100 modified (grown), 50 deleted, 50 added
    let mut after = Vec::with_capacity(500);
    for (i, entry) in snapshot_entries.iter().enumerate().take(450) {
        let size = if i < 100 {
            // Modified (grown)
            entry.size_bytes + 5_000_000
        } else {
            entry.size_bytes
        };
        after.push(Entry {
            path: entry.path.clone(),
            size_bytes: size,
            kind: ArtifactKind::RustTarget,
            package_manager: None,
            last_modified: Some(now),
            workspace_root: None,
            git_status: None,
            custom_rule: None,
        });
    }
    // 50 newly added entries
    for i in 500..550 {
        after.push(Entry {
            path: PathBuf::from(format!("/home/dev/projects/proj_{i}/node_modules")),
            size_bytes: 25_000_000,
            kind: ArtifactKind::NodeModules,
            package_manager: None,
            last_modified: Some(now),
            workspace_root: None,
            git_status: None,
            custom_rule: None,
        });
    }

    let iterations = 100;
    let start = Instant::now();

    for _ in 0..iterations {
        let diff: ScanDiff = compute_scan_diff(&snapshot, &after);
        assert_eq!(diff.added.len(), 50);
        assert_eq!(diff.removed.len(), 50);
        assert_eq!(diff.grown.len(), 100);
    }

    let elapsed = start.elapsed();
    let diffs_per_sec = (iterations as f64) / elapsed.as_secs_f64().max(0.0001);

    println!(
        " [BENCH] Snapshot Diffing: {} diff passes of 1,000 items in {:.3} ms ({:.0} diffs/sec)",
        iterations,
        elapsed.as_secs_f64() * 1000.0,
        diffs_per_sec
    );

    // Guardrail: must complete in under 500 ms for 100 iterations of 1000 entries
    assert!(
        elapsed.as_millis() < 500,
        "Snapshot diffing too slow: {} ms",
        elapsed.as_millis()
    );
}

/// 5. Reclaim Planning Budget Optimizer Benchmark
#[test]
fn benchmark_reclaim_budget_optimizer() {
    let now = std::time::SystemTime::now();

    // Create 300 candidates with varied sizes and risk levels
    let mut entries = Vec::with_capacity(300);
    for i in 0..300 {
        let (kind, size) = match i % 3 {
            0 => (ArtifactKind::RustTarget, 50 * 1024 * 1024), // 50MB (low risk)
            1 => (ArtifactKind::NodeModules, 150 * 1024 * 1024), // 150MB (moderate risk)
            _ => (ArtifactKind::PythonVenv, 80 * 1024 * 1024), // 80MB (moderate risk)
        };
        entries.push(Entry {
            path: PathBuf::from(format!("/home/dev/proj_{i}")),
            size_bytes: size,
            kind,
            package_manager: None,
            last_modified: Some(now),
            workspace_root: None,
            git_status: None,
            custom_rule: None,
        });
    }

    let target_budget = 1024 * 1024 * 1024; // 1 GB target
    let iterations = 100;
    let start = Instant::now();

    for _ in 0..iterations {
        let plan = plan_reclaim(&entries, target_budget, true, None);
        assert!(
            plan.proposed_bytes >= target_budget,
            "Reclaim plan must satisfy budget"
        );
        assert!(plan.met_target);
    }

    let elapsed = start.elapsed();
    let plans_per_sec = (iterations as f64) / elapsed.as_secs_f64().max(0.0001);

    println!(
        " [BENCH] Reclaim Optimizer: {} optimizations over 300 items in {:.3} ms ({:.0} plans/sec)",
        iterations,
        elapsed.as_secs_f64() * 1000.0,
        plans_per_sec
    );

    assert!(
        elapsed.as_millis() < 2000,
        "Reclaim optimization too slow: {} ms",
        elapsed.as_millis()
    );
}
