//! Real, live, end-to-end integration test: `open_file` then `lsp_hover`
//! over the real `handle_request` dispatch spawns a real
//! `pyright-langserver` session and, after the real first diagnostics
//! event, a real `lsp_hover_result` event arrives on the real out-channel
//! carrying pyright's own real hover response. Self-skips honestly if
//! `pyright-langserver` isn't on `$PATH`, matching every other real-
//! external-tool integration suite in this repo (mirrors
//! `crates/spartan-lsp/tests/pyright_integration.rs`'s own live hover
//! test, one layer up at the real IPC dispatch boundary).

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
        "spartan-backend-lsp-hover-e2e-test-{}-{:?}",
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

const FIXED_PY: &str = "x: int = 1\nprint(x)\n";

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
fn lsp_hover_returns_a_real_pyright_hover_response() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(FIXED_PY);
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

    // Hovers on the "int" annotation itself (character 4, the middle of
    // "int" in "x: int = 1"), not on the variable name at character 0 --
    // a real, live-testing-caught correction: pyright's own real hover for
    // a bare variable reports its *narrowed literal type* ("(variable) x:
    // Literal[1]"), not "int", for a simple integer-literal assignment
    // even with an explicit `: int` annotation present. That's genuine,
    // correct pyright behavior, not a product bug -- the original version
    // of this test asserted `contains("int")` while hovering at character
    // 0 and failed for exactly this reason once the real envelope-
    // unwrapping bug below was fixed and a real hover payload started
    // reaching the assertion for the first time.
    let hover_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "lsp_hover".to_string(),
            params: serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 4 }),
        },
        tx.clone(),
    );
    assert!(
        hover_resp.error.is_none(),
        "lsp_hover errored: {:?}",
        hover_resp.error
    );
    assert_eq!(hover_resp.result.unwrap()["status"], "requested");

    // A real, generous bound matching `LspSession::request_hover`'s own
    // internal worst case (indexing wait + the query's own request
    // timeout) -- a query issued immediately after `open_file` can
    // legitimately queue behind the server's real initial indexing pass.
    let hover_event = recv_event_matching(&rx, "lsp_hover_result", Duration::from_secs(100));
    assert_eq!(hover_event["data"]["doc_id"], doc_id);
    assert_eq!(hover_event["data"]["line"], 0);
    assert_eq!(hover_event["data"]["character"], 4);

    // A real, precise shape assertion -- not a loose stringify-and-
    // `contains` check, which would have passed even with the raw
    // JSON-RPC envelope this pass found and fixed still leaking through
    // (its own `"jsonrpc"`/`"id"` fields don't contain "int", but the
    // earlier, looser version of this test never actually proved the
    // *unwrapped* shape a real frontend needs). `result` must be exactly
    // the inner LSP hover payload -- a `contents` field directly on it,
    // no `result`/`jsonrpc`/`id` envelope wrapping it.
    let result = &hover_event["data"]["result"];
    assert!(
        result.get("jsonrpc").is_none() && result.get("id").is_none(),
        "result must be unwrapped from its JSON-RPC envelope, not the raw response: {hover_event}"
    );
    let contents_text = result["contents"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a real contents.value field: {hover_event}"));
    assert!(
        contents_text.to_lowercase().contains("int"),
        "expected pyright's real hover to mention the variable's `int` type: {hover_event}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
