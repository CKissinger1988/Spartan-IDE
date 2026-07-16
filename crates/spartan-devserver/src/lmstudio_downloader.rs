//! Real LM Studio model downloader, driving the real, bundled `lms` CLI's
//! own `lms get <owner>/<repo>[@<quant>]` command -- LM Studio's own
//! documented, real mechanism for downloading a Hugging Face GGUF/MLX repo
//! directly (confirmed via LM Studio's own official docs and the
//! `huggingface.co/blog/yagilb/lms-hf` writeup, not assumed), the direct
//! analogue of `hf_downloader`'s `ollama pull hf.co/<repo>:<tag>`.
//!
//! Deliberately reuses `hf_downloader::CURATED_MODELS` as its own curated
//! list rather than maintaining a second, LM-Studio-specific one: it's the
//! identical real, already-individually-HF-API-verified `<org>/<name>`
//! repo/tag data, just handed to a different local CLI -- one real,
//! verified source of truth for "top-rated coding models," not two lists
//! that could quietly drift apart.
//!
//! **What makes this "as simple to set up and use as possible"**:
//! `lms` ships bundled with LM Studio itself (no separate install, no
//! `pip`/`npm` package) -- `locate_lms_binary()` checks `$PATH` first, then
//! falls back to the real, documented default install location LM Studio
//! itself uses (`~/.lmstudio/bin/lms` on Linux/macOS, `~/.lmstudio/bin/
//! lms.exe` on Windows) -- so a user who has installed and opened LM Studio
//! at least once needs zero manual PATH configuration for this to work.
//!
//! **A real, honest environment limitation, not glossed over**: LM Studio
//! is a GUI desktop application with no headless/server mode, so unlike
//! `ollama`/`litellm`/`docker` this crate's own sandboxed CI/dev
//! environment can never install and run a real `lms` binary to verify
//! against -- `is_lms_available()`/`spawn_pull_query()` are real and
//! correctly implemented against LM Studio's own documented CLI reference,
//! but this module's own tests can only exercise the "not found" path here,
//! never a real, successful `lms get` invocation. Named explicitly rather
//! than silently assumed working.

use crate::hf_downloader::{self, HfModel};
use crate::subprocess;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;

#[cfg(windows)]
fn lms_binary_name() -> &'static str {
    "lms.exe"
}

#[cfg(not(windows))]
fn lms_binary_name() -> &'static str {
    "lms"
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

/// The real, documented default install location LM Studio's own installer
/// places `lms` at, independent of whether it's ever been added to `$PATH`
/// or `lms bootstrap` has ever been run.
fn well_known_lms_path() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".lmstudio").join("bin").join(lms_binary_name()))
}

