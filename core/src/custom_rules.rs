use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static RULES_FILE_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Sets an explicit override for the global rules file location.
pub fn set_global_rules_override(path: PathBuf) {
    let _ = RULES_FILE_OVERRIDE.set(path);
}

fn default_low_risk() -> String {
    "low".to_string()
}

fn default_custom_ecosystem() -> String {
    "Custom".to_string()
}

/// Standard allowed build tool programs that can be safely invoked via custom rule recipes.
pub const ALLOWED_REBUILD_PROGRAMS: &[&str] = &[
    "cargo", "npm", "pnpm", "yarn", "bun", "npx", "pip", "pip3", "poetry", "uv", "pipenv",
    "mvn", "mvnw", "gradle", "gradlew", "go", "zig", "mix", "meson", "cmake", "swift",
    "pod", "composer", "dotnet", "mkdocs", "make", "ninja", "turbo", "pytest", "mypy", "ruff",
];

/// Forbidden argument flags that enable inline or interactive code execution.
pub const FORBIDDEN_RECIPE_ARGS: &[&str] = &[
    "-c", "-e", "--eval", "--execute", "-r", "--interactive", "-i",
];

/// Checks whether a program name is in the allowed rebuild program list.
pub fn is_allowed_rebuild_program(prog: &str) -> bool {
    let prog_name = Path::new(prog)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(prog);
    let clean = prog_name.strip_suffix(".exe").unwrap_or(prog_name);
    ALLOWED_REBUILD_PROGRAMS.contains(&clean)
}

/// A user- or team-defined custom artifact detection rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomRule {
    /// Display name of the artifact (e.g. "Simulator Cache").
    pub name: String,
    /// Directory name to target (e.g. ".sim_cache", "test_output").
    pub dir_name: String,
    /// Optional marker file that must exist in the parent project directory (e.g. "simulator.json").
    #[serde(default)]
    pub parent_marker: Option<String>,
    /// Optional marker file that must exist inside the target directory (e.g. "cache.meta").
    #[serde(default)]
    pub self_marker: Option<String>,
    /// Risk classification: "low" (default), "moderate", or "caution".
    #[serde(default = "default_low_risk")]
    pub risk: String,
    /// Optional toolchain command to regenerate the artifact (e.g. "cargo run --bin build-sim").
    #[serde(default)]
    pub recipe: Option<String>,
    /// Description of the artifact's purpose.
    #[serde(default)]
    pub description: Option<String>,
    /// Ecosystem or toolchain label (default: "Custom").
    #[serde(default = "default_custom_ecosystem")]
    pub ecosystem: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RulesConfigFile {
    #[serde(default)]
    rules: Vec<CustomRule>,
}

impl CustomRule {
    /// Checks whether this rule matches the given directory path.
    pub fn matches_path(&self, path: &Path) -> bool {
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => return false,
        };

        if name != self.dir_name {
            return false;
        }

        if let Some(self_marker) = &self.self_marker
            && !path.join(self_marker).exists()
        {
            return false;
        }

        if let Some(parent_marker) = &self.parent_marker {
            let parent_has_marker = path
                .parent()
                .map(|p| p.join(parent_marker).exists())
                .unwrap_or(false);
            if !parent_has_marker {
                return false;
            }
        }

        true
    }

    /// Returns the parsed RiskLevel.
    pub fn risk_level(&self) -> crate::explain::RiskLevel {
        match self.risk.to_ascii_lowercase().as_str() {
            "caution" | "high" => crate::explain::RiskLevel::Caution,
            "moderate" | "medium" => crate::explain::RiskLevel::Moderate,
            _ => crate::explain::RiskLevel::Low,
        }
    }
}

