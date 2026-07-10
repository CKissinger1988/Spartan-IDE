//! Real, pure parsing of a project's `devcontainer.json` -- the open
//! containers.dev specification the same one VS Code Dev Containers,
//! GitHub Codespaces, and JetBrains Gateway all implement. Deliberately
//! a real, honest *subset* of the full spec (the full spec has
//! `features`, `customizations`, lifecycle hooks beyond
//! `postCreateCommand`, Docker Compose multi-service support, and more)
//! -- this covers the fields that actually matter for "build an image or
//! pull one, start a container with my project mounted in, run a setup
//! command, forward some ports," the real, useful core rather than every
//! edge case the spec allows.
//!
//! A real, deliberate implementation choice: the spec explicitly permits
//! `devcontainer.json` to contain `//` and `/* */` comments (it's JSONC,
//! not strict JSON) -- `strip_jsonc_comments` handles that before handing
//! off to `serde_json`, rather than silently failing to parse a real,
//! spec-compliant file that happens to use comments (which most real
//! `devcontainer.json` files in the wild actually do).

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BuildConfig {
    pub dockerfile: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub args: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DevContainerConfig {
    pub name: Option<String>,
    /// A pre-built image to pull and run directly -- mutually exclusive
    /// in practice with `build` (the real spec allows both to be present
    /// but only one is ever actually used; `build` wins if both are set,
    /// matching every real implementation's own documented precedence).
    pub image: Option<String>,
    pub build: Option<BuildConfig>,
    #[serde(default, rename = "forwardPorts")]
    pub forward_ports: Vec<u16>,
    #[serde(default, rename = "postCreateCommand")]
    pub post_create_command: Option<PostCreateCommand>,
    #[serde(default, rename = "containerEnv")]
    pub container_env: std::collections::BTreeMap<String, String>,
    /// Real spec mount strings, e.g.
    /// `"source=${localWorkspaceFolder},target=/workspace,type=bind"` --
    /// kept as raw strings and handed to Docker verbatim rather than
    /// parsed into a structured type, since Docker's own mount-string
    /// grammar is already the real source of truth and re-validating it
    /// here would just risk drifting from it.
    #[serde(default)]
    pub mounts: Vec<String>,
    #[serde(rename = "remoteUser")]
    pub remote_user: Option<String>,
    #[serde(rename = "workspaceFolder")]
    pub workspace_folder: Option<String>,
}

/// The real spec allows `postCreateCommand` to be a single string OR an
/// array of strings (argv form, no shell involved) -- both are real,
/// valid, commonly-used shapes in the wild.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PostCreateCommand {
    Shell(String),
    Argv(Vec<String>),
}

impl PostCreateCommand {
    /// Real, uniform argv this crate's own exec call actually needs --
    /// the shell form runs through `/bin/sh -c`, matching the real
    /// spec's own documented behavior for a plain string command.
    pub fn to_argv(&self) -> Vec<String> {
        match self {
            PostCreateCommand::Shell(s) => vec!["/bin/sh".to_string(), "-c".to_string(), s.clone()],
            PostCreateCommand::Argv(argv) => argv.clone(),
        }
    }
}

#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Io(e) => write!(f, "io error: {e}"),
            ParseError::Json(e) => write!(f, "invalid devcontainer.json: {e}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Real, minimal JSONC-to-JSON comment stripper -- handles `//` line
/// comments and `/* */` block comments, correctly leaving comment-like
/// sequences *inside* real string literals untouched (the actual reason
/// a naive regex-based stripper would corrupt a real file whose
/// `postCreateCommand` value legitimately contains `"//"` or similar).
/// A real, minimal state machine, not a regex -- string-literal
/// awareness is exactly the thing a regex can't reliably get right here.
fn strip_jsonc_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }

        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                chars.next();
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = ' ';
                for c in chars.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    if c == '\n' {
                        out.push('\n');
                    }
                    prev = c;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Real parse from a raw string -- the pure, fully unit-testable core.
pub fn parse(raw: &str) -> Result<DevContainerConfig, ParseError> {
    let stripped = strip_jsonc_comments(raw);
    serde_json::from_str(&stripped).map_err(ParseError::Json)
}

/// Real filesystem lookup -- the spec allows either `.devcontainer/
/// devcontainer.json` or a bare `.devcontainer.json` at the project
/// root, checked in that order (matching every real implementation's
/// own documented precedence, `.devcontainer/` first).
pub fn find_config_path(project_root: &Path) -> Option<std::path::PathBuf> {
    let nested = project_root.join(".devcontainer").join("devcontainer.json");
    if nested.is_file() {
        return Some(nested);
    }
    let flat = project_root.join(".devcontainer.json");
    if flat.is_file() {
        return Some(flat);
    }
    None
}