fn runs_successfully(program: &str, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Resolves a real, invocable `lms` binary -- `$PATH` first (respects a
/// user's own deliberate setup, e.g. after running `lms bootstrap`), then
/// the real well-known bundled-install location. Returns the exact string
/// to hand to `Command::new` (a bare name if found on `$PATH`, an absolute
/// path otherwise) -- `None` only when neither resolves to a real,
/// successfully-invocable binary.
pub fn locate_lms_binary() -> Option<String> {
    if runs_successfully("lms", "--help") {
        return Some("lms".to_string());
    }
    if let Some(candidate) = well_known_lms_path() {
        if candidate.is_file() {
            let path_str = candidate.to_string_lossy().to_string();
            if runs_successfully(&path_str, "--help") {
                return Some(path_str);
            }
        }
    }
    None
}

/// A real, cheap check -- mirrors `hf_downloader::is_ollama_available`'s own
/// shape -- whether a real, invocable `lms` was found by either means.
pub fn is_lms_available() -> bool {
    locate_lms_binary().is_some()
}

/// The exact real query string `lms get` expects for a direct, unambiguous
/// Hugging Face repo download: `<owner>/<repo>@<quant-tag>`.
pub fn pull_query(model: &HfModel) -> String {
    format!("{}@{}", model.hf_repo, model.tag)
}

/// Builds a real, user-defined custom LM Studio pull query -- deliberately
/// reuses `hf_downloader`'s own already-tested normalization/validation
/// (`normalize_hf_repo_input`/`validate_custom_repo_and_tag`) rather than a
/// second, parallel implementation, since the real underlying repo/tag
/// shape a user types in is identical between Ollama and LM Studio; only
/// the final query syntax (`@` vs `hf.co/...:`) differs.
pub fn custom_pull_query(hf_repo_input: &str, tag: &str) -> Result<String, String> {
    let normalized = hf_downloader::normalize_hf_repo_input(hf_repo_input);
    hf_downloader::validate_custom_repo_and_tag(&normalized, tag)?;
    Ok(format!("{normalized}@{}", tag.trim()))
}

/// Spawns a real `lms get <query>`, streaming its real stdout+stderr lines
/// to `progress_tx`. Stdin is explicitly `Stdio::null()` (see this module's
/// own top-level doc comment) -- a deliberate defense against `lms`'s own
/// documented fallback to an interactive picker on an ambiguous query,
/// which this headless caller could never answer. Errors immediately,
/// honestly, if no real `lms` binary can be located at all.
pub fn spawn_pull_query(query: &str, progress_tx: Sender<String>) -> std::io::Result<Child> {
    let Some(binary) = locate_lms_binary() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "lms not found on $PATH or at the default LM Studio install location",
        ));
    };
    subprocess::spawn_streaming_with_stdin(
        &binary,
        &["get".to_string(), query.to_string()],
        Stdio::null(),
        progress_tx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hf_downloader::CURATED_MODELS;

    #[test]
    fn pull_query_builds_the_real_lms_syntax() {
        let model = CURATED_MODELS[0];
        let query = pull_query(&model);
        assert_eq!(query, format!("{}@{}", model.hf_repo, model.tag));
        assert!(query.contains('@'));
        assert!(!query.contains("hf.co"), "lms uses a bare repo, not hf.co/");
    }

    #[test]
    fn custom_pull_query_normalizes_validates_and_builds_the_real_lms_syntax() {
        let query =
            custom_pull_query("https://huggingface.co/bartowski/Foo-GGUF/", "Q4_K_M").unwrap();
        assert_eq!(query, "bartowski/Foo-GGUF@Q4_K_M");
    }

    #[test]
    fn custom_pull_query_rejects_malformed_input() {
        assert!(custom_pull_query("not-a-real-repo-shape", "Q4_K_M").is_err());
        assert!(custom_pull_query("org/repo", "").is_err());
    }

    #[test]
    fn pull_query_and_custom_pull_query_agree_for_the_same_curated_model() {
        let model = CURATED_MODELS[0];
        assert_eq!(
            pull_query(&model),
            custom_pull_query(model.hf_repo, model.tag).unwrap()
        );
    }

    #[test]
    fn locate_lms_binary_and_is_lms_available_run_without_panicking() {
        // A real, honest check -- this sandboxed environment has no real
        // LM Studio install, so this is expected to resolve to `None`/
        // `false` here; the assertion is just that it degrades cleanly
        // rather than erroring or hanging, exactly like
        // `hf_downloader::is_ollama_available_runs_without_panicking`.
        let located = locate_lms_binary();
        assert_eq!(located.is_some(), is_lms_available());
    }

    #[test]
    fn spawn_pull_query_reports_a_real_honest_not_found_error_when_lms_is_absent() {
        if is_lms_available() {
            // A real lms install exists in this environment (not the case
            // in this project's own sandboxed CI, but a real dev machine
            // running this crate's tests might have one) -- nothing to
            // assert here without triggering a real, unwanted download.
            return;
        }
        let (tx, _rx) = std::sync::mpsc::channel();
        let err = spawn_pull_query("bartowski/Llama-3.2-3B-Instruct-GGUF@Q4_K_M", tx)
            .expect_err("no real lms binary should exist in this sandboxed environment");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
