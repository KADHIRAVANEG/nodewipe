use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Default)]
struct UpdateState {
    last_check_unix: u64,
}

fn update_state_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("nodewipe").join("update.json"))
}

/// Tests whether a directory is writable by creating and removing a hidden temporary probe file.
/// If the directory does not exist, it recursively checks the closest existing ancestor directory.
pub fn is_dir_writable(dir: &Path) -> bool {
    if !dir.exists() {
        if let Some(parent) = dir.parent()
            && !parent.as_os_str().is_empty()
        {
            return is_dir_writable(parent);
        }
        return false;
    }

    let probe_name = format!(
        ".nodewipe_probe_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let probe_path = dir.join(probe_name);
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_path)
    {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(probe_path);
            true
        }
        Err(_) => false,
    }
}

/// Returns the user-local binary path:
/// - Unix (Linux/macOS): ~/.local/bin/nodewipe
/// - Windows: %LOCALAPPDATA%\Programs\nodewipe\nodewipe.exe (or %USERPROFILE%\.cargo\bin)
pub fn user_local_bin_path() -> Option<PathBuf> {
    if cfg!(windows) {
        dirs::data_local_dir()
            .map(|d| d.join("Programs").join("nodewipe").join("nodewipe.exe"))
            .or_else(|| dirs::home_dir().map(|h| h.join(".cargo").join("bin").join("nodewipe.exe")))
    } else {
        dirs::home_dir().map(|h| h.join(".local").join("bin").join("nodewipe"))
    }
}

/// Checks whether `dir` is present in the current process's PATH environment variable.
pub fn is_in_path(dir: &Path) -> bool {
    if let Some(path_var) = std::env::var_os("PATH") {
        let dir_canon = std::fs::canonicalize(dir).ok();
        for entry in std::env::split_paths(&path_var) {
            if entry == dir {
                return true;
            }
            if let (Some(c_entry), Some(c_dir)) = (std::fs::canonicalize(&entry).ok(), &dir_canon)
                && c_entry == *c_dir
            {
                return true;
            }
        }
    }
    false
}

/// Resolves where the updated binary should be installed:
/// - If the currently running executable's directory is writable (e.g. user-owned or running with sudo),
///   it updates that executable in place: `(current_exe, false)`.
/// - If the currently running executable is in a system read-only directory (e.g. `/usr/bin/nodewipe`),
///   it falls back to user-space `~/.local/bin/nodewipe` without requiring elevated privileges: `(local_bin, true)`.
pub fn resolve_install_target() -> (PathBuf, bool) {
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
        && is_dir_writable(parent)
    {
        return (exe, false);
    }

    if let Some(local_bin) = user_local_bin_path()
        && let Some(parent) = local_bin.parent()
        && is_dir_writable(parent)
    {
        return (local_bin, true);
    }

    // Default fallback to current_exe even if indeterminate
    (
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("nodewipe")),
        false,
    )
}

fn save_update_timestamp() -> Result<()> {
    if let Some(path) = update_state_path() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let state = UpdateState {
            last_check_unix: now,
        };
        if let Ok(content) = serde_json::to_string(&state) {
            let _ = fs::write(path, content);
        }
    }
    Ok(())
}

/// Verifies that a downloaded binary has a valid executable header format.
fn verify_downloaded_binary(bin_path: &Path) -> Result<(), self_update::errors::Error> {
    use std::io::Read;
    let mut file = std::fs::File::open(bin_path).map_err(|e| {
        self_update::errors::Error::verification_rejected(format!(
            "failed to open binary for verification: {e}"
        ))
    })?;
    let mut magic = [0u8; 4];
    let bytes_read = file.read(&mut magic).map_err(|e| {
        self_update::errors::Error::verification_rejected(format!(
            "failed to read binary header: {e}"
        ))
    })?;
    if bytes_read < 4 {
        return Err(self_update::errors::Error::verification_rejected(
            "downloaded binary is truncated (less than 4 bytes)",
        ));
    }

    // Verify executable magic numbers:
    // Linux ELF: 0x7f 'E' 'L' 'F'
    // Windows PE: 'M' 'Z'
    // macOS Mach-O (32-bit, 64-bit, FAT, little/big endian)
    let is_elf = &magic == b"\x7fELF";
    let is_pe = &magic[0..2] == b"MZ";
    let is_macho = matches!(
        magic,
        [0xfe, 0xed, 0xfa, 0xce]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xce, 0xfa, 0xed, 0xfe]
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xbe, 0xba, 0xfe, 0xca]
    );

    if !is_elf && !is_pe && !is_macho {
        return Err(self_update::errors::Error::verification_rejected(
            "downloaded asset is not a valid executable binary format",
        ));
    }

    Ok(())
}

