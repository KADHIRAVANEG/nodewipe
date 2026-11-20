use crate::git::inspect_git_repo;
use crate::model::{ArtifactKind, Entry, GitStatus};
use crate::pin::is_path_pinned;
use crate::rebuild::{RebuildPlan, resolve_rebuild_plan};
use crate::scanner::{classify_path, detect_package_manager};
use crate::stale::youngest_source_file_mtime;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Risk classification for cleaning an artifact directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// Safe to delete; 100% regenerable local cache or build output (e.g. Rust target/, pycache, next/cache).
    Low,
    /// Safe to delete but requires network download to restore (e.g. node_modules, venv, vendor).
    Moderate,
    /// High bandwidth or long download (e.g. Ollama models, HuggingFace weights, Playwright browsers).
    Caution,
}

impl RiskLevel {
    pub fn label(&self) -> &'static str {
        match self {
            RiskLevel::Low => "Low (Safe to delete, rebuilds locally)",
            RiskLevel::Moderate => "Moderate (Safe to delete, requires re-download)",
            RiskLevel::Caution => "Caution (Large download or bandwidth required)",
        }
    }
}

/// Source code activity and staleness analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceStaleness {
    pub stale_days: u64,
    pub is_abandoned: bool,
    pub description: String,
}

/// Comprehensive explanation of an artifact directory to help developers make confident decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Explanation {
    pub path: PathBuf,
    pub project_root: Option<PathBuf>,
    pub artifact_kind: ArtifactKind,
    pub artifact_label: String,
    pub size_bytes: u64,
    pub formatted_size: String,
    pub match_reason: String,
    pub source_staleness: Option<SourceStaleness>,
    pub git_status: Option<GitStatus>,
    pub rebuild_plan: Option<RebuildPlan>,
    pub risk_level: RiskLevel,
    pub risk_description: String,
    pub is_pinned: bool,
}

