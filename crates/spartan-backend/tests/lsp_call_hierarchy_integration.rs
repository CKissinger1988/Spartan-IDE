//! Real, live, end-to-end integration test: `open_file` then
//! `lsp_call_hierarchy` over the real `handle_request` dispatch spawns a real
//! `pyright-langserver` session and a real `lsp_call_hierarchy_result` event
//! arrives carrying pyright's own real incoming-calls list. Self-skips
//! honestly if `pyright-langserver` isn't on `$PATH`, matching every other
//! real-external-tool integration suite in this repo (mirrors
//! `lsp_references_integration.rs`'s own shape).
//!
//! Unlike every other LSP query method here, call hierarchy is a real
//! two-request protocol under the hood (`prepareCallHierarchy` then
//! `callHierarchy/incomingCalls`) -- see `LspClient::incoming_calls`. This
//! exercises the whole thing through one real IPC call.

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
        "spartan-backend-lsp-callhierarchy-e2e-test-{}-{:?}",
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

// `greet` (defined line 0) is called from within `caller` (line 5). Incoming
// calls of `greet` must report `caller` as the one real caller.
const CALL_HIERARCHY_PY: &str = "def greet():\n    return 1\n\n\ndef caller():\n    return greet()";

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
    }
}

#[test]
fn lsp_call_hierarchy_reports_the_real_incoming_caller() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(CALL_HIERARCHY_PY);
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

    // Incoming calls of the real "greet" identifier at its own definition
    // (line 0, character 4).
    let ch_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "lsp_call_hierarchy".to_string(),
            params: serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 4 }),
        },
        tx.clone(),
    );
    assert!(
        ch_resp.error.is_none(),
        "lsp_call_hierarchy errored: {:?}",
        ch_resp.error
    );
    assert_eq!(ch_resp.result.unwrap()["status"], "requested");

    let ch_event = recv_event_matching(&rx, "lsp_call_hierarchy_result", Duration::from_secs(100));
    assert_eq!(ch_event["data"]["doc_id"], doc_id);

    let result = &ch_event["data"]["result"];
    assert!(
        result.get("jsonrpc").is_none() && result.get("id").is_none(),
        "result must be unwrapped from its JSON-RPC envelope: {ch_event}"
    );
    let calls = result
        .as_array()
        .unwrap_or_else(|| panic!("expected a real CallHierarchyIncomingCall[] array: {ch_event}"));
    assert_eq!(
        calls.len(),
        1,
        "expected exactly one real incoming call (from `caller`): {ch_event}"
    );
    assert_eq!(
        calls[0]["from"]["name"].as_str(),
        Some("caller"),
        "the one real caller of greet must be `caller`: {ch_event}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn lsp_call_hierarchy_outgoing_reports_the_real_callee() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(CALL_HIERARCHY_PY);
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
    assert!(open_resp.error.is_none());
    let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

    // Outgoing calls from the real "caller" definition (line 4, character 4)
    // -- it calls `greet`, which must be the one real outgoing callee.
    let ch_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "lsp_call_hierarchy".to_string(),
            params: serde_json::json!({
                "doc_id": doc_id, "line": 4, "character": 4, "direction": "outgoing"
            }),
        },
        tx.clone(),
    );
    assert!(
        ch_resp.error.is_none(),
        "lsp_call_hierarchy (outgoing) errored: {:?}",
        ch_resp.error
    );

    let ch_event = recv_event_matching(&rx, "lsp_call_hierarchy_result", Duration::from_secs(100));
    assert_eq!(ch_event["data"]["direction"], "outgoing");
    let result = &ch_event["data"]["result"];
    let calls = result
        .as_array()
        .unwrap_or_else(|| panic!("expected a real CallHierarchyOutgoingCall[] array: {ch_event}"));
    // The real callee is `greet` (each outgoing call carries the callee in
    // `to`, not `from`).
    assert!(
        calls
            .iter()
            .any(|c| c["to"]["name"].as_str() == Some("greet")),
        "the real outgoing callee of caller must include `greet`: {ch_event}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
