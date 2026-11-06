use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Default)]
struct UpdateState {
    last_check_unix: u64,
}

fn update_state_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("nodewipe").join("update.json"))
}

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
        .unwrap()
        .as_secs();
    if now.saturating_sub(state.last_check_unix) < 86400 {
        return Ok(());
    }

    // Perform check
    let current_version = env!("CARGO_PKG_VERSION");
    let builder = self_update::backends::github::Update::configure()
        .repo_owner("KADHIRAVANEG")
        .repo_name("nodewipe")
        .bin_name(if cfg!(windows) {
            "nodewipe.exe"
        } else {
            "nodewipe"
        })
        .current_version(current_version)
        .build()?;

    let releases = builder.get_latest_release()?;
    if let Some(latest) = releases.latest() {
        let is_newer = self_update::version::bump_is_greater(current_version, latest.version())
            .unwrap_or(false);
        if is_newer {
            println!(
                "✨ A new version of nodewipe is available! ({} -> {})",
                current_version,
                latest.version()
            );
            use std::io::Write;
            print!("Would you like to update now? [y/N]: ");
            std::io::stdout().flush()?;

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;

            if input.trim().eq_ignore_ascii_case("y") {
                println!("Downloading update...");
                match builder.update() {
                    Ok(status) => {
                        println!("Successfully updated to v{}!", status.version());
                        std::process::exit(0);
                    }
                    Err(e) => eprintln!("Failed to update: {}", e),
                }
            }
        }
    }

    // Save timestamp
    if let Some(path) = &state_file {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        state.last_check_unix = now;
        if let Ok(content) = serde_json::to_string(&state) {
            fs::write(path, content)?;
        }
    }

    Ok(())
}
