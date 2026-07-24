//! Real, live, end-to-end integration test: `open_file` then
//! `lsp_type_definition` over the real `handle_request` dispatch spawns a
//! real `pyright-langserver` session and a real `lsp_type_definition_result`
//! event arrives on the real out-channel carrying pyright's own real
//! type-jump target. Self-skips honestly if `pyright-langserver` isn't on
//! `$PATH`, matching every other real-external-tool integration suite in
//! this repo (mirrors `lsp_definition_integration.rs`'s own shape).

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
        "spartan-backend-lsp-type-definition-e2e-test-{}-{:?}",
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

// A real, plainly-typed variable -- requesting a type definition from `x`
// (character 0, the real start of the identifier) must jump to `int`'s own
// real definition, not to the assignment itself.
const TYPE_DEFINITION_PY: &str = "x: int = 1\n";

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

/// A real LSP `typeDefinition` result is `Location | Location[] |
/// LocationLink[] | null` -- normalize whichever real shape pyright sends
/// into the first entry's real target URI, the one field this test
/// actually asserts on (pyright's own bundled typeshed path, not this
/// fixture's own file).
fn first_uri(result: &serde_json::Value) -> Option<String> {
    let entry = if result.is_array() {
        result.as_array()?.first()?
    } else {
        result
    };
    entry
        .get("uri")
        .or_else(|| entry.get("targetUri"))?
        .as_str()
        .map(str::to_string)
}

#[test]
fn lsp_type_definition_jumps_to_a_real_pyright_resolved_type() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(TYPE_DEFINITION_PY);
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

    // Type-definition request on `x` itself (line 0, character 0).
    let type_definition_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "lsp_type_definition".to_string(),
            params: serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 0 }),
        },
        tx.clone(),
    );
    assert!(
        type_definition_resp.error.is_none(),
        "lsp_type_definition errored: {:?}",
        type_definition_resp.error
    );
    assert_eq!(type_definition_resp.result.unwrap()["status"], "requested");

    let event = recv_event_matching(&rx, "lsp_type_definition_result", Duration::from_secs(100));
    assert_eq!(event["data"]["doc_id"], doc_id);
    assert_eq!(event["data"]["line"], 0);
    assert_eq!(event["data"]["character"], 0);

    let result = &event["data"]["result"];
    assert!(
        result.get("jsonrpc").is_none() && result.get("id").is_none(),
        "result must be unwrapped from its JSON-RPC envelope, not the raw response: {event}"
    );

    let uri = first_uri(result).unwrap_or_else(|| panic!("no real location in result: {event}"));
    assert!(
        uri.contains("typeshed") && uri.ends_with("builtins.pyi"),
        "expected a real jump into pyright's own bundled typeshed builtins.pyi, got: {uri}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