/// Validates a custom rule definition for syntax and security safety.
pub fn validate_custom_rule(rule: &CustomRule) -> Result<()> {
    if rule.name.trim().is_empty() {
        bail!("rule name cannot be empty");
    }

    let dir = rule.dir_name.trim();
    if dir.is_empty() {
        bail!("rule '{}': dir_name cannot be empty", rule.name);
    }

    if dir.contains('/') || dir.contains('\\') || dir == "." || dir == ".." {
        bail!(
            "rule '{}': dir_name '{}' must be a single directory name without slashes or path traversal",
            rule.name,
            dir
        );
    }

    if let Some(parent_marker) = &rule.parent_marker {
        let marker = parent_marker.trim();
        if marker.is_empty()
            || marker.contains('/')
            || marker.contains('\\')
            || marker == "."
            || marker == ".."
        {
            bail!(
                "rule '{}': parent_marker '{}' must be a single filename without slashes or path traversal",
                rule.name,
                marker
            );
        }
    }

    if let Some(self_marker) = &rule.self_marker {
        let marker = self_marker.trim();
        if marker.is_empty()
            || marker.contains('/')
            || marker.contains('\\')
            || marker == "."
            || marker == ".."
        {
            bail!(
                "rule '{}': self_marker '{}' must be a single filename without slashes or path traversal",
                rule.name,
                marker
            );
        }
    }

    if let Some(recipe) = &rule.recipe {
        let r = recipe.trim();
        if r.is_empty() {
            bail!("rule '{}': recipe cannot be empty if specified", rule.name);
        }
        for c in [';', '&', '|', '`', '$', '>', '<', '\n', '\r'] {
            if r.contains(c) {
                bail!(
                    "rule '{}': recipe '{}' contains forbidden shell metacharacter '{}'",
                    rule.name,
                    r,
                    c
                );
            }
        }
        let parts: Vec<&str> = r.split_whitespace().collect();
        if let Some(prog) = parts.first()
            && !is_allowed_rebuild_program(prog)
        {
            bail!(
                "rule '{}': recipe program '{}' is not in the allowed rebuild program list",
                rule.name,
                prog
            );
        }
        for arg in &parts[1..] {
            if FORBIDDEN_RECIPE_ARGS.contains(arg) {
                bail!(
                    "rule '{}': recipe argument '{}' enables arbitrary code execution and is forbidden",
                    rule.name,
                    arg
                );
            }
        }
    }

    let valid_risks = ["low", "moderate", "medium", "caution", "high"];
    if !valid_risks.contains(&rule.risk.to_ascii_lowercase().as_str()) {
        bail!(
            "rule '{}': invalid risk level '{}' (expected 'low', 'moderate', or 'caution')",
            rule.name,
            rule.risk
        );
    }

    Ok(())
}

/// Parses rules from a TOML string.
pub fn load_rules_from_str(content: &str) -> Result<Vec<CustomRule>> {
    let config: RulesConfigFile = toml::from_str(content).context("failed to parse TOML rules")?;
    for rule in &config.rules {
        validate_custom_rule(rule)?;
    }
    Ok(config.rules)
}

/// Loads and validates custom rules from a TOML file (enforcing a 1 MB file size limit).
pub fn load_rules_from_file(path: &Path) -> Result<Vec<CustomRule>> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to read metadata for rules file {}", path.display()))?;
    const MAX_RULES_FILE_BYTES: u64 = 1024 * 1024;
    if metadata.len() > MAX_RULES_FILE_BYTES {
        bail!(
            "rules file {} exceeds 1 MB limit (size: {} bytes)",
            path.display(),
            metadata.len()
        );
    }
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read rules file {}", path.display()))?;
    load_rules_from_str(&content)
}

