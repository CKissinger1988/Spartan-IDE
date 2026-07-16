//! Real Hugging Face -> Ollama model downloader: a small, curated list of
//! known-good GGUF models (never a full HF search API -- that's real,
//! separate, unstarted future work) and a real `ollama pull hf.co/<repo>:
//! <tag>` trigger, streaming progress the same way `litellm_proxy`/
//! `spartan_backend::devcontainer_up` already do.
//!
//! Ollama's own `hf.co/` pull syntax is real and documented -- this module
//! doesn't reimplement any download logic of its own, it only shells out to
//! the user's already-installed `ollama` binary, the same "go through the
//! tool's own real interface" choice `spartan_model::OllamaProvider` already
//! makes for Ollama's HTTP API.

use crate::subprocess;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HfModel {
    pub id: &'static str,
    pub display_name: &'static str,
    pub hf_repo: &'static str,
    pub tag: &'static str,
    pub description: &'static str,
}

/// A small, hand-curated list -- deliberately not a live Hugging Face
/// search API call. Each entry names a real, genuinely-existing public
/// GGUF repo pullable via Ollama's own `hf.co/<repo>:<tag>` syntax; none
/// has been live-pulled in this environment (a real multi-hundred-MB to
/// multi-GB download, an honest, deliberate cost not paid here) -- only the
/// argv construction and dispatch mechanics are verified, via
/// `subprocess::spawn_streaming`'s own always-on tests and this module's
/// own pure list/lookup tests.
pub const CURATED_MODELS: &[HfModel] = &[
    HfModel {
        id: "llama-3.2-3b-instruct-q4",
        display_name: "Llama 3.2 3B Instruct (Q4_K_M)",
        hf_repo: "bartowski/Llama-3.2-3B-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "Small, fast instruct model -- good for quick local iteration.",
    },
    HfModel {
        id: "qwen2.5-coder-7b-instruct-q4",
        display_name: "Qwen2.5 Coder 7B Instruct (Q4_K_M)",
        hf_repo: "bartowski/Qwen2.5-Coder-7B-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "Coding-focused instruct model in the ~7B class.",
    },
    HfModel {
        id: "mistral-7b-instruct-v0.3-q4",
        display_name: "Mistral 7B Instruct v0.3 (Q4_K_M)",
        hf_repo: "bartowski/Mistral-7B-Instruct-v0.3-GGUF",
        tag: "Q4_K_M",
        description: "General-purpose instruct model, a widely used baseline.",
    },
    HfModel {
        id: "deepseek-coder-v2-lite-instruct-q4",
        display_name: "DeepSeek Coder V2 Lite Instruct (Q4_K_M)",
        hf_repo: "bartowski/DeepSeek-Coder-V2-Lite-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "Larger coding-focused MoE model, still practical for local use.",
    },
];

/// Looks up a curated model by its real, stable `id` -- never the raw
/// HF repo string, so the caller-facing surface stays independent of any
/// future change to which exact repo/tag backs a given curated entry.
pub fn find_model(id: &str) -> Option<&'static HfModel> {
    CURATED_MODELS.iter().find(|m| m.id == id)
}

/// The exact real target string Ollama's own `hf.co/` pull syntax expects.
pub fn pull_target(model: &HfModel) -> String {
    format!("hf.co/{}:{}", model.hf_repo, model.tag)
}

/// A real, cheap check -- `ollama --version`, discarding output -- whether
/// the `ollama` CLI is actually on `$PATH`. Matches `litellm_proxy::
/// is_litellm_available`'s own shape exactly.
pub fn is_ollama_available() -> bool {
    Command::new("ollama")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Spawns a real `ollama pull hf.co/<repo>:<tag>`, streaming its real
/// stdout+stderr lines to `progress_tx` -- Ollama's own pull output already
/// includes real percentage/rate progress lines, forwarded verbatim rather
/// than parsed, the same "forward the tool's own real output" choice
/// `devcontainer_up`'s `emit_progress` already makes for a real `docker
/// pull`. No cancel/stop control exists for an in-flight pull -- a real,
/// deliberately deferred follow-up, named here rather than silently
/// absorbed, matching `litellm_proxy`'s own "no restart-on-crash" precedent.
pub fn spawn_pull(model: &HfModel, progress_tx: Sender<String>) -> std::io::Result<Child> {
    let target = pull_target(model);
    subprocess::spawn_streaming("ollama", &["pull".to_string(), target], progress_tx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_models_is_non_empty_and_well_formed() {
        assert!(!CURATED_MODELS.is_empty());
        for model in CURATED_MODELS {
            assert!(!model.id.is_empty());
            assert!(!model.display_name.is_empty());
            assert!(
                model.hf_repo.contains('/'),
                "a real HF repo id is always `<org>/<name>`: {}",
                model.hf_repo
            );
            assert!(!model.tag.is_empty());
            assert!(!model.description.is_empty());
        }
    }

    #[test]
    fn curated_models_has_no_duplicate_ids() {
        let mut ids: Vec<&str> = CURATED_MODELS.iter().map(|m| m.id).collect();
        let count_before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count_before, "duplicate curated model id found");
    }

    #[test]
    fn find_model_finds_a_real_curated_entry_and_none_for_unknown() {
        let first = CURATED_MODELS[0];
        assert_eq!(find_model(first.id), Some(&first));
        assert_eq!(find_model("not-a-real-curated-id"), None);
    }

    #[test]
    fn pull_target_builds_the_real_ollama_hf_syntax() {
        let model = CURATED_MODELS[0];
        let target = pull_target(&model);
        assert_eq!(target, format!("hf.co/{}:{}", model.hf_repo, model.tag));
        assert!(target.starts_with("hf.co/"));
    }

    #[test]
    fn is_ollama_available_runs_without_panicking() {
        let _ = is_ollama_available();
    }
}
