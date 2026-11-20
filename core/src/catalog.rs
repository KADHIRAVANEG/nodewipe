use crate::explain::RiskLevel;
use crate::model::ArtifactKind;
use serde::{Deserialize, Serialize};

/// High-level ecosystem grouping for artifacts and tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EcosystemCategory {
    Node,
    Python,
    Rust,
    Java,
    Go,
    Dotnet,
    Php,
    AiMl,
    AppleMobile,
    ModernWeb,
    SystemsNative,
    Custom,
}

impl EcosystemCategory {
    pub fn label(&self) -> &'static str {
        match self {
            EcosystemCategory::Node => "Node.js / JavaScript",
            EcosystemCategory::Python => "Python",
            EcosystemCategory::Rust => "Rust / Cargo",
            EcosystemCategory::Java => "Java / JVM",
            EcosystemCategory::Go => "Go",
            EcosystemCategory::Dotnet => ".NET / C#",
            EcosystemCategory::Php => "PHP / Composer",
            EcosystemCategory::AiMl => "AI / Machine Learning",
            EcosystemCategory::AppleMobile => "Apple & Mobile",
            EcosystemCategory::ModernWeb => "Modern Web Frameworks",
            EcosystemCategory::SystemsNative => "Systems & Native (C/C++/Zig)",
            EcosystemCategory::Custom => "User & Team Custom",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            EcosystemCategory::Node => "\u{e718}",          // 
            EcosystemCategory::Python => "\u{e73c}",        // 
            EcosystemCategory::Rust => "\u{e7a8}",          // 
            EcosystemCategory::Java => "\u{e738}",          // 
            EcosystemCategory::Go => "\u{e627}",            // 
            EcosystemCategory::Dotnet => "\u{e77f}",        // 
            EcosystemCategory::Php => "\u{e73d}",           // 
            EcosystemCategory::AiMl => "\u{f0fa}",          // 
            EcosystemCategory::AppleMobile => "\u{e755}",   // 
            EcosystemCategory::ModernWeb => "\u{e718}",     // 
            EcosystemCategory::SystemsNative => "\u{e61e}", // 
            EcosystemCategory::Custom => "\u{f013}",        // 
        }
    }
}

/// An entry in the authoritative artifact catalog.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CatalogEntry {
    pub kind: ArtifactKind,
    pub slug: &'static str,
    pub name: &'static str,
    pub category: EcosystemCategory,
    pub default_risk: RiskLevel,
    pub description: &'static str,
    pub default_recipe: Option<&'static str>,
    pub icon: &'static str,
    pub default_dir_names: &'static [&'static str],
    pub default_markers: &'static [&'static str],
}