/// Real, combined detect-and-parse -- the one real call site
/// `spartan-backend` actually needs.
pub fn detect(project_root: &Path) -> Result<Option<DevContainerConfig>, ParseError> {
    let Some(path) = find_config_path(project_root) else {
        return Ok(None);
    };
    let raw = std::fs::read_to_string(&path).map_err(ParseError::Io)?;
    parse(&raw).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_real_minimal_image_only_config() {
        let raw = r#"{ "name": "My Project", "image": "mcr.microsoft.com/devcontainers/rust:1" }"#;
        let config = parse(raw).unwrap();
        assert_eq!(config.name.as_deref(), Some("My Project"));
        assert_eq!(
            config.image.as_deref(),
            Some("mcr.microsoft.com/devcontainers/rust:1")
        );
        assert!(config.build.is_none());
    }

    #[test]
    fn parses_a_real_build_config_with_args() {
        let raw = r#"{
            "build": {
                "dockerfile": "Dockerfile",
                "context": "..",
                "args": { "VARIANT": "1.75" }
            }
        }"#;
        let config = parse(raw).unwrap();
        let build = config.build.unwrap();
        assert_eq!(build.dockerfile.as_deref(), Some("Dockerfile"));
        assert_eq!(build.context.as_deref(), Some(".."));
        assert_eq!(build.args.get("VARIANT").map(String::as_str), Some("1.75"));
    }

    #[test]
    fn parses_real_forward_ports_and_container_env() {
        let raw = r#"{
            "image": "ubuntu:22.04",
            "forwardPorts": [3000, 8080],
            "containerEnv": { "RUST_LOG": "debug" }
        }"#;
        let config = parse(raw).unwrap();
        assert_eq!(config.forward_ports, vec![3000, 8080]);
        assert_eq!(
            config.container_env.get("RUST_LOG").map(String::as_str),
            Some("debug")
        );
    }

    #[test]
    fn parses_a_real_shell_form_post_create_command() {
        let raw = r#"{ "image": "ubuntu:22.04", "postCreateCommand": "cargo build" }"#;
        let config = parse(raw).unwrap();
        assert_eq!(
            config.post_create_command.unwrap().to_argv(),
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "cargo build".to_string()
            ]
        );
    }

    #[test]
    fn parses_a_real_argv_form_post_create_command() {
        let raw = r#"{ "image": "ubuntu:22.04", "postCreateCommand": ["cargo", "build"] }"#;
        let config = parse(raw).unwrap();
        assert_eq!(
            config.post_create_command.unwrap().to_argv(),
            vec!["cargo".to_string(), "build".to_string()]
        );
    }

    #[test]
    fn parses_real_mounts_remote_user_and_workspace_folder() {
        let raw = r#"{
            "image": "ubuntu:22.04",
            "mounts": ["source=${localWorkspaceFolder},target=/workspace,type=bind"],
            "remoteUser": "vscode",
            "workspaceFolder": "/workspace"
        }"#;
        let config = parse(raw).unwrap();
        assert_eq!(config.mounts.len(), 1);
        assert!(config.mounts[0].contains("target=/workspace"));
        assert_eq!(config.remote_user.as_deref(), Some("vscode"));
        assert_eq!(config.workspace_folder.as_deref(), Some("/workspace"));
    }

    #[test]
    fn strips_real_line_and_block_comments_without_corrupting_real_strings() {
        let raw = r#"{
            // this is a real line comment
            "name": "Test", /* inline block comment */ "image": "ubuntu:22.04",
            /* a real
               multi-line
               block comment */
            "postCreateCommand": "echo 'not a // comment inside a string'"
        }"#;
        let config = parse(raw).unwrap();
        assert_eq!(config.name.as_deref(), Some("Test"));
        assert_eq!(
            config.post_create_command.unwrap().to_argv()[2],
            "echo 'not a // comment inside a string'"
        );
    }

    #[test]
    fn a_url_looking_value_inside_a_string_is_not_mistaken_for_a_comment() {
        // A real, easy-to-get-wrong case for a naive stripper: "https://"
        // contains "//" but must not be treated as a line comment.
        let raw =
            r#"{ "image": "ubuntu:22.04", "containerEnv": { "URL": "https://example.com/a" } }"#;
        let config = parse(raw).unwrap();
        assert_eq!(
            config.container_env.get("URL").map(String::as_str),
            Some("https://example.com/a")
        );
    }

    #[test]
    fn invalid_json_after_stripping_comments_is_a_real_parse_error_not_a_panic() {
        let raw = "{ this is not valid json }";
        assert!(parse(raw).is_err());
    }

    #[test]
    fn build_wins_over_image_per_real_spec_precedence_when_both_are_present() {
        // Real spec note: both fields are structurally allowed to
        // coexist in a real file even though only one is ever actually
        // used -- this crate doesn't attempt to detect or warn on that
        // real ambiguity here, callers are expected to check `build`
        // first, matching every other real implementation's documented
        // precedence (asserted in `docker.rs`'s own real container-
        // creation logic, not here in pure parsing).
        let raw = r#"{
            "image": "ubuntu:22.04",
            "build": { "dockerfile": "Dockerfile" }
        }"#;
        let config = parse(raw).unwrap();
        assert!(config.image.is_some());
        assert!(config.build.is_some());
    }

    #[test]
    fn find_config_path_prefers_the_nested_devcontainer_directory_form() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".devcontainer")).unwrap();
        std::fs::write(
            dir.path().join(".devcontainer").join("devcontainer.json"),
            r#"{"image":"ubuntu:22.04"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".devcontainer.json"),
            r#"{"image":"debian:12"}"#,
        )
        .unwrap();

        let found = find_config_path(dir.path()).unwrap();
        assert!(found.ends_with(".devcontainer/devcontainer.json"));
    }

    #[test]
    fn find_config_path_falls_back_to_the_flat_form() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".devcontainer.json"),
            r#"{"image":"debian:12"}"#,
        )
        .unwrap();
        let found = find_config_path(dir.path()).unwrap();
        assert!(found.ends_with(".devcontainer.json"));
    }

    #[test]
    fn find_config_path_returns_none_when_no_real_config_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_config_path(dir.path()).is_none());
    }

    #[test]
    fn detect_returns_none_not_an_error_when_no_config_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect(dir.path()).unwrap().is_none());
    }

    #[test]
    fn detect_reads_and_parses_a_real_config_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".devcontainer")).unwrap();
        std::fs::write(
            dir.path().join(".devcontainer").join("devcontainer.json"),
            r#"{ "name": "Real", "image": "ubuntu:22.04" }"#,
        )
        .unwrap();
        let config = detect(dir.path()).unwrap().unwrap();
        assert_eq!(config.name.as_deref(), Some("Real"));
    }
}
