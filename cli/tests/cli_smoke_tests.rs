use std::process::Command;
use tempfile::TempDir;

#[test]
fn test_cli_man_command_outputs_roff() {
    let output = Command::new(env!("CARGO_BIN_EXE_nodewipe"))
        .arg("man")
        .output()
        .expect("failed to execute nodewipe man");

    assert!(output.status.success(), "nodewipe man must succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(".TH nodewipe 1"),
        "man page must contain .TH nodewipe 1 header"
    );
    assert!(
        stdout.contains("Find and reclaim disk space"),
        "man page must contain description"
    );
}

#[test]
fn test_cli_ignore_lifecycle_and_scan_exclusion_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Create two mock project trees
    let proj_a = root.join("proj_a");
    let nm_a = proj_a.join("node_modules");
    std::fs::create_dir_all(&nm_a).unwrap();
    std::fs::write(proj_a.join("package.json"), "{}").unwrap();

    let proj_b = root.join("proj_b");
    let nm_b = proj_b.join("node_modules");
    std::fs::create_dir_all(&nm_b).unwrap();
    std::fs::write(proj_b.join("package.json"), "{}").unwrap();

    // Scan initially: both node_modules must be discovered
    let initial_scan = Command::new(env!("CARGO_BIN_EXE_nodewipe"))
        .arg(root)
        .arg("--json")
        .arg("--no-git")
        .output()
        .expect("initial scan");

    assert!(initial_scan.status.success());
    let initial_json: serde_json::Value =
        serde_json::from_slice(&initial_scan.stdout).expect("valid scan json");
    let entries = initial_json["entries"]
        .as_array()
        .expect("array of entries");
    assert_eq!(entries.len(), 2, "initial scan discovers both node_modules");

    // Add proj_a to a local ignore file inside root
    let local_ignore = root.join(".nodewipeignore");
    std::fs::write(&local_ignore, format!("{}\n", nm_a.display())).unwrap();

    // Re-scan: nm_a must be excluded, only nm_b present
    let ignored_scan = Command::new(env!("CARGO_BIN_EXE_nodewipe"))
        .arg(root)
        .arg("--json")
        .arg("--no-git")
        .output()
        .expect("ignored scan");

    assert!(ignored_scan.status.success());
    let ignored_json: serde_json::Value =
        serde_json::from_slice(&ignored_scan.stdout).expect("valid scan json");
    let filtered_entries = ignored_json["entries"]
        .as_array()
        .expect("array of entries");
    assert_eq!(
        filtered_entries.len(),
        1,
        "ignored node_modules must be excluded from scan"
    );
    let remaining_path = filtered_entries[0]["path"].as_str().unwrap();
    assert!(remaining_path.contains("proj_b"));
}

#[test]
fn test_cli_stats_json_contract() {
    let output = Command::new(env!("CARGO_BIN_EXE_nodewipe"))
        .args(["stats", "--json"])
        .output()
        .expect("nodewipe stats --json");

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid stats json");
    assert!(
        json.get("total_freed_bytes").is_some(),
        "stats --json must contain 'total_freed_bytes'"
    );
    assert!(
        json.get("total_deletions").is_some(),
        "stats --json must contain 'total_deletions'"
    );
}

#[test]
fn test_cli_rules_list_and_catalog() {
    let output = Command::new(env!("CARGO_BIN_EXE_nodewipe"))
        .args(["rules", "list", "--builtin-only"])
        .output()
        .expect("nodewipe rules list");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("node_modules"));
    assert!(stdout.contains("target"));
}

#[test]
fn test_cli_dedupe_accurate_diagnostics() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Create duplicates
    let file1 = root.join("dup1.txt");
    let file2 = root.join("dup2.txt");
    let content = "identical-sample-data-exceeding-minimum-threshold-padding-1234567890\n";
    std::fs::write(&file1, content).unwrap();
    std::fs::write(&file2, content).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nodewipe"))
        .args(["dedupe", root.to_str().unwrap()])
        .output()
        .expect("nodewipe dedupe");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // If not reflinkable and not allow-hardlinks, it must not claim hardlinked success
    if stdout.contains("0 duplicate files linked") {
        assert!(stdout.contains("pass `--allow-hardlinks`"));
    }
}