/// The authoritative catalog of all built-in artifact types.
pub static CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        kind: ArtifactKind::NodeModules,
        slug: "node_modules",
        name: "node_modules",
        category: EcosystemCategory::Node,
        default_risk: RiskLevel::Moderate,
        description: "Node.js package dependencies installed via npm, yarn, or pnpm.",
        default_recipe: Some("npm install"),
        icon: "\u{e718}",
        default_dir_names: &["node_modules"],
        default_markers: &["package.json"],
    },
    CatalogEntry {
        kind: ArtifactKind::PnpmIgnored,
        slug: "pnpm_ignored",
        name: "pnpm .ignored",
        category: EcosystemCategory::Node,
        default_risk: RiskLevel::Low,
        description: "Leftover npm packages archived by pnpm in node_modules/.ignored.",
        default_recipe: Some("pnpm install"),
        icon: "\u{e718}",
        default_dir_names: &[".ignored"],
        default_markers: &["pnpm-lock.yaml"],
    },
    CatalogEntry {
        kind: ArtifactKind::YarnCache,
        slug: "yarn_cache",
        name: "Yarn Cache",
        category: EcosystemCategory::Node,
        default_risk: RiskLevel::Moderate,
        description: "Global offline Yarn package tarball cache (v6).",
        default_recipe: None,
        icon: "\u{e718}",
        default_dir_names: &["v6"],
        default_markers: &["yarn", "Yarn"],
    },
    CatalogEntry {
        kind: ArtifactKind::PythonVenv,
        slug: "venv",
        name: "Python venv",
        category: EcosystemCategory::Python,
        default_risk: RiskLevel::Moderate,
        description: "Isolated Python virtual environment runtime and installed packages.",
        default_recipe: Some("python3 -m venv .venv"),
        icon: "\u{e73c}",
        default_dir_names: &["venv", ".venv"],
        default_markers: &["pyvenv.cfg"],
    },
    CatalogEntry {
        kind: ArtifactKind::PythonPycache,
        slug: "pycache",
        name: "Python __pycache__",
        category: EcosystemCategory::Python,
        default_risk: RiskLevel::Low,
        description: "Compiled Python bytecode (.pyc) directories.",
        default_recipe: None,
        icon: "\u{e73c}",
        default_dir_names: &["__pycache__"],
        default_markers: &[],
    },
    CatalogEntry {
        kind: ArtifactKind::PythonPytestCache,
        slug: "pytest_cache",
        name: ".pytest_cache",
        category: EcosystemCategory::Python,
        default_risk: RiskLevel::Low,
        description: "pytest test runner execution and failure state cache.",
        default_recipe: Some("pytest"),
        icon: "\u{e73c}",
        default_dir_names: &[".pytest_cache"],
        default_markers: &[],
    },
    CatalogEntry {
        kind: ArtifactKind::PythonMypyCache,
        slug: "mypy_cache",
        name: ".mypy_cache",
        category: EcosystemCategory::Python,
        default_risk: RiskLevel::Low,
        description: "mypy type checker incremental analysis cache.",
        default_recipe: Some("mypy ."),
        icon: "\u{e73c}",
        default_dir_names: &[".mypy_cache"],
        default_markers: &[],
    },
    CatalogEntry {
        kind: ArtifactKind::PythonRuffCache,
        slug: "ruff_cache",
        name: ".ruff_cache",
        category: EcosystemCategory::Python,
        default_risk: RiskLevel::Low,
        description: "Ruff linter and code formatter AST cache.",
        default_recipe: Some("ruff check ."),
        icon: "\u{e73c}",
        default_dir_names: &[".ruff_cache"],
        default_markers: &[],
    },
    CatalogEntry {
        kind: ArtifactKind::RustTarget,
        slug: "rust_target",
        name: "Cargo target",
        category: EcosystemCategory::Rust,
        default_risk: RiskLevel::Low,
        description: "Rust compiler build artifacts and intermediate object files.",
        default_recipe: Some("cargo build"),
        icon: "\u{e7a8}",
        default_dir_names: &["target"],
        default_markers: &["Cargo.toml"],
    },
    CatalogEntry {
        kind: ArtifactKind::JavaMavenTarget,
        slug: "maven_target",
        name: "Maven target",
        category: EcosystemCategory::Java,
        default_risk: RiskLevel::Low,
        description: "Maven compilation output and packaged JAR/WAR targets.",
        default_recipe: Some("mvn package"),
        icon: "\u{e738}",
        default_dir_names: &["target"],
        default_markers: &["pom.xml"],
    },
    CatalogEntry {
        kind: ArtifactKind::JavaGradleBuild,
        slug: "gradle_build",
        name: "Gradle build",
        category: EcosystemCategory::Java,
        default_risk: RiskLevel::Low,
        description: "Gradle project build outputs and task caches.",
        default_recipe: Some("gradle build"),
        icon: "\u{e738}",
        default_dir_names: &["build"],
        default_markers: &["build.gradle", "build.gradle.kts"],
    },
    CatalogEntry {
        kind: ArtifactKind::GradleCache,
        slug: "gradle_cache",
        name: "Gradle Cache",
        category: EcosystemCategory::Java,
        default_risk: RiskLevel::Low,
        description: "Global Gradle dependency and execution cache (~/.gradle/caches).",
        default_recipe: None,
        icon: "\u{e738}",
        default_dir_names: &["caches"],
        default_markers: &[".gradle"],
    },
    CatalogEntry {
        kind: ArtifactKind::GoModCache,
        slug: "go_mod_cache",
        name: "Go mod cache",
        category: EcosystemCategory::Go,
        default_risk: RiskLevel::Moderate,
        description: "Downloaded Go module sources in $GOPATH/pkg/mod.",
        default_recipe: Some("go mod download"),
        icon: "\u{e627}",
        default_dir_names: &["mod"],
        default_markers: &["go.mod"],
    },
    CatalogEntry {
        kind: ArtifactKind::GoBuildCache,
        slug: "go_build_cache",
        name: "Go build cache",
        category: EcosystemCategory::Go,
        default_risk: RiskLevel::Low,
        description: "Go compiler object and build cache.",
        default_recipe: Some("go build ./..."),
        icon: "\u{e627}",
        default_dir_names: &["go-build"],
        default_markers: &[],
    },
    CatalogEntry {
        kind: ArtifactKind::PhpVendor,
        slug: "php_vendor",
        name: "PHP vendor",
        category: EcosystemCategory::Php,
        default_risk: RiskLevel::Moderate,
        description: "PHP Composer packages and auto-generated classmaps.",
        default_recipe: Some("composer install"),
        icon: "\u{e73d}",
        default_dir_names: &["vendor"],
        default_markers: &["composer.json"],
    },
    CatalogEntry {
        kind: ArtifactKind::DotnetBin,
        slug: "dotnet_bin",
        name: ".NET bin/",
        category: EcosystemCategory::Dotnet,
        default_risk: RiskLevel::Low,
        description: ".NET compiled binaries and DLL outputs.",
        default_recipe: Some("dotnet build"),
        icon: "\u{e77f}",
        default_dir_names: &["bin"],
        default_markers: &[".csproj", ".fsproj"],
    },
    CatalogEntry {
        kind: ArtifactKind::DotnetObj,
        slug: "dotnet_obj",
        name: ".NET obj/",
        category: EcosystemCategory::Dotnet,
        default_risk: RiskLevel::Low,
        description: ".NET intermediate object files and NuGet restore caches.",
        default_recipe: Some("dotnet build"),
        icon: "\u{e77f}",
        default_dir_names: &["obj"],
        default_markers: &[".csproj", ".fsproj"],
    },
    CatalogEntry {
        kind: ArtifactKind::NextCache,
        slug: "next_cache",
        name: "Next.js cache",
        category: EcosystemCategory::ModernWeb,
        default_risk: RiskLevel::Low,
        description: "Next.js dev server and production webpack/turbopack cache.",
        default_recipe: Some("npm run build"),
        icon: "\u{e718}",
        default_dir_names: &[".next"],
        default_markers: &[],
    },
    CatalogEntry {
        kind: ArtifactKind::TurboCache,
        slug: "turbo_cache",
        name: "Turborepo cache",
        category: EcosystemCategory::ModernWeb,
        default_risk: RiskLevel::Low,
        description: "Turborepo task hash output cache.",
        default_recipe: Some("turbo build"),
        icon: "\u{e718}",
        default_dir_names: &[".turbo"],
        default_markers: &["turbo.json"],
    },
    CatalogEntry {
        kind: ArtifactKind::GenericDist,
        slug: "dist",
        name: "dist/ bundle",
        category: EcosystemCategory::ModernWeb,
        default_risk: RiskLevel::Low,
        description: "Generic JavaScript/TypeScript bundler distribution output.",
        default_recipe: Some("npm run build"),
        icon: "\u{e718}",
        default_dir_names: &["dist"],
        default_markers: &["package.json"],
    },
    CatalogEntry {
        kind: ArtifactKind::HuggingFaceCache,
        slug: "hf_cache",
        name: "HuggingFace cache",
        category: EcosystemCategory::AiMl,
        default_risk: RiskLevel::Caution,
        description: "Hugging Face model weights and dataset shards.",
        default_recipe: None,
        icon: "\u{f0fa}",
        default_dir_names: &["hub"],
        default_markers: &["huggingface"],
    },
    CatalogEntry {
        kind: ArtifactKind::OllamaModels,
        slug: "ollama_models",
        name: "Ollama models",
        category: EcosystemCategory::AiMl,
        default_risk: RiskLevel::Caution,
        description: "Local Ollama model weights and manifest blobs (~/.ollama/models).",
        default_recipe: None,
        icon: "\u{f0fa}",
        default_dir_names: &["models"],
        default_markers: &[".ollama"],
    },
    CatalogEntry {
        kind: ArtifactKind::TorchCache,
        slug: "torch_cache",
        name: "PyTorch cache",
        category: EcosystemCategory::AiMl,
        default_risk: RiskLevel::Low,
        description: "PyTorch kernel compilation and model checkpoint cache.",
        default_recipe: None,
        icon: "\u{f0fa}",
        default_dir_names: &["torch"],
        default_markers: &[],
    },
    CatalogEntry {
        kind: ArtifactKind::TritonCache,
        slug: "triton_cache",
        name: "Triton cache",
        category: EcosystemCategory::AiMl,
        default_risk: RiskLevel::Low,
        description: "OpenAI Triton GPU compiler kernel cache.",
        default_recipe: None,
        icon: "\u{f0fa}",
        default_dir_names: &["cache"],
        default_markers: &[".triton"],
    },
    CatalogEntry {
        kind: ArtifactKind::XcodeDerivedData,
        slug: "derived_data",
        name: "Xcode DerivedData",
        category: EcosystemCategory::AppleMobile,
        default_risk: RiskLevel::Low,
        description: "Apple Xcode build products, module caches, and index stores.",
        default_recipe: None,
        icon: "\u{e755}",
        default_dir_names: &["DerivedData"],
        default_markers: &[],
    },
    CatalogEntry {
        kind: ArtifactKind::SwiftPmBuild,
        slug: "swiftpm_build",
        name: "SwiftPM .build",
        category: EcosystemCategory::AppleMobile,
        default_risk: RiskLevel::Low,
        description: "Swift Package Manager build output (.build/).",
        default_recipe: Some("swift build"),
        icon: "\u{e755}",
        default_dir_names: &[".build"],
        default_markers: &["Package.swift"],
    },
    CatalogEntry {
        kind: ArtifactKind::CocoaPods,
        slug: "cocoapods",
        name: "CocoaPods Pods",
        category: EcosystemCategory::AppleMobile,
        default_risk: RiskLevel::Moderate,
        description: "CocoaPods dependency workspace directory (Pods/).",
        default_recipe: Some("pod install"),
        icon: "\u{e755}",
        default_dir_names: &["Pods"],
        default_markers: &["Podfile"],
    },
    CatalogEntry {
        kind: ArtifactKind::AndroidBuildCache,
        slug: "android_build_cache",
        name: "Android build cache",
        category: EcosystemCategory::AppleMobile,
        default_risk: RiskLevel::Low,
        description: "Android SDK and Gradle build cache (~/.android/build-cache).",
        default_recipe: None,
        icon: "\u{e755}",
        default_dir_names: &["build-cache"],
        default_markers: &[".android"],
    },
    CatalogEntry {
        kind: ArtifactKind::PlaywrightBrowsers,
        slug: "playwright_browsers",
        name: "Playwright browsers",
        category: EcosystemCategory::ModernWeb,
        default_risk: RiskLevel::Caution,
        description: "Downloaded Playwright test browser binaries (Chromium, Firefox, WebKit).",
        default_recipe: Some("npx playwright install"),
        icon: "\u{e718}",
        default_dir_names: &["ms-playwright"],
        default_markers: &[],
    },
    CatalogEntry {
        kind: ArtifactKind::CypressCache,
        slug: "cypress_cache",
        name: "Cypress cache",
        category: EcosystemCategory::ModernWeb,
        default_risk: RiskLevel::Moderate,
        description: "Downloaded Cypress test runner desktop application binaries.",
        default_recipe: Some("npx cypress install"),
        icon: "\u{e718}",
        default_dir_names: &["Cypress"],
        default_markers: &[],
    },
    CatalogEntry {
        kind: ArtifactKind::AstroCache,
        slug: "astro_cache",
        name: "Astro cache",
        category: EcosystemCategory::ModernWeb,
        default_risk: RiskLevel::Low,
        description: "Astro build and content layer cache (.astro/).",
        default_recipe: Some("npm run build"),
        icon: "\u{e718}",
        default_dir_names: &[".astro"],
        default_markers: &["astro.config.mjs", "astro.config.ts"],
    },
    CatalogEntry {
        kind: ArtifactKind::SvelteKitCache,
        slug: "sveltekit_cache",
        name: "SvelteKit cache",
        category: EcosystemCategory::ModernWeb,
        default_risk: RiskLevel::Low,
        description: "SvelteKit generated types and build cache (.svelte-kit/).",
        default_recipe: Some("npm run build"),
        icon: "\u{e718}",
        default_dir_names: &[".svelte-kit"],
        default_markers: &["svelte.config.js"],
    },
    CatalogEntry {
        kind: ArtifactKind::NuxtCache,
        slug: "nuxt_cache",
        name: "Nuxt cache",
        category: EcosystemCategory::ModernWeb,
        default_risk: RiskLevel::Low,
        description: "Nuxt 3 build cache and generated routes (.nuxt/).",
        default_recipe: Some("npm run build"),
        icon: "\u{e718}",
        default_dir_names: &[".nuxt"],
        default_markers: &["nuxt.config.ts"],
    },
    CatalogEntry {
        kind: ArtifactKind::DocusaurusCache,
        slug: "docusaurus_cache",
        name: "Docusaurus cache",
        category: EcosystemCategory::ModernWeb,
        default_risk: RiskLevel::Low,
        description: "Docusaurus build cache (.docusaurus/).",
        default_recipe: Some("npm run build"),
        icon: "\u{e718}",
        default_dir_names: &[".docusaurus"],
        default_markers: &["docusaurus.config.js"],
    },
    CatalogEntry {
        kind: ArtifactKind::ViteCache,
        slug: "vite_cache",
        name: "Vite cache",
        category: EcosystemCategory::ModernWeb,
        default_risk: RiskLevel::Low,
        description: "Vite pre-bundled dependency cache (.vite/).",
        default_recipe: Some("npx vite"),
        icon: "\u{e718}",
        default_dir_names: &[".vite"],
        default_markers: &["vite.config.js", "vite.config.ts"],
    },
    CatalogEntry {
        kind: ArtifactKind::CmakeBuild,
        slug: "cmake_build",
        name: "CMake / Ninja build",
        category: EcosystemCategory::SystemsNative,
        default_risk: RiskLevel::Low,
        description: "CMake and Ninja compiled object files and build directories.",
        default_recipe: Some("cmake --build build"),
        icon: "\u{e61e}",
        default_dir_names: &["build", "cmake-build-debug", "cmake-build-release"],
        default_markers: &["CMakeLists.txt", "CMakeCache.txt"],
    },
    CatalogEntry {
        kind: ArtifactKind::ZigCache,
        slug: "zig_cache",
        name: "Zig cache & build",
        category: EcosystemCategory::SystemsNative,
        default_risk: RiskLevel::Low,
        description: "Zig compiler cache (.zig-cache/) and output (zig-out/).",
        default_recipe: Some("zig build"),
        icon: "\u{e61e}",
        default_dir_names: &[".zig-cache", "zig-out"],
        default_markers: &["build.zig"],
    },
    CatalogEntry {
        kind: ArtifactKind::ElixirBuild,
        slug: "elixir_build",
        name: "Elixir / Mix build",
        category: EcosystemCategory::SystemsNative,
        default_risk: RiskLevel::Low,
        description: "Elixir Mix compiled BEAM bytecode and dependency artifacts (_build/).",
        default_recipe: Some("mix compile"),
        icon: "\u{e62d}",
        default_dir_names: &["_build"],
        default_markers: &["mix.exs"],
    },
    CatalogEntry {
        kind: ArtifactKind::MesonBuild,
        slug: "meson_build",
        name: "Meson build",
        category: EcosystemCategory::SystemsNative,
        default_risk: RiskLevel::Low,
        description: "Meson compiled build directory (builddir/).",
        default_recipe: Some("meson compile -C builddir"),
        icon: "\u{e61e}",
        default_dir_names: &["builddir"],
        default_markers: &["meson.build"],
    },
];

