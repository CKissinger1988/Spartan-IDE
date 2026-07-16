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

/// A hand-curated list spanning the real, well-known top-rated coding
/// models available on Hugging Face as GGUF quants -- deliberately not a
/// live Hugging Face search API call (that's real, separate, unstarted
/// future work; a live search would need its own auth/rate-limit story).
/// Every single entry below was verified for real in this environment
/// before being added, not assumed from memory: each repo was checked via
/// a live, unauthenticated `GET https://huggingface.co/api/models/<repo>`
/// (200, not 401/404 -- a handful of otherwise-plausible candidates,
/// including `bartowski/CodeLlama-7B-Instruct-GGUF` and
/// `bartowski/deepseek-coder-6.7b-instruct-GGUF`, came back 401/gated and
/// were deliberately excluded, since a gated repo can't be anonymously
/// `ollama pull`ed either), and each repo's real file listing was checked
/// to confirm an actual `*Q4_K_M*.gguf` sibling exists at the exact tag
/// string used here. None has actually been *pulled* in this environment
/// (a real multi-hundred-MB to multi-GB download, an honest, deliberate
/// cost not paid here) -- only real repo/tag existence, plus this module's
/// own argv construction and dispatch mechanics (verified via
/// `subprocess::spawn_streaming`'s own always-on tests and this module's
/// own pure list/lookup tests).
pub const CURATED_MODELS: &[HfModel] = &[
    HfModel {
        id: "llama-3.2-3b-instruct-q4",
        display_name: "Llama 3.2 3B Instruct (Q4_K_M)",
        hf_repo: "bartowski/Llama-3.2-3B-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "Small, fast general instruct model -- good for quick local iteration.",
    },
    HfModel {
        id: "llama-3.1-8b-instruct-q4",
        display_name: "Llama 3.1 8B Instruct (Q4_K_M)",
        hf_repo: "bartowski/Meta-Llama-3.1-8B-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "General-purpose ~8B baseline -- this project's own default Leo/Ollama model.",
    },
    HfModel {
        id: "mistral-7b-instruct-v0.3-q4",
        display_name: "Mistral 7B Instruct v0.3 (Q4_K_M)",
        hf_repo: "bartowski/Mistral-7B-Instruct-v0.3-GGUF",
        tag: "Q4_K_M",
        description: "General-purpose instruct model, a widely used baseline.",
    },
    HfModel {
        id: "phi-3.5-mini-instruct-q4",
        display_name: "Phi-3.5 Mini Instruct (Q4_K_M)",
        hf_repo: "bartowski/Phi-3.5-mini-instruct-GGUF",
        tag: "Q4_K_M",
        description: "Small (3.8B) Microsoft model, strong for its size, decent at code.",
    },
    // -- Dedicated coding models, smallest to largest --
    HfModel {
        id: "qwen2.5-coder-0.5b-instruct-q4",
        display_name: "Qwen2.5 Coder 0.5B Instruct (Q4_K_M)",
        hf_repo: "bartowski/Qwen2.5-Coder-0.5B-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "Tiny coding model -- fits on almost anything, lowest quality tier.",
    },
    HfModel {
        id: "qwen2.5-coder-1.5b-instruct-q4",
        display_name: "Qwen2.5 Coder 1.5B Instruct (Q4_K_M)",
        hf_repo: "bartowski/Qwen2.5-Coder-1.5B-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "Small coding model, a step up from the 0.5B tier.",
    },
    HfModel {
        id: "qwen2.5-coder-3b-instruct-q4",
        display_name: "Qwen2.5 Coder 3B Instruct (Q4_K_M)",
        hf_repo: "bartowski/Qwen2.5-Coder-3B-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "Small-to-mid coding model, a practical low-resource choice.",
    },
    HfModel {
        id: "qwen2.5-coder-7b-instruct-q4",
        display_name: "Qwen2.5 Coder 7B Instruct (Q4_K_M)",
        hf_repo: "bartowski/Qwen2.5-Coder-7B-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "Coding-focused instruct model in the ~7B class -- a top-rated pick.",
    },
    HfModel {
        id: "qwen2.5-coder-14b-instruct-q4",
        display_name: "Qwen2.5 Coder 14B Instruct (Q4_K_M)",
        hf_repo: "bartowski/Qwen2.5-Coder-14B-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "Larger Qwen2.5 Coder tier -- stronger, needs more RAM/VRAM.",
    },
    HfModel {
        id: "qwen2.5-coder-32b-instruct-q4",
        display_name: "Qwen2.5 Coder 32B Instruct (Q4_K_M)",
        hf_repo: "bartowski/Qwen2.5-Coder-32B-Instruct-GGUF",
        tag: "Q4_K_M",
        description:
            "Top-of-line Qwen2.5 Coder tier -- one of the highest-rated open coding models.",
    },
    HfModel {
        id: "codeqwen1.5-7b-chat-q4",
        display_name: "CodeQwen1.5 7B Chat (Q4_K_M)",
        hf_repo: "bartowski/CodeQwen1.5-7B-Chat-GGUF",
        tag: "Q4_K_M",
        description: "Qwen's earlier dedicated coding model line, still widely used.",
    },
    HfModel {
        id: "deepseek-coder-v2-lite-instruct-q4",
        display_name: "DeepSeek Coder V2 Lite Instruct (Q4_K_M)",
        hf_repo: "bartowski/DeepSeek-Coder-V2-Lite-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "Coding-focused MoE model, still practical for local use.",
    },
    HfModel {
        id: "codestral-22b-v0.1-q4",
        display_name: "Codestral 22B v0.1 (Q4_K_M)",
        hf_repo: "bartowski/Codestral-22B-v0.1-GGUF",
        tag: "Q4_K_M",
        description: "Mistral's own dedicated code-generation model, 22B, 80+ languages.",
    },
    HfModel {
        id: "codellama-7b-instruct-q4",
        display_name: "Code Llama 7B Instruct (Q4_K_M)",
        hf_repo: "TheBloke/CodeLlama-7B-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "Meta's original dedicated coding model line, small tier.",
    },
    HfModel {
        id: "codellama-34b-instruct-q4",
        display_name: "Code Llama 34B Instruct (Q4_K_M)",
        hf_repo: "TheBloke/CodeLlama-34B-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "Meta's original dedicated coding model line, large tier.",
    },
    HfModel {
        id: "starcoder2-15b-instruct-q4",
        display_name: "StarCoder2 15B Instruct (Q4_K_M)",
        hf_repo: "bartowski/starcoder2-15b-instruct-v0.1-GGUF",
        tag: "Q4_K_M",
        description: "BigCode's StarCoder2 line, instruction-tuned, trained on The Stack.",
    },
    HfModel {
        id: "yi-coder-1.5b-chat-q4",
        display_name: "Yi Coder 1.5B Chat (Q4_K_M)",
        hf_repo: "bartowski/Yi-Coder-1.5B-Chat-GGUF",
        tag: "Q4_K_M",
        description: "01.AI's small dedicated coding model.",
    },
    HfModel {
        id: "yi-coder-9b-chat-q4",
        display_name: "Yi Coder 9B Chat (Q4_K_M)",
        hf_repo: "bartowski/Yi-Coder-9B-Chat-GGUF",
        tag: "Q4_K_M",
        description: "01.AI's larger dedicated coding model, strong benchmark results.",
    },
    HfModel {
        id: "opencoder-8b-instruct-q4",
        display_name: "OpenCoder 8B Instruct (Q4_K_M)",
        hf_repo: "bartowski/OpenCoder-8B-Instruct-GGUF",
        tag: "Q4_K_M",
        description: "Fully open-data coding model (InfiniAI/M-A-P), competitive at 8B.",
    },
    HfModel {
        id: "granite-3.0-8b-instruct-q4",
        display_name: "Granite 3.0 8B Instruct (Q4_K_M)",
        hf_repo: "bartowski/granite-3.0-8b-instruct-GGUF",
        tag: "Q4_K_M",
        description: "IBM's open Granite line -- general + code, enterprise-oriented.",
    },
    HfModel {
        id: "codegemma-7b-it-q4",
        display_name: "CodeGemma 7B IT (Q4_K_M)",
        hf_repo: "bartowski/codegemma-7b-it-GGUF",
        tag: "Q4_K_M",
        description: "Google's dedicated coding model, built on the Gemma line.",
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

/// Spawns a real `ollama pull <target>` (`target` already the exact
/// `hf.co/<repo>:<tag>` string), streaming its real stdout+stderr lines to
/// `progress_tx` -- Ollama's own pull output already includes real
/// percentage/rate progress lines, forwarded verbatim rather than parsed,
/// the same "forward the tool's own real output" choice `devcontainer_up`'s
/// `emit_progress` already makes for a real `docker pull`. No cancel/stop
/// control exists for an in-flight pull -- a real, deliberately deferred
/// follow-up, named here rather than silently absorbed, matching
/// `litellm_proxy`'s own "no restart-on-crash" precedent. The one real
/// entry point both a curated pull and a user-defined custom pull go
/// through -- neither path reimplements subprocess spawning separately.
pub fn spawn_pull_target(target: &str, progress_tx: Sender<String>) -> std::io::Result<Child> {
    subprocess::spawn_streaming(
        "ollama",
        &["pull".to_string(), target.to_string()],
        progress_tx,
    )
}

/// Spawns a real pull for a curated model -- a thin wrapper over
/// `spawn_pull_target`.
pub fn spawn_pull(model: &HfModel, progress_tx: Sender<String>) -> std::io::Result<Child> {
    spawn_pull_target(&pull_target(model), progress_tx)
}

/// Strips a handful of real, common prefixes a user might reasonably paste
/// in (a full `https://huggingface.co/<repo>` browser URL, a bare
/// `huggingface.co/<repo>`, or Ollama's own `hf.co/<repo>` short form) down
/// to the bare `<org>/<name>` repo id this module's own `HfModel::hf_repo`
/// field always holds -- the real "user defined model download links"
/// entry point's own first step, so a pasted link and a bare repo id both
/// resolve to the identical real pull target.
pub fn normalize_hf_repo_input(input: &str) -> String {
    let s = input.trim();
    let s = s
        .strip_prefix("https://huggingface.co/")
        .or_else(|| s.strip_prefix("http://huggingface.co/"))
        .or_else(|| s.strip_prefix("huggingface.co/"))
        .or_else(|| s.strip_prefix("hf.co/"))
        .unwrap_or(s);
    s.trim_matches('/').to_string()
}

/// Real validation for a user-supplied custom HF repo + tag pair, run
/// before either is ever turned into a real `ollama pull` subprocess
/// argument. `Command`'s argv is passed directly to the OS, never through a
/// shell, so there's no shell-injection surface here regardless -- this
/// instead guards against two real, distinct problems: malformed input
/// producing a confusing failure deep inside `ollama` itself instead of a
/// clear one here, and a caller trying to smuggle a second CLI flag in as
/// if it were a repo/tag (a leading `-` would otherwise be interpreted by
/// `ollama` itself as a real flag, not a literal repo/tag string).
fn is_safe_component(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub fn validate_custom_repo_and_tag(hf_repo: &str, tag: &str) -> Result<(), String> {
    let hf_repo = hf_repo.trim();
    let tag = tag.trim();
    if hf_repo.is_empty() {
        return Err("hf_repo must not be empty".to_string());
    }
    if tag.is_empty() {
        return Err("tag must not be empty".to_string());
    }
    let parts: Vec<&str> = hf_repo.split('/').collect();
    if parts.len() != 2 {
        return Err(format!(
            "hf_repo must be in `<org>/<name>` form, got {hf_repo:?}"
        ));
    }
    if !parts.iter().all(|p| is_safe_component(p)) {
        return Err(format!("hf_repo contains invalid characters: {hf_repo:?}"));
    }
    if !is_safe_component(tag) {
        return Err(format!("tag contains invalid characters: {tag:?}"));
    }
    Ok(())
}

/// Builds a real `hf.co/<repo>:<tag>` pull target from user-supplied input
/// -- the real "user defined model download links" mechanism: any real,
/// public, anonymously-pullable HF GGUF repo works here, not just a
/// `CURATED_MODELS` entry. Normalizes common pasted-link prefixes first
/// (see `normalize_hf_repo_input`), then validates before ever building the
/// target string.
pub fn custom_pull_target(hf_repo_input: &str, tag: &str) -> Result<String, String> {
    let hf_repo = normalize_hf_repo_input(hf_repo_input);
    validate_custom_repo_and_tag(&hf_repo, tag)?;
    Ok(format!("hf.co/{}:{}", hf_repo, tag.trim()))
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

    #[test]
    fn curated_models_covers_a_real_range_of_sizes_and_families() {
        // A real, coarse sanity check that this is genuinely a broad
        // curated list, not just the original 4 renamed -- at least a
        // dozen entries, spanning several distinct real HF orgs.
        assert!(
            CURATED_MODELS.len() >= 12,
            "expected a real, broad curated list, found {}",
            CURATED_MODELS.len()
        );
        let orgs: std::collections::HashSet<&str> = CURATED_MODELS
            .iter()
            .map(|m| m.hf_repo.split('/').next().unwrap())
            .collect();
        assert!(
            orgs.len() >= 2,
            "expected models from more than one real HF org, found {orgs:?}"
        );
    }

    #[test]
    fn normalize_hf_repo_input_strips_real_common_prefixes() {
        assert_eq!(
            normalize_hf_repo_input("https://huggingface.co/bartowski/Foo-GGUF"),
            "bartowski/Foo-GGUF"
        );
        assert_eq!(
            normalize_hf_repo_input("http://huggingface.co/bartowski/Foo-GGUF/"),
            "bartowski/Foo-GGUF"
        );
        assert_eq!(
            normalize_hf_repo_input("huggingface.co/bartowski/Foo-GGUF"),
            "bartowski/Foo-GGUF"
        );
        assert_eq!(
            normalize_hf_repo_input("hf.co/bartowski/Foo-GGUF"),
            "bartowski/Foo-GGUF"
        );
        assert_eq!(
            normalize_hf_repo_input("  bartowski/Foo-GGUF  "),
            "bartowski/Foo-GGUF"
        );
    }

    #[test]
    fn validate_custom_repo_and_tag_accepts_a_real_well_formed_pair() {
        assert!(validate_custom_repo_and_tag("bartowski/Foo-GGUF", "Q4_K_M").is_ok());
    }

    #[test]
    fn validate_custom_repo_and_tag_rejects_malformed_input() {
        assert!(validate_custom_repo_and_tag("", "Q4_K_M").is_err());
        assert!(validate_custom_repo_and_tag("bartowski/Foo-GGUF", "").is_err());
        assert!(validate_custom_repo_and_tag("not-a-repo-no-slash", "Q4_K_M").is_err());
        assert!(validate_custom_repo_and_tag("a/b/c", "Q4_K_M").is_err());
        assert!(validate_custom_repo_and_tag("org/repo name", "Q4_K_M").is_err());
        assert!(validate_custom_repo_and_tag("org/repo;rm -rf", "Q4_K_M").is_err());
        // A leading `-` could otherwise be interpreted by `ollama` itself
        // as a real CLI flag rather than a literal repo/tag.
        assert!(validate_custom_repo_and_tag("-flag/repo", "Q4_K_M").is_err());
        assert!(validate_custom_repo_and_tag("org/repo", "-flag").is_err());
    }

    #[test]
    fn custom_pull_target_normalizes_validates_and_builds_the_real_ollama_syntax() {
        let target =
            custom_pull_target("https://huggingface.co/bartowski/Foo-GGUF/", "Q4_K_M").unwrap();
        assert_eq!(target, "hf.co/bartowski/Foo-GGUF:Q4_K_M");

        assert!(custom_pull_target("not-a-real-repo-shape", "Q4_K_M").is_err());
    }

    #[test]
    fn pull_target_and_custom_pull_target_agree_on_the_real_ollama_syntax() {
        // Confirms both real entry points -- a curated model and a
        // user-defined custom repo/tag -- produce the identical `hf.co/`
        // target shape `spawn_pull_target` ultimately shells out with, not
        // two subtly different string formats.
        let model = CURATED_MODELS[0];
        let via_curated = pull_target(&model);
        let via_custom = custom_pull_target(model.hf_repo, model.tag).unwrap();
        assert_eq!(via_curated, via_custom);
    }
}
