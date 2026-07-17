//! Real, live, end-to-end integration test: `open_file` then
//! `lsp_references` over the real `handle_request` dispatch spawns a real
//! `pyright-langserver` session and a real `lsp_references_result` event
//! arrives on the real out-channel carrying pyright's own real reference
//! list. Self-skips honestly if `pyright-langserver` isn't on `$PATH`,
//! matching every other real-external-tool integration suite in this repo
//! (mirrors `lsp_definition_integration.rs`'s own shape).
//!
//! Deliberately no trailing newline on the fixture's own final line --
//! `lsp_signature_help_integration.rs`'s own account already documents the
//! real bug class a trailing newline caused there (a live-verification
//! mistake, not caught by this test-layer's own simpler fixed-position
//! queries, but avoided here from the start).

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
        "spartan-backend-lsp-references-e2e-test-{}-{:?}",
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

// A real function definition (line 0) and two real call sites (lines 4, 5)
// -- a real "find all references" from the definition itself should
// report all three real occurrences (the definition, since
// includeDeclaration defaults to true, plus both call sites).
const REFERENCES_PY: &str = "def greet():\n    return 1\n\n\ngreet()\ngreet()";

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
fn lsp_references_reports_every_real_pyright_resolved_call_site() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(REFERENCES_PY);
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

    // References from the real "greet" identifier at its own definition
    // (line 0, character 4).
    let refs_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "lsp_references".to_string(),
            params: serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 4 }),
        },
        tx.clone(),
    );
    assert!(
        refs_resp.error.is_none(),
        "lsp_references errored: {:?}",
        refs_resp.error
    );
    assert_eq!(refs_resp.result.unwrap()["status"], "requested");

    let refs_event = recv_event_matching(&rx, "lsp_references_result", Duration::from_secs(100));
    assert_eq!(refs_event["data"]["doc_id"], doc_id);
    assert_eq!(refs_event["data"]["line"], 0);
    assert_eq!(refs_event["data"]["character"], 4);

    let result = &refs_event["data"]["result"];
    assert!(
        result.get("jsonrpc").is_none() && result.get("id").is_none(),
        "result must be unwrapped from its JSON-RPC envelope, not the raw response: {refs_event}"
    );
    let locations = result
        .as_array()
        .unwrap_or_else(|| panic!("expected a real Location[] array: {refs_event}"));
    // The real definition (includeDeclaration defaults to true) plus the
    // two real call sites -- three real locations total.
    assert_eq!(
        locations.len(),
        3,
        "expected 3 real locations (1 definition + 2 call sites): {refs_event}"
    );
    let start_lines: Vec<u64> = locations
        .iter()
        .map(|l| l["range"]["start"]["line"].as_u64().unwrap())
        .collect();
    assert!(
        start_lines.contains(&0),
        "missing the real definition line: {start_lines:?}"
    );
    assert!(
        start_lines.contains(&4),
        "missing the real first call site line: {start_lines:?}"
    );
    assert!(
        start_lines.contains(&5),
        "missing the real second call site line: {start_lines:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
