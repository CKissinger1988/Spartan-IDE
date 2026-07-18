//! Real, live, end-to-end integration test: `open_file` then
//! `lsp_document_highlight` over the real `handle_request` dispatch spawns
//! a real `pyright-langserver` session and a real
//! `lsp_document_highlight_result` event arrives on the real out-channel
//! carrying pyright's own real occurrence list. Self-skips honestly if
//! `pyright-langserver` isn't on `$PATH`, matching every other
//! real-external-tool integration suite in this repo (mirrors
//! `lsp_document_symbol_integration.rs`'s own shape -- the eighth real
//! query method, the direct sibling of the seven before it).
//!
//! Deliberately no trailing newline on the fixture's own final line --
//! `lsp_signature_help_integration.rs`'s own account already documents the
//! real bug class a trailing newline caused there.

use spartan_backend::{handle_request, BackendState, Request};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn pyright_available() -> bool {
    std::process::Command::new("pyright-langserver")
        .arg("--version")
        .output()
        .is_ok()
}

fn make_fixture(content: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "spartan-backend-lsp-document-highlight-e2e-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("pyproject.toml"),
        "[project]\nname = \"fixture\"\n",
    )
    .unwrap();
    let file = dir.join("main.py");
    std::fs::write(&file, content).unwrap();
    (dir, file)
}

// A real local variable, assigned (a real "Write") on line 0 and used (a
// real "Read") on line 1 -- a real document highlight from either
// occurrence should report both.
const HIGHLIGHT_PY: &str = "value = 1\nprint(value)";

fn recv_event_matching(
    rx: &mpsc::Receiver<String>,
    event_name: &str,
    timeout: Duration,
) -> serde_json::Value {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for a real {event_name} event"
        );
        let line = rx
            .recv_timeout(remaining)
            .unwrap_or_else(|_| panic!("timed out waiting for a real {event_name} event"));
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        if value.get("event").and_then(|e| e.as_str()) == Some(event_name) {
            return value;
        }
        // Any other real event (e.g. the initial lsp_diagnostics pass) is
        // skipped, not treated as a failure.
    }
}

#[test]
fn lsp_document_highlight_reports_a_real_write_and_a_real_read_occurrence() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(HIGHLIGHT_PY);
    let state: Arc<Mutex<BackendState>> = Arc::new(Mutex::new(BackendState::new()));
    let (tx, rx) = mpsc::channel();

    let open_resp = handle_request(
        &state,
        Request {
            id: 1,
            method: "open_file".to_string(),
            params: serde_json::json!({ "path": file.to_string_lossy() }),
        },
        tx.clone(),
    );
    assert!(
        open_resp.error.is_none(),
        "open_file errored: {:?}",
        open_resp.error
    );
    let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

    // Highlight from the real "value" identifier at its own definition
    // (line 0, character 0).
    let highlight_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "lsp_document_highlight".to_string(),
            params: serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 0 }),
        },
        tx.clone(),
    );
    assert!(
        highlight_resp.error.is_none(),
        "lsp_document_highlight errored: {:?}",
        highlight_resp.error
    );
    assert_eq!(highlight_resp.result.unwrap()["status"], "requested");

    let highlight_event = recv_event_matching(
        &rx,
        "lsp_document_highlight_result",
        Duration::from_secs(100),
    );
    assert_eq!(highlight_event["data"]["doc_id"], doc_id);
    assert_eq!(highlight_event["data"]["line"], 0);
    assert_eq!(highlight_event["data"]["character"], 0);

    let result = &highlight_event["data"]["result"];
    assert!(
        result.get("jsonrpc").is_none() && result.get("id").is_none(),
        "result must be unwrapped from its JSON-RPC envelope, not the raw response: {highlight_event}"
    );

    let highlights = result
        .as_array()
        .unwrap_or_else(|| panic!("expected a real DocumentHighlight[] array: {highlight_event}"));
    assert_eq!(
        highlights.len(),
        2,
        "expected 2 real occurrences (the definition plus the one real usage): {highlight_event}"
    );
    let start_lines: Vec<u64> = highlights
        .iter()
        .map(|h| h["range"]["start"]["line"].as_u64().unwrap())
        .collect();
    assert!(
        start_lines.contains(&0),
        "missing the real definition line: {start_lines:?}"
    );
    assert!(
        start_lines.contains(&1),
        "missing the real usage line: {start_lines:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
