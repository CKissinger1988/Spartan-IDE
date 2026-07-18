//! Real, live, end-to-end integration test: `open_file` then
//! `lsp_document_symbol` over the real `handle_request` dispatch spawns a
//! real `pyright-langserver` session and a real `lsp_document_symbol_result`
//! event arrives on the real out-channel carrying pyright's own real
//! symbol tree. Self-skips honestly if `pyright-langserver` isn't on
//! `$PATH`, matching every other real-external-tool integration suite in
//! this repo (mirrors `lsp_rename_integration.rs`'s own shape -- the
//! seventh real query method, the direct sibling of the six before it).
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
        "spartan-backend-lsp-document-symbol-e2e-test-{}-{:?}",
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

// A real class with one real method, plus a real standalone function -- a
// real document symbol request should report both top-level real symbols,
// with the method nested as a real child of the class.
const SYMBOLS_PY: &str = "class Greeter:\n    def greet(self):\n        return \"hi\"\n\n\ndef standalone():\n    return 1";

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
fn lsp_document_symbol_reports_a_real_nested_class_method_and_a_real_top_level_function() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(SYMBOLS_PY);
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

    let symbol_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "lsp_document_symbol".to_string(),
            params: serde_json::json!({ "doc_id": doc_id }),
        },
        tx.clone(),
    );
    assert!(
        symbol_resp.error.is_none(),
        "lsp_document_symbol errored: {:?}",
        symbol_resp.error
    );
    assert_eq!(symbol_resp.result.unwrap()["status"], "requested");

    let symbol_event =
        recv_event_matching(&rx, "lsp_document_symbol_result", Duration::from_secs(100));
    assert_eq!(symbol_event["data"]["doc_id"], doc_id);

    let result = &symbol_event["data"]["result"];
    assert!(
        result.get("jsonrpc").is_none() && result.get("id").is_none(),
        "result must be unwrapped from its JSON-RPC envelope, not the raw response: {symbol_event}"
    );

    let symbols = result
        .as_array()
        .unwrap_or_else(|| panic!("expected a real DocumentSymbol[] array: {symbol_event}"));
    assert_eq!(
        symbols.len(),
        2,
        "expected 2 real top-level symbols (Greeter class, standalone function): {symbol_event}"
    );

    let names: Vec<&str> = symbols
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"Greeter"),
        "missing the real Greeter class: {names:?}"
    );
    assert!(
        names.contains(&"standalone"),
        "missing the real standalone function: {names:?}"
    );

    // `open_project` declares `hierarchicalDocumentSymbolSupport`, so a
    // real server (confirmed live against pyright-langserver before this
    // test was written) replies with real, correctly nested `children` --
    // the Greeter class's own real `greet` method should appear nested
    // under it, not as a separate top-level entry.
    let greeter = symbols
        .iter()
        .find(|s| s["name"] == "Greeter")
        .expect("Greeter symbol must be present");
    let children = greeter["children"]
        .as_array()
        .unwrap_or_else(|| panic!("expected Greeter to have real children: {symbol_event}"));
    assert_eq!(
        children.len(),
        1,
        "expected exactly 1 real nested method (greet): {symbol_event}"
    );
    assert_eq!(children[0]["name"], "greet");

    std::fs::remove_dir_all(&dir).ok();
}