pub fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    if bytes >= TIB {
        format!("{:.2} TB", bytes as f64 / TIB as f64)
    } else if bytes >= GIB {
        format!("{:.2} GB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.0} KB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Analyzes an artifact and generates detection reason, risk level, and risk description.
pub fn analyze_artifact_rule(kind: ArtifactKind, path: &Path) -> (String, RiskLevel, String) {
    let parent = path.parent();

    match kind {
        ArtifactKind::RustTarget => (
            "Detected as Rust compiler build output because 'Cargo.toml' exists in the project root.".into(),
            RiskLevel::Low,
            "Zero data loss risk. Recreated locally by running 'cargo build' (no network download needed).".into(),
        ),
        ArtifactKind::NodeModules => {
            let pm_name = if parent.map(|p| p.join("pnpm-lock.yaml").exists()).unwrap_or(false) {
                "pnpm"
            } else if parent.map(|p| p.join("yarn.lock").exists()).unwrap_or(false) {
                "yarn"
            } else if parent.map(|p| p.join("bun.lockb").exists() || p.join("bun.lock").exists()).unwrap_or(false) {
                "bun"
            } else {
                "npm"
            };
            (
                format!("Detected as JavaScript/TypeScript dependencies ({pm_name}) because 'package.json' exists in the project root."),
                RiskLevel::Moderate,
                format!("Safe to delete. Reinstalled from registry using your lockfile via '{pm_name} install'."),
            )
        }
        ArtifactKind::PythonVenv => (
            "Detected as Python virtual environment confirmed by 'pyvenv.cfg' inside the directory.".into(),
            RiskLevel::Moderate,
            "Safe to delete. Recreated by creating a new venv and installing dependencies (e.g. 'uv sync' or 'pip install -r requirements.txt').".into(),
        ),
        ArtifactKind::PythonPycache => (
            "Detected as Python compiled bytecode cache (__pycache__).".into(),
            RiskLevel::Low,
            "Zero data loss risk. Python will automatically regenerate bytecode on the next run.".into(),
        ),
        ArtifactKind::PythonPytestCache | ArtifactKind::PythonMypyCache | ArtifactKind::PythonRuffCache => (
            format!("Detected as Python developer tooling cache ({})", kind.label()),
            RiskLevel::Low,
            "Zero data loss risk. Automatically repopulated on the next test or lint run.".into(),
        ),
        ArtifactKind::JavaMavenTarget => (
            "Detected as Apache Maven build artifacts because 'pom.xml' exists in the project root.".into(),
            RiskLevel::Low,
            "Zero data loss risk. Recreated locally by running 'mvn package' or './mvnw package'.".into(),
        ),
        ArtifactKind::JavaGradleBuild => (
            "Detected as Gradle build output because 'build.gradle' or 'build.gradle.kts' exists in the project root.".into(),
            RiskLevel::Low,
            "Zero data loss risk. Recreated locally by running 'gradle build' or './gradlew build'.".into(),
        ),
        ArtifactKind::NextCache => (
            "Detected as Next.js build cache (.next/cache) alongside project configuration.".into(),
            RiskLevel::Low,
            "Zero data loss risk. Next.js will regenerate compiled pages and server chunks on next build.".into(),
        ),
        ArtifactKind::TurboCache => (
            "Detected as Turborepo local task execution cache (.turbo).".into(),
            RiskLevel::Low,
            "Zero data loss risk. Turborepo will recompute task outputs when rerun.".into(),
        ),
        ArtifactKind::GenericDist => (
            "Detected as bundler distribution output (dist) beside a project package manifest.".into(),
            RiskLevel::Low,
            "Zero data loss risk. Rebuilt from project source code using your build script.".into(),
        ),
        ArtifactKind::OllamaModels => (
            "Detected as local Ollama large language model weights and manifests (~/.ollama/models).".into(),
            RiskLevel::Caution,
            "Caution: Deleting will remove local LLM model weights. Re-downloading models will require network bandwidth.".into(),
        ),
        ArtifactKind::HuggingFaceCache => (
            "Detected as Hugging Face Hub model weights and dataset cache (~/.cache/huggingface).".into(),
            RiskLevel::Caution,
            "Caution: Deleting will remove downloaded model weights and dataset snapshots.".into(),
        ),
        ArtifactKind::PlaywrightBrowsers => (
            "Detected as Playwright downloaded browser binaries (~/.cache/ms-playwright).".into(),
            RiskLevel::Caution,
            "Caution: Playwright test runners will need to re-download browser binaries ('npx playwright install').".into(),
        ),
        ArtifactKind::CypressCache => (
            "Detected as Cypress test runner binary cache (~/.cache/Cypress).".into(),
            RiskLevel::Caution,
            "Caution: Cypress will need to re-download binary runners ('npx cypress install').".into(),
        ),
        ArtifactKind::DotnetBin | ArtifactKind::DotnetObj => (
            "Detected as .NET compiler build output or intermediate object files.".into(),
            RiskLevel::Low,
            "Zero data loss risk. Recompiled locally via 'dotnet build'.".into(),
        ),
        ArtifactKind::CmakeBuild | ArtifactKind::ZigCache | ArtifactKind::ElixirBuild | ArtifactKind::MesonBuild => (
            format!("Detected as compiled native build artifacts ({})", kind.label()),
            RiskLevel::Low,
            "Zero data loss risk. Recompiled locally using your native build toolchain.".into(),
        ),
        _ => (
            format!("Detected as disposable developer artifact ({})", kind.label()),
            RiskLevel::Moderate,
            "Safe to delete. Can be regenerated from project source files or dependencies.".into(),
        ),
    }
}

/// Generates a comprehensive explanation for an already-scanned `Entry`.
pub fn explain_entry(entry: &Entry) -> Explanation {
    let project_root = entry.path.parent().map(|p| p.to_path_buf());
    let (match_reason, risk_level, risk_desc) = if let Some(rule) = &entry.custom_rule {
        let reason = if let Some(pm) = &rule.parent_marker {
            format!(
                "Detected by custom rule '{}' matching parent marker '{}'",
                rule.name, pm
            )
        } else if let Some(sm) = &rule.self_marker {
            format!(
                "Detected by custom rule '{}' matching self marker '{}'",
                rule.name, sm
            )
        } else {
            format!(
                "Detected by custom rule '{}' matching directory name '{}'",
                rule.name, rule.dir_name
            )
        };
        let desc = rule.description.clone().unwrap_or_else(|| {
            format!("Custom artifact rule '{}' ({}).", rule.name, rule.ecosystem)
        });
        (reason, rule.risk_level(), desc)
    } else {
        analyze_artifact_rule(entry.kind, &entry.path)
    };
    let rebuild_plan = resolve_rebuild_plan(entry);
    let is_pinned = is_path_pinned(&entry.path);

    let source_staleness = project_root.as_ref().and_then(|root| {
        youngest_source_file_mtime(root, &entry.path).map(|mtime| {
            let now = SystemTime::now();
            let age = now.duration_since(mtime).unwrap_or(Duration::ZERO);
            let stale_days = age.as_secs() / 86400;
            let description = if stale_days >= 60 {
                format!("Source code last touched {stale_days} days ago (likely abandoned project)")
            } else if stale_days >= 14 {
                format!("Source code last touched {stale_days} days ago (inactive project)")
            } else if stale_days > 0 {
                format!("Source code touched {stale_days} days ago (moderately active)")
            } else {
                "Source code touched today (actively developing project)".to_string()
            };
            SourceStaleness {
                stale_days,
                is_abandoned: stale_days >= 30,
                description,
            }
        })
    });

    let git_status = entry.git_status.clone().or_else(|| {
        project_root
            .as_ref()
            .and_then(|root| inspect_git_repo(root))
    });

    Explanation {
        path: entry.path.clone(),
        project_root,
        artifact_kind: entry.kind,
        artifact_label: entry.display_label().to_string(),
        size_bytes: entry.size_bytes,
        formatted_size: format_size(entry.size_bytes),
        match_reason,
        source_staleness,
        git_status,
        rebuild_plan,
        risk_level,
        risk_description: risk_desc,
        is_pinned,
    }
}

/// Explains an arbitrary filesystem path.
pub fn explain_path(target: &Path) -> Result<Explanation> {
    let canonical = fs::canonicalize(target).unwrap_or_else(|_| {
        if target.is_relative() {
            std::env::current_dir()
                .map(|cwd| cwd.join(target))
                .unwrap_or_else(|_| target.to_path_buf())
        } else {
            target.to_path_buf()
        }
    });

    if !canonical.exists() {
        anyhow::bail!("Path does not exist: {}", canonical.display());
    }

    // 1. Direct artifact check
    if let Some(kind) = classify_path(&canonical) {
        let pm = if kind == ArtifactKind::NodeModules {
            Some(detect_package_manager(&canonical))
        } else {
            None
        };
        let size = compute_dir_size(&canonical);
        let custom_rule = if kind == ArtifactKind::Custom {
            crate::custom_rules::load_active_rules(canonical.parent().unwrap_or(&canonical))
                .into_iter()
                .find(|r| r.matches_path(&canonical))
        } else {
            None
        };
        let entry = Entry {
            path: canonical,
            size_bytes: size,
            kind,
            package_manager: pm,
            last_modified: fs::metadata(target).ok().and_then(|m| m.modified().ok()),
            workspace_root: None,
            git_status: None,
            custom_rule,
        };
        return Ok(explain_entry(&entry));
    }

    // 2. Check if the target is a project directory containing known artifacts
    if canonical.is_dir()
        && let Ok(entries) = fs::read_dir(&canonical)
    {
        for child in entries.flatten() {
            let child_path = child.path();
            if child_path.is_dir()
                && let Some(kind) = classify_path(&child_path)
            {
                let pm = if kind == ArtifactKind::NodeModules {
                    Some(detect_package_manager(&child_path))
                } else {
                    None
                };
                let size = compute_dir_size(&child_path);
                let custom_rule = if kind == ArtifactKind::Custom {
                    crate::custom_rules::load_active_rules(&canonical)
                        .into_iter()
                        .find(|r| r.matches_path(&child_path))
                } else {
                    None
                };
                let entry = Entry {
                    path: child_path,
                    size_bytes: size,
                    kind,
                    package_manager: pm,
                    last_modified: child.metadata().ok().and_then(|m| m.modified().ok()),
                    workspace_root: None,
                    git_status: None,
                    custom_rule,
                };
                return Ok(explain_entry(&entry));
            }
        }
    }

    anyhow::bail!(
        "Directory '{}' is not recognized as a disposable artifact. Supported artifacts include node_modules, target, venv, .cache, etc.",
        canonical.display()
    );
}

fn compute_dir_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .same_file_system(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_explain_rust_target() {
        let tmp = tempdir().expect("tempdir");
        let project_dir = tmp.path().join("rust-project");
        let target_dir = project_dir.join("target");
        fs::create_dir_all(&target_dir).expect("create target");
        fs::write(project_dir.join("Cargo.toml"), "[package]\nname = \"foo\"").expect("Cargo.toml");
        fs::write(target_dir.join("dummy.bin"), "12345678").expect("dummy file");

        let explanation = explain_path(&target_dir).expect("explain");
        assert_eq!(explanation.artifact_kind, ArtifactKind::RustTarget);
        assert_eq!(explanation.risk_level, RiskLevel::Low);
        assert!(explanation.rebuild_plan.is_some());
        assert_eq!(explanation.rebuild_plan.unwrap().description, "cargo build");
        assert!(explanation.match_reason.contains("Cargo.toml"));
    }

    #[test]
    fn test_explain_node_modules() {
        let tmp = tempdir().expect("tempdir");
        let project_dir = tmp.path().join("js-project");
        let nm_dir = project_dir.join("node_modules");
        fs::create_dir_all(&nm_dir).expect("create nm");
        fs::write(project_dir.join("package.json"), "{}").expect("package.json");

        let explanation = explain_path(&nm_dir).expect("explain");
        assert_eq!(explanation.artifact_kind, ArtifactKind::NodeModules);
        assert_eq!(explanation.risk_level, RiskLevel::Moderate);
        assert!(explanation.match_reason.contains("package.json"));
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(10 * 1024 * 1024), "10.0 MB");
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.00 GB");
    }
}
