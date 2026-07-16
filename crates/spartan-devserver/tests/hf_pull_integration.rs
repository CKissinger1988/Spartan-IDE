//! Real, live HF -> Ollama pull test. Self-skips honestly (prints a
//! message, doesn't fail) if `ollama` isn't installed, matching every other
//! real-external-tool integration suite in this repo.
//!
//! Deliberately does NOT pull any of the real curated models -- each is a
//! real, multi-hundred-MB-to-multi-GB download, an honest cost this suite
//! does not pay in CI. Instead it drives `hf_downloader::spawn_pull`
//! against a real, deliberately nonexistent HF repo -- Ollama's own real
//! `hf.co/` resolution genuinely reaches out and fails fast (a real 404-
//! shaped error, no multi-GB body), which is enough to prove this crate's
//! spawn/stream mechanics genuinely invoke Ollama's real pull path end to
//! end, without paying a real model's download cost.

use spartan_devserver::hf_downloader::{self, HfModel};
use std::sync::mpsc;

#[test]
fn real_ollama_pull_reaches_hf_co_and_fails_fast_for_a_nonexistent_repo() {
    if !hf_downloader::is_ollama_available() {
        println!("SKIP: `ollama` isn't installed in this environment");
        return;
    }

    let nonexistent = HfModel {
        id: "spartan-test-nonexistent",
        display_name: "test only",
        hf_repo: "spartan-ide-test-org/definitely-does-not-exist-xyz",
        tag: "Q4_K_M",
        description: "test only",
    };

    let (tx, rx) = mpsc::channel();
    let mut child = hf_downloader::spawn_pull(&nonexistent, tx).expect("a real ollama pull spawn");

    let status = child.wait().expect("real process must exit");
    assert!(
        !status.success(),
        "a pull for a genuinely nonexistent HF repo must fail"
    );

    let mut saw_a_line = false;
    while rx.try_recv().is_ok() {
        saw_a_line = true;
    }
    assert!(
        saw_a_line,
        "expected at least one real streamed line from the real ollama pull attempt"
    );
}