#[test]
fn test_cli_scan_format_html_and_report_subcommand() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let proj = root.join("webapp");
    let nm = proj.join("node_modules");
    std::fs::create_dir_all(&nm).unwrap();
    std::fs::write(proj.join("package.json"), r#"{"name":"webapp"}"#).unwrap();
    std::fs::write(nm.join("bundle.js"), "console.log('bundle');").unwrap();

    // 1. nodewipe scan --format html <root>
    let scan_html = Command::new(env!("CARGO_BIN_EXE_nodewipe"))
        .args([
            "scan",
            "--format",
            "html",
            "--no-git",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("nodewipe scan --format html");

    assert!(
        scan_html.status.success(),
        "scan --format html failed: {}",
        String::from_utf8_lossy(&scan_html.stderr)
    );
    let html_stdout = String::from_utf8_lossy(&scan_html.stdout);
    assert!(html_stdout.starts_with("<!DOCTYPE html>"));
    assert!(html_stdout.contains("nodewipe Storage Audit Report"));
    assert!(html_stdout.contains("webapp"));
    assert!(html_stdout.contains("donut-svg"));
    assert!(html_stdout.contains("age-svg"));

    // 2. nodewipe report <root> --output <file>
    let out_file = root.join("audit-report.html");
    let report_cmd = Command::new(env!("CARGO_BIN_EXE_nodewipe"))
        .args([
            "report",
            root.to_str().unwrap(),
            "--output",
            out_file.to_str().unwrap(),
            "--no-git",
        ])
        .output()
        .expect("nodewipe report --output");

    assert!(
        report_cmd.status.success(),
        "nodewipe report failed: {}",
        String::from_utf8_lossy(&report_cmd.stderr)
    );
    assert!(out_file.exists(), "audit-report.html should exist");
    let file_html = std::fs::read_to_string(&out_file).unwrap();
    assert!(file_html.starts_with("<!DOCTYPE html>"));
    assert!(file_html.contains("node_modules"));

    // 3. nodewipe scan --export <out2.html>
    let export_file = root.join("exported.html");
    let export_cmd = Command::new(env!("CARGO_BIN_EXE_nodewipe"))
        .args([
            "scan",
            root.to_str().unwrap(),
            "--export",
            export_file.to_str().unwrap(),
            "--no-git",
        ])
        .output()
        .expect("nodewipe scan --export exported.html");
    assert!(export_cmd.status.success());
    let exported_html = std::fs::read_to_string(&export_file).unwrap();
    assert!(exported_html.starts_with("<!DOCTYPE html>"));
    assert!(exported_html.contains("webapp"));

    // 4. nodewipe scan --format csv
    let scan_csv = Command::new(env!("CARGO_BIN_EXE_nodewipe"))
        .args([
            "scan",
            "--format",
            "csv",
            "--no-git",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("nodewipe scan --format csv");
    assert!(scan_csv.status.success());
    let csv_out = String::from_utf8_lossy(&scan_csv.stdout);
    assert!(csv_out.starts_with("path,kind,size_bytes,"));
    assert!(csv_out.contains("node_modules"));
}

#[test]
fn test_cli_workspace_analysis_and_cache_pruning() {
    use std::fs::File;
    use std::time::{Duration, SystemTime};

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    std::fs::write(
        root.join("pnpm-workspace.yaml"),
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    )
    .unwrap();
    std::fs::write(root.join("turbo.json"), r#"{"tasks":{"build":{}}}"#).unwrap();

    let web_app = root.join("apps/web");
    let web_nm = web_app.join("node_modules");
    std::fs::create_dir_all(&web_nm).unwrap();
    std::fs::write(web_app.join("package.json"), "{}").unwrap();
    std::fs::write(web_nm.join("dep.js"), "export const x = 1;").unwrap();

    let turbo_cache = root.join(".turbo/cache");
    std::fs::create_dir_all(&turbo_cache).unwrap();
    let stale_item = turbo_cache.join("abc123-stale.tar.zst");
    let fresh_item = turbo_cache.join("def456-fresh.tar.zst");
    std::fs::write(&stale_item, "old-cached-build-artifact").unwrap();
    std::fs::write(&fresh_item, "fresh-cached-build-artifact").unwrap();

    let old_time = SystemTime::now() - Duration::from_secs(30 * 86400);
    File::options()
        .write(true)
        .open(&stale_item)
        .unwrap()
        .set_modified(old_time)
        .unwrap();

    // 1. Workspace analysis JSON
    let ws_out = Command::new(env!("CARGO_BIN_EXE_nodewipe"))
        .args(["workspace", root.to_str().unwrap(), "--json", "--no-git"])
        .output()
        .expect("nodewipe workspace --json");
    assert!(ws_out.status.success());
    let ws_json: serde_json::Value =
        serde_json::from_slice(&ws_out.stdout).expect("valid workspace analysis json");
    assert_eq!(ws_json["workspace_type"], "pnpm + Turborepo workspace");
    assert!(ws_json["resolved_member_count"].as_u64().unwrap() >= 1);
    assert!(
        !ws_json["cache_summary"].as_array().unwrap().is_empty(),
        "cache_summary must detect .turbo/cache"
    );

    // 2. Cache prune dry-run
    let dry_out = Command::new(env!("CARGO_BIN_EXE_nodewipe"))
        .args([
            "workspace",
            root.to_str().unwrap(),
            "--prune-caches",
            "--max-age",
            "14",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("nodewipe workspace --prune-caches --dry-run");
    assert!(dry_out.status.success());
    let dry_json: serde_json::Value =
        serde_json::from_slice(&dry_out.stdout).expect("valid prune dry-run json");
    assert_eq!(dry_json["pruned_count"], 1);
    assert_eq!(dry_json["skipped_fresh_count"], 1);
    assert!(stale_item.exists());

    // 3. Cache prune execute (--yes)
    let real_out = Command::new(env!("CARGO_BIN_EXE_nodewipe"))
        .args([
            "workspace",
            root.to_str().unwrap(),
            "--prune-caches",
            "--max-age",
            "14",
            "--yes",
            "--json",
        ])
        .output()
        .expect("nodewipe workspace --prune-caches --yes");
    assert!(real_out.status.success());
    let real_json: serde_json::Value =
        serde_json::from_slice(&real_out.stdout).expect("valid prune json");
    assert_eq!(real_json["pruned_count"], 1);
    assert!(!stale_item.exists(), "stale cache item must be deleted");
    assert!(fresh_item.exists(), "fresh cache item must be kept");
}