/// Returns a slice of all built-in catalog entries.
pub fn catalog() -> &'static [CatalogEntry] {
    CATALOG
}

/// Retrieves the catalog entry for a specific artifact kind.
pub fn get_by_kind(kind: ArtifactKind) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.kind == kind)
}

/// Retrieves a catalog entry by its CLI slug name.
pub fn get_by_slug(slug: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.slug == slug)
}

/// Retrieves a catalog entry by its display name.
pub fn get_by_name(name: &str) -> Option<&'static CatalogEntry> {
    CATALOG
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case(name) || e.slug.eq_ignore_ascii_case(name))
}

/// Filters catalog entries by an ecosystem category.
pub fn by_ecosystem(category: EcosystemCategory) -> Vec<&'static CatalogEntry> {
    CATALOG.iter().filter(|e| e.category == category).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catalog_completeness_and_lookup() {
        assert_eq!(CATALOG.len(), 39);
        for entry in CATALOG {
            assert!(!entry.slug.is_empty());
            assert!(!entry.name.is_empty());
            assert!(!entry.description.is_empty());
            assert_eq!(get_by_kind(entry.kind), Some(entry));
            assert_eq!(get_by_slug(entry.slug), Some(entry));
        }
    }

    #[test]
    fn test_ecosystem_filtering() {
        let rust_entries = by_ecosystem(EcosystemCategory::Rust);
        assert_eq!(rust_entries.len(), 1);
        assert_eq!(rust_entries[0].kind, ArtifactKind::RustTarget);

        let py_entries = by_ecosystem(EcosystemCategory::Python);
        assert_eq!(py_entries.len(), 5);
    }
}
