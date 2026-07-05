//! The real `LanguageProfile` registry (§20.1) — not a spike. Loads a
//! curated `languages.toml`, auto-detects a project's language(s) from
//! marker files, and matches individual files to a profile by extension.

use serde::Deserialize;
use std::path::Path;

pub type LanguageId = String;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Mirrors §20.1's `LanguageProfile` struct exactly — `file_globs`,
/// `lsp_command`, `dap_command`, `build_systems`, `formatter`,
/// `tree_sitter_grammar` — plus `marker_files`, which the spec's prose
/// names ("scans for marker files ... Cargo.toml, go.mod, pyproject.toml")
/// without giving the struct itself a field for it; added here since
/// auto-detection needs it to be data, not a hardcoded match statement.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct LanguageProfile {
    pub id: LanguageId,
    pub file_globs: Vec<String>,
    #[serde(default)]
    pub marker_files: Vec<String>,
    pub lsp_command: Option<CommandSpec>,
    pub dap_command: Option<CommandSpec>,
    #[serde(default)]
    pub build_systems: Vec<String>,
    pub formatter: Option<CommandSpec>,
    pub tree_sitter_grammar: String,
}

#[derive(Debug, Deserialize)]
struct RegistryFile {
    language: Vec<LanguageProfile>,
}

#[derive(Debug)]
pub struct LanguageRegistry {
    profiles: Vec<LanguageProfile>,
}

#[derive(Debug)]
pub enum RegistryError {
    Parse(toml::de::Error),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::Parse(e) => write!(f, "failed to parse language registry TOML: {e}"),
        }
    }
}
impl std::error::Error for RegistryError {}

/// The registry this repo ships with — exactly the Tier 1 (v1) six
/// languages named in §35.4 (Rust, TS/JS, Python, Kotlin, Java, Go), not
/// the full ~40-language vision. Growing it is a `languages.toml` edit,
/// not a code change — that's the whole point of the registry pattern,
/// bundled into the binary via `include_str!` so it works with no
/// runtime file lookup required.
const DEFAULT_LANGUAGES_TOML: &str = include_str!("../languages.toml");

impl LanguageRegistry {
    pub fn from_toml_str(toml_str: &str) -> Result<Self, RegistryError> {
        let parsed: RegistryFile = toml::from_str(toml_str).map_err(RegistryError::Parse)?;
        Ok(Self {
            profiles: parsed.language,
        })
    }

    pub fn curated_default() -> Self {
        Self::from_toml_str(DEFAULT_LANGUAGES_TOML).expect("bundled languages.toml must parse")
    }

    pub fn profiles(&self) -> &[LanguageProfile] {
        &self.profiles
    }

    pub fn profile_by_id(&self, id: &str) -> Option<&LanguageProfile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// Which profile owns this file, by extension glob. Returns `None`
    /// for anything not in the registry — that's correct, not a bug: an
    /// unrecognized file still gets tree-sitter highlighting per §20.1's
    /// "degrades gracefully rather than treating unknown languages as
    /// plain text," which is a fact about the editor layer, not something
    /// this registry crate can honestly claim to provide by itself.
    pub fn profile_for_file(&self, path: &Path) -> Option<&LanguageProfile> {
        let file_name = path.file_name()?.to_str()?;
        self.profiles.iter().find(|p| {
            p.file_globs
                .iter()
                .any(|glob| glob_matches(glob, file_name))
        })
    }

    /// Which profile(s) a project root activates. Returns a `Vec` rather
    /// than a single profile since a real repo can be legitimately
    /// multi-language (§20.2's own example: "a Rust core + Kotlin Android
    /// app + TypeScript web frontend in one repo").
    ///
    /// Known, honest limitation: detection is marker-file-name-only, so
    /// two profiles sharing an identical marker file name (e.g. Java and
    /// Kotlin both recognizing a Groovy-DSL `build.gradle`) can't be
    /// perfectly disambiguated without also inspecting file contents or
    /// scanning for `.kt`/`.java` sources — not attempted here, and not
    /// pretended away either.
    pub fn detect_project_languages(&self, project_root: &Path) -> Vec<&LanguageProfile> {
        self.profiles
            .iter()
            .filter(|p| p.marker_files.iter().any(|m| project_root.join(m).exists()))
            .collect()
    }
}

