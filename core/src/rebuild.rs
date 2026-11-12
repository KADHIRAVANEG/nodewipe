use crate::model::{ArtifactKind, Entry, PackageManager};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// A planned toolchain rebuild command to regenerate dev-dependencies or build output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebuildPlan {
    /// Executable binary to invoke (e.g. "pnpm", "cargo", "npm", "python3").
    pub program: String,
    /// Arguments to pass to the executable (e.g. ["install"], ["build"]).
    pub args: Vec<String>,
    /// Working directory where the command should be executed (project root).
    pub work_dir: PathBuf,
    /// Human-friendly description of the rebuild operation (e.g. "cargo build").
    pub description: String,
}

/// Resolves the appropriate toolchain reinstall/rebuild plan for a given artifact entry.
pub fn resolve_rebuild_plan(entry: &Entry) -> Option<RebuildPlan> {
    let work_dir = determine_project_root(&entry.path, entry.workspace_root.as_deref())?;

    match entry.kind {
        ArtifactKind::NodeModules => resolve_node_rebuild(&work_dir, entry.package_manager),
        ArtifactKind::RustTarget => {
            if work_dir.join("Cargo.toml").exists() {
                Some(RebuildPlan {
                    program: "cargo".into(),
                    args: vec!["build".into()],
                    work_dir,
                    description: "cargo build".into(),
                })
            } else {
                None
            }
        }
        ArtifactKind::PythonVenv
        | ArtifactKind::PythonPycache
        | ArtifactKind::PythonPytestCache
        | ArtifactKind::PythonMypyCache
        | ArtifactKind::PythonRuffCache => resolve_python_rebuild(&work_dir),
        ArtifactKind::JavaMavenTarget => {
            if work_dir.join("mvnw").exists() {
                Some(RebuildPlan {
                    program: "./mvnw".into(),
                    args: vec!["package".into()],
                    work_dir,
                    description: "./mvnw package".into(),
                })
            } else if work_dir.join("pom.xml").exists() {
                Some(RebuildPlan {
                    program: "mvn".into(),
                    args: vec!["package".into()],
                    work_dir,
                    description: "mvn package".into(),
                })
            } else {
                None
            }
        }
        ArtifactKind::JavaGradleBuild => {
            if work_dir.join("gradlew").exists() {
                Some(RebuildPlan {
                    program: "./gradlew".into(),
                    args: vec!["build".into()],
                    work_dir,
                    description: "./gradlew build".into(),
                })
            } else if work_dir.join("build.gradle").exists()
                || work_dir.join("build.gradle.kts").exists()
            {
                Some(RebuildPlan {
                    program: "gradle".into(),
                    args: vec!["build".into()],
                    work_dir,
                    description: "gradle build".into(),
                })
            } else {
                None
            }
        }
        ArtifactKind::GoModCache | ArtifactKind::GoBuildCache => {
            if work_dir.join("go.mod").exists() {
                Some(RebuildPlan {
                    program: "go".into(),
                    args: vec!["build".into(), "./...".into()],
                    work_dir,
                    description: "go build ./...".into(),
                })
            } else {
                None
            }
        }
        ArtifactKind::PhpVendor => {
            if work_dir.join("composer.json").exists() {
                Some(RebuildPlan {
                    program: "composer".into(),
                    args: vec!["install".into()],
                    work_dir,
                    description: "composer install".into(),
                })
            } else {
                None
            }
        }
        ArtifactKind::DotnetBin | ArtifactKind::DotnetObj => Some(RebuildPlan {
            program: "dotnet".into(),
            args: vec!["build".into()],
            work_dir,
            description: "dotnet build".into(),
        }),
        ArtifactKind::SwiftPmBuild => {
            if work_dir.join("Package.swift").exists() {
                Some(RebuildPlan {
                    program: "swift".into(),
                    args: vec!["build".into()],
                    work_dir,
                    description: "swift build".into(),
                })
            } else {
                None
            }
        }
        ArtifactKind::CocoaPods => {
            if work_dir.join("Podfile").exists() {
                Some(RebuildPlan {
                    program: "pod".into(),
                    args: vec!["install".into()],
                    work_dir,
                    description: "pod install".into(),
                })
            } else {
                None
            }
        }
        ArtifactKind::CmakeBuild => Some(RebuildPlan {
            program: "cmake".into(),
            args: vec!["--build".into(), ".".into()],
            work_dir,
            description: "cmake --build .".into(),
        }),
        ArtifactKind::ZigCache => {
            if work_dir.join("build.zig").exists() {
                Some(RebuildPlan {
                    program: "zig".into(),
                    args: vec!["build".into()],
                    work_dir,
                    description: "zig build".into(),
                })
            } else {
                None
            }
        }
        ArtifactKind::ElixirBuild => {
            if work_dir.join("mix.exs").exists() {
                Some(RebuildPlan {
                    program: "mix".into(),
                    args: vec!["compile".into()],
                    work_dir,
                    description: "mix compile".into(),
                })
            } else {
                None
            }
        }
        ArtifactKind::MesonBuild => {
            if work_dir.join("meson.build").exists() {
                Some(RebuildPlan {
                    program: "meson".into(),
                    args: vec!["compile".into(), "-C".into(), "builddir".into()],
                    work_dir,
                    description: "meson compile -C builddir".into(),
                })
            } else {
                None
            }
        }
        ArtifactKind::NextCache
        | ArtifactKind::TurboCache
        | ArtifactKind::AstroCache
        | ArtifactKind::SvelteKitCache
        | ArtifactKind::NuxtCache
        | ArtifactKind::ViteCache
        | ArtifactKind::DocusaurusCache
        | ArtifactKind::GenericDist => {
            if work_dir.join("package.json").exists() {
                Some(RebuildPlan {
                    program: "npm".into(),
                    args: vec!["run".into(), "build".into()],
                    work_dir,
                    description: "npm run build".into(),
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

fn determine_project_root(artifact_path: &Path, workspace_root: Option<&Path>) -> Option<PathBuf> {
    let parent = artifact_path.parent()?;
    if let Some(ws) = workspace_root {
        // If the immediate parent contains standard build manifests, use it; otherwise use workspace root
        if parent.join("package.json").exists()
            || parent.join("Cargo.toml").exists()
            || parent.join("pyproject.toml").exists()
        {
            Some(parent.to_path_buf())
        } else {
            Some(ws.to_path_buf())
        }
    } else {
        Some(parent.to_path_buf())
    }
}

fn resolve_node_rebuild(work_dir: &Path, pm_hint: Option<PackageManager>) -> Option<RebuildPlan> {
    if work_dir.join("pnpm-lock.yaml").exists() || pm_hint == Some(PackageManager::Pnpm) {
        Some(RebuildPlan {
            program: "pnpm".into(),
            args: vec!["install".into()],
            work_dir: work_dir.to_path_buf(),
            description: "pnpm install".into(),
        })
    } else if work_dir.join("yarn.lock").exists() || pm_hint == Some(PackageManager::Yarn) {
        Some(RebuildPlan {
            program: "yarn".into(),
            args: vec!["install".into()],
            work_dir: work_dir.to_path_buf(),
            description: "yarn install".into(),
        })
    } else if work_dir.join("bun.lockb").exists() || work_dir.join("bun.lock").exists() {
        Some(RebuildPlan {
            program: "bun".into(),
            args: vec!["install".into()],
            work_dir: work_dir.to_path_buf(),
            description: "bun install".into(),
        })
    } else if work_dir.join("package.json").exists() || pm_hint == Some(PackageManager::Npm) {
        Some(RebuildPlan {
            program: "npm".into(),
            args: vec!["install".into()],
            work_dir: work_dir.to_path_buf(),
            description: "npm install".into(),
        })
    } else {
        None
    }
}

fn resolve_python_rebuild(work_dir: &Path) -> Option<RebuildPlan> {
    if work_dir.join("uv.lock").exists() {
        Some(RebuildPlan {
            program: "uv".into(),
            args: vec!["sync".into()],
            work_dir: work_dir.to_path_buf(),
            description: "uv sync".into(),
        })
    } else if work_dir.join("poetry.lock").exists() {
        Some(RebuildPlan {
            program: "poetry".into(),
            args: vec!["install".into()],
            work_dir: work_dir.to_path_buf(),
            description: "poetry install".into(),
        })
    } else if work_dir.join("Pipfile").exists() {
        Some(RebuildPlan {
            program: "pipenv".into(),
            args: vec!["install".into()],
            work_dir: work_dir.to_path_buf(),
            description: "pipenv install".into(),
        })
    } else if work_dir.join("requirements.txt").exists() || work_dir.join("pyproject.toml").exists()
    {
        Some(RebuildPlan {
            program: "python3".into(),
            args: vec!["-m".into(), "venv".into(), ".venv".into()],
            work_dir: work_dir.to_path_buf(),
            description: "python3 -m venv .venv".into(),
        })
    } else {
        None
    }
}