/// Default global rules file location: `~/.config/nodewipe/rules.toml`.
pub fn global_rules_file() -> PathBuf {
    if let Some(path) = RULES_FILE_OVERRIDE.get() {
        return path.clone();
    }
    if let Ok(path) = std::env::var("NODEWIPE_RULES_FILE") {
        return PathBuf::from(path);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nodewipe")
        .join("rules.toml")
}

/// Discovers and loads global custom rules from `~/.config/nodewipe/rules.toml`.
pub fn load_global_rules() -> Vec<CustomRule> {
    let file = global_rules_file();
    if file.exists()
        && let Ok(rules) = load_rules_from_file(&file)
    {
        return rules;
    }
    Vec::new()
}

/// Discovers project-level custom rules from `.nodewipe.toml`, `.nodewipe-rules.toml`, or `nodewipe.toml`.
pub fn load_project_rules(start_dir: &Path) -> Vec<CustomRule> {
    let candidate_names = [".nodewipe.toml", ".nodewipe-rules.toml", "nodewipe.toml"];

    let mut current = if start_dir.is_dir() {
        start_dir.to_path_buf()
    } else {
        start_dir.parent().unwrap_or(start_dir).to_path_buf()
    };

    loop {
        for name in &candidate_names {
            let candidate = current.join(name);
            if candidate.is_file()
                && let Ok(rules) = load_rules_from_file(&candidate)
            {
                return rules;
            }
        }

        // Stop at Git repository boundary or filesystem root
        if current.join(".git").exists() || !current.pop() {
            break;
        }
    }

    Vec::new()
}

/// Merges global and project-level custom rules. Project rules take precedence if same name.
pub fn load_active_rules(dir: &Path) -> Vec<CustomRule> {
    let global = load_global_rules();
    let project = load_project_rules(dir);

    let mut merged = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    // Project rules first
    for r in project {
        seen_names.insert(r.name.to_lowercase());
        merged.push(r);
    }

    // Global rules next (if not overridden by name)
    for r in global {
        if !seen_names.contains(&r.name.to_lowercase()) {
            seen_names.insert(r.name.to_lowercase());
            merged.push(r);
        }
    }

    merged
}

/// Saves custom rules to a TOML file.
pub fn save_rules_to_file(rules: &[CustomRule], path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    let config = RulesConfigFile {
        rules: rules.to_vec(),
    };
    let toml_str = toml::to_string_pretty(&config).context("failed to serialize rules to TOML")?;
    fs::write(path, toml_str)
        .with_context(|| format!("failed to write rules file {}", path.display()))?;
    Ok(())
}

/// Starter template for `nodewipe rules init`.
pub fn generate_default_rules_template() -> &'static str {
    r#"# nodewipe — Custom Artifact Detection Rules
#
# Define project-specific or team-specific disposable build artifact directories.
# Place this file as `.nodewipe.toml` in your repository root, or globally
# at `~/.config/nodewipe/rules.toml`.

[[rules]]
name = "Simulator Cache"
dir_name = ".sim_cache"
parent_marker = "simulator.json"
risk = "low"
recipe = "cargo run --bin build-sim"
description = "Hardware simulator intermediate cache and waveforms"
ecosystem = "Hardware Simulation"

[[rules]]
name = "Documentation Build Output"
dir_name = "site_build"
parent_marker = "mkdocs.yml"
risk = "low"
recipe = "mkdocs build"
description = "Compiled MkDocs HTML documentation site"
ecosystem = "Documentation"
"#
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_custom_rule_validation_and_matching() {
        let rule = CustomRule {
            name: "Sim Cache".into(),
            dir_name: ".sim_cache".into(),
            parent_marker: Some("simulator.json".into()),
            self_marker: None,
            risk: "low".into(),
            recipe: Some("cargo run --bin sim".into()),
            description: Some("Simulator cache".into()),
            ecosystem: "Sim".into(),
        };

        assert!(validate_custom_rule(&rule).is_ok());

        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path();
        let sim_cache = project_root.join(".sim_cache");
        fs::create_dir_all(&sim_cache).unwrap();

        // Not matched yet because parent_marker simulator.json doesn't exist
        assert!(!rule.matches_path(&sim_cache));

        // Create simulator.json
        fs::write(project_root.join("simulator.json"), "{}").unwrap();
        assert!(rule.matches_path(&sim_cache));

        // Different directory name doesn't match
        let other_dir = project_root.join("other_dir");
        fs::create_dir_all(&other_dir).unwrap();
        assert!(!rule.matches_path(&other_dir));
    }

    #[test]
    fn test_invalid_rule_rejections() {
        let empty_name = CustomRule {
            name: "".into(),
            dir_name: "test".into(),
            parent_marker: None,
            self_marker: None,
            risk: "low".into(),
            recipe: None,
            description: None,
            ecosystem: "Custom".into(),
        };
        assert!(validate_custom_rule(&empty_name).is_err());

        let path_traversal = CustomRule {
            name: "Bad Dir".into(),
            dir_name: "../escape".into(),
            parent_marker: None,
            self_marker: None,
            risk: "low".into(),
            recipe: None,
            description: None,
            ecosystem: "Custom".into(),
        };
        assert!(validate_custom_rule(&path_traversal).is_err());

        let invalid_risk = CustomRule {
            name: "Bad Risk".into(),
            dir_name: "test".into(),
            parent_marker: None,
            self_marker: None,
            risk: "extreme_danger".into(),
            recipe: None,
            description: None,
            ecosystem: "Custom".into(),
        };
        assert!(validate_custom_rule(&invalid_risk).is_err());

        // Test marker path traversal rejection (S-01)
        let parent_traversal = CustomRule {
            name: "Bad Parent Marker".into(),
            dir_name: "test".into(),
            parent_marker: Some("../evil.json".into()),
            self_marker: None,
            risk: "low".into(),
            recipe: None,
            description: None,
            ecosystem: "Custom".into(),
        };
        assert!(validate_custom_rule(&parent_traversal).is_err());

        let self_traversal = CustomRule {
            name: "Bad Self Marker".into(),
            dir_name: "test".into(),
            parent_marker: None,
            self_marker: Some("../../../etc/shadow".into()),
            risk: "low".into(),
            recipe: None,
            description: None,
            ecosystem: "Custom".into(),
        };
        assert!(validate_custom_rule(&self_traversal).is_err());

        // Test recipe shell injection rejection (S-05)
        let recipe_shell_injection = CustomRule {
            name: "Evil Recipe Chaining".into(),
            dir_name: "test".into(),
            parent_marker: None,
            self_marker: None,
            risk: "low".into(),
            recipe: Some("npm build; rm -rf /".into()),
            description: None,
            ecosystem: "Custom".into(),
        };
        assert!(validate_custom_rule(&recipe_shell_injection).is_err());

        let recipe_forbidden_cmd = CustomRule {
            name: "Evil Recipe Command".into(),
            dir_name: "test".into(),
            parent_marker: None,
            self_marker: None,
            risk: "low".into(),
            recipe: Some("rm -rf target".into()),
            description: None,
            ecosystem: "Custom".into(),
        };
        assert!(validate_custom_rule(&recipe_forbidden_cmd).is_err());

        // Test recipe interpreter code injection (CVE-CLASS-02)
        let recipe_python_eval = CustomRule {
            name: "Evil Python Eval".into(),
            dir_name: "test".into(),
            parent_marker: None,
            self_marker: None,
            risk: "low".into(),
            recipe: Some("python3 -c exec(open('/etc/passwd').read())".into()),
            description: None,
            ecosystem: "Custom".into(),
        };
        assert!(validate_custom_rule(&recipe_python_eval).is_err());

        let recipe_node_eval = CustomRule {
            name: "Evil Node Eval".into(),
            dir_name: "test".into(),
            parent_marker: None,
            self_marker: None,
            risk: "low".into(),
            recipe: Some("node -e require('fs').readFileSync('/etc/passwd')".into()),
            description: None,
            ecosystem: "Custom".into(),
        };
        assert!(validate_custom_rule(&recipe_node_eval).is_err());

        let recipe_forbidden_flag = CustomRule {
            name: "Evil Flag".into(),
            dir_name: "test".into(),
            parent_marker: None,
            self_marker: None,
            risk: "low".into(),
            recipe: Some("cargo build -c foo".into()),
            description: None,
            ecosystem: "Custom".into(),
        };
        assert!(validate_custom_rule(&recipe_forbidden_flag).is_err());
    }

    #[test]
    fn test_rules_file_size_limit() {
        let tmp = TempDir::new().unwrap();
        let large_file = tmp.path().join("too_big.toml");
        // Create file larger than 1MB
        let large_data = vec![b'#'; 1024 * 1024 + 10];
        fs::write(&large_file, large_data).unwrap();

        let res = load_rules_from_file(&large_file);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("exceeds 1 MB limit"));
    }

    #[test]
    fn test_global_rules_override() {
        let tmp = TempDir::new().unwrap();
        let override_file = tmp.path().join("my_custom_rules.toml");
        set_global_rules_override(override_file.clone());
        assert_eq!(global_rules_file(), override_file);
    }

    #[test]
    fn test_save_and_load_rules_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("rules.toml");

        let rule = CustomRule {
            name: "Doc Output".into(),
            dir_name: "site_build".into(),
            parent_marker: Some("mkdocs.yml".into()),
            self_marker: None,
            risk: "low".into(),
            recipe: Some("mkdocs build".into()),
            description: Some("MkDocs output".into()),
            ecosystem: "Documentation".into(),
        };

        save_rules_to_file(std::slice::from_ref(&rule), &file).unwrap();
        assert!(file.exists());

        let loaded = load_rules_from_file(&file).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], rule);
    }
}
