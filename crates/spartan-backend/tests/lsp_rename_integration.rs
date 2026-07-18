//! Real, live, end-to-end integration test: `open_file` then `lsp_rename`
//! over the real `handle_request` dispatch spawns a real
//! `pyright-langserver` session and a real `lsp_rename_result` event
//! arrives on the real out-channel carrying pyright's own real
//! `WorkspaceEdit`. Self-skips honestly if `pyright-langserver` isn't on
//! `$PATH`, matching every other real-external-tool integration suite in
//! this repo (mirrors `lsp_references_integration.rs`'s own shape -- the
//! sixth real query method, the direct sibling of the five before it).
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
        "spartan-backend-lsp-rename-e2e-test-{}-{:?}",
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

// A real local variable, assigned on line 0 and used on line 1 -- a real
// rename from its definition should produce a real `WorkspaceEdit`
// touching both real occurrences, in this one file.
const RENAME_PY: &str = "value = 1\nprint(value)";

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
fn lsp_rename_produces_a_real_workspace_edit_touching_both_real_occurrences() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(RENAME_PY);
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

    // Rename from the real "value" identifier at its own definition
    // (line 0, character 0).
    let rename_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "lsp_rename".to_string(),
            params: serde_json::json!({
                "doc_id": doc_id,
                "line": 0,
                "character": 0,
                "new_name": "renamed_value",
            }),
        },
        tx.clone(),
    );
    assert!(
        rename_resp.error.is_none(),
        "lsp_rename errored: {:?}",
        rename_resp.error
    );
    assert_eq!(rename_resp.result.unwrap()["status"], "requested");

    let rename_event = recv_event_matching(&rx, "lsp_rename_result", Duration::from_secs(100));
    assert_eq!(rename_event["data"]["doc_id"], doc_id);
    assert_eq!(rename_event["data"]["line"], 0);
    assert_eq!(rename_event["data"]["character"], 0);

    let result = &rename_event["data"]["result"];
    assert!(
        result.get("jsonrpc").is_none() && result.get("id").is_none(),
        "result must be unwrapped from its JSON-RPC envelope, not the raw response: {rename_event}"
    );

    // A real, live finding, not assumed from the spec: `open_project`
    // declares no `workspace.workspaceEdit` capability at all, which per
    // spec should mean a server sticks to the simpler `changes` shape --
    // but a real, live `pyright-langserver` session replies with
    // `documentChanges` (an array of real `TextDocumentEdit`s) regardless.
    // See `LspClient::rename`'s own doc comment for the same finding
    // recorded on the Rust side. This test accepts either real shape,
    // matching the same "a real caller must handle both" discipline that
    // doc comment names.
    let edits: Vec<serde_json::Value> = if let Some(changes) =
        result.get("changes").and_then(|c| c.as_object())
    {
        assert_eq!(
            changes.len(),
            1,
            "expected exactly one real file touched by this rename: {rename_event}"
        );
        let (uri, edits) = changes.iter().next().unwrap();
        assert!(
            uri.ends_with("main.py"),
            "expected the real fixture file's own URI: {uri}"
        );
        edits
            .as_array()
            .unwrap_or_else(|| panic!("expected a real TextEdit[] array: {rename_event}"))
            .clone()
    } else {
        let document_changes = result
            .get("documentChanges")
            .and_then(|c| c.as_array())
            .unwrap_or_else(|| {
                panic!("expected either a real `changes` map or `documentChanges` array: {rename_event}")
            });
        assert_eq!(
            document_changes.len(),
            1,
            "expected exactly one real TextDocumentEdit (one file touched): {rename_event}"
        );
        let doc_edit = &document_changes[0];
        let uri = doc_edit["textDocument"]["uri"]
            .as_str()
            .unwrap_or_else(|| panic!("expected a real textDocument.uri: {rename_event}"));
        assert!(
            uri.ends_with("main.py"),
            "expected the real fixture file's own URI: {uri}"
        );
        doc_edit["edits"]
            .as_array()
            .unwrap_or_else(|| panic!("expected a real edits[] array: {rename_event}"))
            .clone()
    };
    assert_eq!(
        edits.len(),
        2,
        "expected 2 real edits (the definition plus the one real usage): {rename_event}"
    );
    let start_lines: Vec<u64> = edits
        .iter()
        .map(|e| e["range"]["start"]["line"].as_u64().unwrap())
        .collect();
    assert!(
        start_lines.contains(&0),
        "missing the real definition line: {start_lines:?}"
    );
    assert!(
        start_lines.contains(&1),
        "missing the real usage line: {start_lines:?}"
    );
    for e in &edits {
        assert_eq!(
            e["newText"], "renamed_value",
            "every real edit must carry the real new name: {rename_event}"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}