/// A minimal `*.ext` matcher — not a full glob engine (no `**`, `?`,
/// character classes). Every pattern in the bundled registry is a plain
/// `*.ext` form, so this covers what's actually used; upgrading to a real
/// glob crate is a clean drop-in later if a richer pattern is ever needed,
/// not a redesign.
fn glob_matches(glob: &str, file_name: &str) -> bool {
    match glob.strip_prefix("*.") {
        Some(ext) => file_name.rsplit('.').next() == Some(ext),
        None => glob == file_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn curated_default_loads_the_tier_1_six_languages() {
        let registry = LanguageRegistry::curated_default();
        let ids: Vec<&str> = registry.profiles().iter().map(|p| p.id.as_str()).collect();
        for expected in ["rust", "typescript", "python", "kotlin", "java", "go"] {
            assert!(
                ids.contains(&expected),
                "missing {expected} from curated registry: {ids:?}"
            );
        }
        assert_eq!(
            ids.len(),
            6,
            "curated registry should be exactly the Tier 1 six, got {ids:?}"
        );
    }

    #[test]
    fn rust_profile_has_the_real_tools_named_in_the_spec() {
        let registry = LanguageRegistry::curated_default();
        let rust = registry
            .profile_by_id("rust")
            .expect("rust profile must exist");
        assert_eq!(rust.lsp_command.as_ref().unwrap().program, "rust-analyzer");
        assert_eq!(rust.dap_command.as_ref().unwrap().program, "lldb-dap");
        assert_eq!(rust.formatter.as_ref().unwrap().program, "rustfmt");
    }

    #[test]
    fn profile_for_file_matches_by_extension() {
        let registry = LanguageRegistry::curated_default();
        let profile = registry.profile_for_file(Path::new("src/main.rs")).unwrap();
        assert_eq!(profile.id, "rust");

        let ts_profile = registry
            .profile_for_file(Path::new("app/component.tsx"))
            .unwrap();
        assert_eq!(ts_profile.id, "typescript");
    }

    #[test]
    fn profile_for_unknown_extension_returns_none_not_a_panic() {
        let registry = LanguageRegistry::curated_default();
        assert!(registry.profile_for_file(Path::new("notes.txt")).is_none());
    }

    #[test]
    fn detect_project_languages_finds_rust_from_cargo_toml() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();

        let registry = LanguageRegistry::curated_default();
        let detected = registry.detect_project_languages(dir.path());
        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].id, "rust");
    }

    #[test]
    fn detect_project_languages_finds_go_from_go_mod() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("go.mod"), "module example.com/x\n").unwrap();

        let registry = LanguageRegistry::curated_default();
        let detected = registry.detect_project_languages(dir.path());
        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].id, "go");
    }

    #[test]
    fn detect_project_languages_returns_empty_for_unrecognized_project_not_a_panic() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("README.md"), "just a readme\n").unwrap();

        let registry = LanguageRegistry::curated_default();
        let detected = registry.detect_project_languages(dir.path());
        assert!(detected.is_empty());
    }

    /// §20.2's own example: "a Rust core + Kotlin Android app + TypeScript
    /// web frontend in one repo" — a real multi-language monorepo must
    /// detect all applicable profiles, not just the first match.
    #[test]
    fn detect_project_languages_finds_multiple_profiles_in_a_polyglot_repo() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        fs::write(dir.path().join("package.json"), "{}").unwrap();
        fs::write(dir.path().join("build.gradle.kts"), "").unwrap();

        let registry = LanguageRegistry::curated_default();
        let mut ids: Vec<&str> = registry
            .detect_project_languages(dir.path())
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["kotlin", "rust", "typescript"]);
    }

    #[test]
    fn custom_registry_from_toml_str_parses_a_minimal_profile() {
        let toml_str = r#"
            [[language]]
            id = "zig"
            file_globs = ["*.zig"]
            tree_sitter_grammar = "tree-sitter-zig"
        "#;
        let registry = LanguageRegistry::from_toml_str(toml_str).unwrap();
        assert_eq!(registry.profiles().len(), 1);
        assert_eq!(registry.profiles()[0].id, "zig");
        assert!(registry.profiles()[0].lsp_command.is_none());
    }

    #[test]
    fn malformed_toml_returns_a_parse_error_not_a_panic() {
        let err = LanguageRegistry::from_toml_str("this is not valid toml [[[").unwrap_err();
        assert!(matches!(err, RegistryError::Parse(_)));
    }
}
