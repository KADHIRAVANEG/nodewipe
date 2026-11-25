use crate::model::ArtifactKind;
use serde::{Deserialize, Serialize};

/// Predefined cleanup profiles that express high-level cleanup intent
/// rather than manual artifact-by-artifact filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileKind {
    /// Stale dependency directories (node_modules, venvs, vendor, etc.)
    /// that can be restored from lockfiles/registries.
    OldDependencies,
    /// Compiler, bundler, and build outputs that can be rebuilt 100% locally
    /// from source code with zero network access required.
    BuildOutputs,
    /// Heavy AI/ML model caches, checkpoint weights, and kernel compile caches.
    LargeModels,
    /// Zero-risk, offline-rebuildable artifacts with no data loss and no network downloads.
    SafeSweep,
}

impl ProfileKind {
    /// All available profile kinds.
    pub const ALL: &'static [ProfileKind] = &[
        ProfileKind::OldDependencies,
        ProfileKind::BuildOutputs,
        ProfileKind::LargeModels,
        ProfileKind::SafeSweep,
    ];

    /// Parse a profile kind from string slug (case-insensitive, kebab or snake case).
    pub fn from_slug(s: &str) -> Option<Self> {
        let norm = s.trim().to_lowercase().replace('_', "-");
        match norm.as_str() {
            "old-dependencies" | "dependencies" | "deps" => Some(Self::OldDependencies),
            "build-outputs" | "build" | "builds" => Some(Self::BuildOutputs),
            "large-models" | "models" | "ai" | "ml" => Some(Self::LargeModels),
            "safe-sweep" | "safe" => Some(Self::SafeSweep),
            _ => None,
        }
    }

    /// Primary CLI / config slug for this profile.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::OldDependencies => "old-dependencies",
            Self::BuildOutputs => "build-outputs",
            Self::LargeModels => "large-models",
            Self::SafeSweep => "safe-sweep",
        }
    }

    /// User-friendly display label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::OldDependencies => "Old Dependencies",
            Self::BuildOutputs => "Build Outputs",
            Self::LargeModels => "Large Models",
            Self::SafeSweep => "Safe Sweep",
        }
    }

    /// Descriptive explanation of what this profile cleans.
    pub fn description(&self) -> &'static str {
        match self {
            Self::OldDependencies => {
                "Stale dependency directories (>60d) from inactive projects that can be restored from lockfiles"
            }
            Self::BuildOutputs => {
                "Compiler, bundler, and build outputs that rebuild 100% locally from source code"
            }
            Self::LargeModels => {
                "Large AI/ML model weights, checkpoints, and compiler kernel caches"
            }
            Self::SafeSweep => "Zero-risk local artifacts with zero network download requirements",
        }
    }

    /// The specific artifact kinds targeted by this profile.
    pub fn target_kinds(&self) -> &'static [ArtifactKind] {
        match self {
            Self::OldDependencies => &[
                ArtifactKind::NodeModules,
                ArtifactKind::PnpmIgnored,
                ArtifactKind::YarnCache,
                ArtifactKind::PythonVenv,
                ArtifactKind::PhpVendor,
                ArtifactKind::CocoaPods,
                ArtifactKind::GoModCache,
            ],
            Self::BuildOutputs => &[
                ArtifactKind::RustTarget,
                ArtifactKind::JavaMavenTarget,
                ArtifactKind::JavaGradleBuild,
                ArtifactKind::DotnetBin,
                ArtifactKind::DotnetObj,
                ArtifactKind::GenericDist,
                ArtifactKind::NextCache,
                ArtifactKind::NuxtCache,
                ArtifactKind::SvelteKitCache,
                ArtifactKind::AstroCache,
                ArtifactKind::DocusaurusCache,
                ArtifactKind::ViteCache,
                ArtifactKind::CmakeBuild,
                ArtifactKind::ZigCache,
                ArtifactKind::ElixirBuild,
                ArtifactKind::MesonBuild,
                ArtifactKind::BazelCache,
                ArtifactKind::SwiftPmBuild,
                ArtifactKind::TurboCache,
                ArtifactKind::NxCache,
                ArtifactKind::Custom,
            ],
            Self::LargeModels => &[
                ArtifactKind::HuggingFaceCache,
                ArtifactKind::OllamaModels,
                ArtifactKind::TorchCache,
                ArtifactKind::TritonCache,
            ],
            Self::SafeSweep => &[
                ArtifactKind::RustTarget,
                ArtifactKind::JavaMavenTarget,
                ArtifactKind::JavaGradleBuild,
                ArtifactKind::DotnetBin,
                ArtifactKind::DotnetObj,
                ArtifactKind::GenericDist,
                ArtifactKind::CmakeBuild,
                ArtifactKind::ZigCache,
                ArtifactKind::ElixirBuild,
                ArtifactKind::MesonBuild,
                ArtifactKind::BazelCache,
                ArtifactKind::SwiftPmBuild,
                ArtifactKind::TurboCache,
                ArtifactKind::NxCache,
                ArtifactKind::PythonPycache,
                ArtifactKind::PythonPytestCache,
                ArtifactKind::PythonMypyCache,
                ArtifactKind::PythonRuffCache,
            ],
        }
    }

    /// Optional default minimum age in days for this profile.
    pub fn default_min_age_days(&self) -> Option<u64> {
        match self {
            Self::OldDependencies => Some(60),
            _ => None,
        }
    }

    /// Checks if a given artifact kind is included in this profile.
    pub fn includes_kind(&self, kind: ArtifactKind) -> bool {
        self.target_kinds().contains(&kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_slug_parsing() {
        assert_eq!(
            ProfileKind::from_slug("build-outputs"),
            Some(ProfileKind::BuildOutputs)
        );
        assert_eq!(
            ProfileKind::from_slug("build"),
            Some(ProfileKind::BuildOutputs)
        );
        assert_eq!(
            ProfileKind::from_slug("old_dependencies"),
            Some(ProfileKind::OldDependencies)
        );
        assert_eq!(
            ProfileKind::from_slug("deps"),
            Some(ProfileKind::OldDependencies)
        );
        assert_eq!(
            ProfileKind::from_slug("large-models"),
            Some(ProfileKind::LargeModels)
        );
        assert_eq!(ProfileKind::from_slug("ai"), Some(ProfileKind::LargeModels));
        assert_eq!(
            ProfileKind::from_slug("safe-sweep"),
            Some(ProfileKind::SafeSweep)
        );
        assert_eq!(ProfileKind::from_slug("safe"), Some(ProfileKind::SafeSweep));
        assert_eq!(ProfileKind::from_slug("unknown"), None);
    }

    #[test]
    fn test_profile_kind_membership() {
        let build = ProfileKind::BuildOutputs;
        assert!(build.includes_kind(ArtifactKind::RustTarget));
        assert!(build.includes_kind(ArtifactKind::CmakeBuild));
        assert!(!build.includes_kind(ArtifactKind::NodeModules));
        assert!(!build.includes_kind(ArtifactKind::HuggingFaceCache));

        let models = ProfileKind::LargeModels;
        assert!(models.includes_kind(ArtifactKind::HuggingFaceCache));
        assert!(models.includes_kind(ArtifactKind::OllamaModels));
        assert!(!models.includes_kind(ArtifactKind::RustTarget));

        let deps = ProfileKind::OldDependencies;
        assert!(deps.includes_kind(ArtifactKind::NodeModules));
        assert!(deps.includes_kind(ArtifactKind::PythonVenv));
        assert!(!deps.includes_kind(ArtifactKind::RustTarget));
    }
}