/// Explicit command invoked via `nodewipe update [--check] [--yes] [--force]`.
pub fn run_update(check: bool, yes: bool, force: bool) -> Result<ExitCode> {
    println!("Checking for updates...");
    let current_version = env!("CARGO_PKG_VERSION");
    let (target_path, is_fallback) = resolve_install_target();

    let mut config_builder = self_update::backends::github::Update::configure();
    config_builder
        .repo_owner("KADHIRAVANEG")
        .repo_name("nodewipe")
        .bin_name(if cfg!(windows) {
            "nodewipe.exe"
        } else {
            "nodewipe"
        })
        .bin_install_path(&target_path)
        .current_version(current_version)
        .no_confirm(true)
        .verify_release_digest(true)
        .verify_binary(verify_downloaded_binary)
        .check_install_path_writable(true);

    let builder = config_builder.build()?;
    let releases = builder.get_latest_release()?;
    let Some(latest) = releases.latest() else {
        println!("No releases found on GitHub.");
        return Ok(ExitCode::SUCCESS);
    };

    let is_newer =
        self_update::version::bump_is_greater(current_version, latest.version()).unwrap_or(false);

    if check {
        if is_newer {
            println!(
                "A new version of nodewipe is available: v{} -> v{}",
                current_version,
                latest.version()
            );
            println!("Run 'nodewipe update' to install the latest version.");
        } else {
            println!("nodewipe is already up to date (v{}).", current_version);
        }
        return Ok(ExitCode::SUCCESS);
    }

    if !is_newer && !force {
        println!("nodewipe is already up to date (v{}).", current_version);
        return Ok(ExitCode::SUCCESS);
    }

    println!(
        "Updating nodewipe from v{} to v{}...",
        current_version,
        latest.version()
    );

    if is_fallback {
        println!(
            "Notice: System binary is read-only. Installing update to user-space: {} (no sudo required).",
            target_path.display()
        );
    } else {
        println!("Installing update to: {}", target_path.display());
    }

    if !yes {
        use std::io::Write;
        print!("Proceed with update? [Y/n]: ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case("n") {
            println!("Update cancelled.");
            return Ok(ExitCode::SUCCESS);
        }
    }

    if let Some(parent) = target_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    println!("Downloading release assets...");
    match builder.update() {
        Ok(status) => {
            if is_fallback {
                println!(
                    "Successfully installed nodewipe v{} to {} (no sudo required)!",
                    status.version(),
                    target_path.display()
                );
                if let Some(parent) = target_path.parent()
                    && !is_in_path(parent)
                {
                    println!(
                        "Tip: Add {} to your PATH to run it directly:",
                        parent.display()
                    );
                    println!("   export PATH=\"{}:$PATH\"", parent.display());
                }
            } else {
                println!("Successfully updated nodewipe to v{}!", status.version());
            }
            let _ = save_update_timestamp();
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => {
            eprintln!("Failed to update: {e}");
            eprintln!("Tip: You can manually update using the installer:");
            eprintln!(
                "  curl -fsSL https://raw.githubusercontent.com/KADHIRAVANEG/nodewipe/main/install.sh | bash"
            );
            Ok(ExitCode::FAILURE)
        }
    }
}

/// Automatic background check run once per 24 hours in interactive mode.
pub fn check_and_prompt_update() -> Result<()> {
    // Only check once every 24 hours to avoid slowing down the CLI
    let state_file = update_state_path();
    let mut state = UpdateState::default();

    if let Some(path) = &state_file
        && let Ok(content) = fs::read_to_string(path)
        && let Ok(parsed) = serde_json::from_str::<UpdateState>(&content)
    {
        state = parsed;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now.saturating_sub(state.last_check_unix) < 86400 {
        return Ok(());
    }

    // Save timestamp immediately to avoid repeated checks if network fails or update is declined
    let _ = save_update_timestamp();

    let current_version = env!("CARGO_PKG_VERSION");
    let (target_path, is_fallback) = resolve_install_target();

    let builder = self_update::backends::github::Update::configure()
        .repo_owner("KADHIRAVANEG")
        .repo_name("nodewipe")
        .bin_name(if cfg!(windows) {
            "nodewipe.exe"
        } else {
            "nodewipe"
        })
        .bin_install_path(&target_path)
        .current_version(current_version)
        .no_confirm(true)
        .verify_release_digest(true)
        .verify_binary(verify_downloaded_binary)
        .check_install_path_writable(true)
        .build()?;

    let releases = builder.get_latest_release()?;
    if let Some(latest) = releases.latest() {
        let is_newer = self_update::version::bump_is_greater(current_version, latest.version())
            .unwrap_or(false);
        if is_newer {
            println!(
                "A new version of nodewipe is available! ({} -> {})",
                current_version,
                latest.version()
            );
            if is_fallback {
                println!(
                    "   System binary is read-only. Update will install to user-space: {} (no sudo required).",
                    target_path.display()
                );
            }
            use std::io::Write;
            print!("Would you like to update now? [y/N]: ");
            std::io::stdout().flush()?;

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;

            if input.trim().eq_ignore_ascii_case("y") {
                if let Some(parent) = target_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                println!("Downloading update...");
                match builder.update() {
                    Ok(status) => {
                        if is_fallback {
                            println!(
                                "Successfully installed nodewipe v{} to {} (no sudo required)!",
                                status.version(),
                                target_path.display()
                            );
                            if let Some(parent) = target_path.parent()
                                && !is_in_path(parent)
                            {
                                println!(
                                    "Tip: Add {} to your PATH: export PATH=\"{}:$PATH\"",
                                    parent.display(),
                                    parent.display()
                                );
                            }
                        } else {
                            println!("Successfully updated to v{}!", status.version());
                        }
                        std::process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("Failed to update: {}", e);
                        eprintln!("Tip: You can manually update using:");
                        eprintln!(
                            "  curl -fsSL https://raw.githubusercontent.com/KADHIRAVANEG/nodewipe/main/install.sh | bash"
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_dir_writable_on_temp_dir() {
        let temp = tempfile::tempdir().expect("create temp dir");
        assert!(is_dir_writable(temp.path()));

        let nested = temp.path().join("nested").join("subfolder");
        assert!(is_dir_writable(&nested));
    }

    #[test]
    fn test_user_local_bin_path_structure() {
        let local_bin = user_local_bin_path();
        assert!(local_bin.is_some());
        let path = local_bin.unwrap();
        if cfg!(windows) {
            assert!(path.to_string_lossy().ends_with("nodewipe.exe"));
        } else {
            assert!(path.to_string_lossy().ends_with("nodewipe"));
            assert!(path.to_string_lossy().contains(".local"));
        }
    }

    #[test]
    fn test_resolve_install_target_returns_valid_path() {
        let (target, is_fallback) = resolve_install_target();
        assert!(!target.as_os_str().is_empty());
        let file_name = target.file_name().expect("file name").to_string_lossy();
        assert!(file_name.starts_with("nodewipe"));
        // If not writable in current_exe parent, is_fallback should be true
        let is_parent_unwritable = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| !is_dir_writable(p)))
            .unwrap_or(false);
        if is_parent_unwritable {
            assert!(is_fallback);
        }
    }

    #[test]
    fn test_is_in_path_evaluates_system_path() {
        if let Some(first) = std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).next())
        {
            assert!(is_in_path(&first));
        }
        let fake_path = PathBuf::from("/nonexistent_path_definitely_not_in_env_path_12345");
        assert!(!is_in_path(&fake_path));
    }

    #[test]
    fn test_verify_downloaded_binary_format() {
        let temp = tempfile::tempdir().expect("tempdir");

        // Valid ELF
        let elf_path = temp.path().join("fake_elf");
        fs::write(&elf_path, b"\x7fELFsome_binary_payload").unwrap();
        assert!(verify_downloaded_binary(&elf_path).is_ok());

        // Valid PE
        let pe_path = temp.path().join("fake_pe.exe");
        fs::write(&pe_path, b"MZ\x90\x00some_windows_payload").unwrap();
        assert!(verify_downloaded_binary(&pe_path).is_ok());

        // Valid Mach-O
        let macho_path = temp.path().join("fake_macho");
        fs::write(&macho_path, [0xcf, 0xfa, 0xed, 0xfe, 0x01, 0x02]).unwrap();
        assert!(verify_downloaded_binary(&macho_path).is_ok());

        // Invalid: shell script or HTML
        let script_path = temp.path().join("evil.sh");
        fs::write(&script_path, b"#!/bin/sh\nrm -rf /").unwrap();
        assert!(verify_downloaded_binary(&script_path).is_err());

        // Invalid: empty or truncated
        let empty_path = temp.path().join("empty");
        fs::write(&empty_path, b"").unwrap();
        assert!(verify_downloaded_binary(&empty_path).is_err());
    }
}
