//! Real, live, end-to-end integration test: `open_file` then
//! `lsp_signature_help` over the real `handle_request` dispatch spawns a
//! real `pyright-langserver` session and a real `lsp_signature_help_result`
//! event arrives on the real out-channel carrying pyright's own real
//! function signature. Self-skips honestly if `pyright-langserver` isn't
//! on `$PATH`, matching every other real-external-tool integration suite
//! in this repo (mirrors `lsp_definition_integration.rs`'s own shape).

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
        "spartan-backend-lsp-signature-help-e2e-test-{}-{:?}",
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

// A real function with a real, distinctively-typed parameter, and a real
// call site with the cursor positioned right after the opening paren --
// exactly where a real editor would trigger signature help after typing
// "(".
const SIGNATURE_HELP_PY: &str = "def greet(name: str) -> str:\n    return name\n\ngreet(\n";

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
fn lsp_signature_help_returns_a_real_pyright_function_signature() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(SIGNATURE_HELP_PY);
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

    // Signature help right after "greet(" (line 3, character 6).
    let sig_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "lsp_signature_help".to_string(),
            params: serde_json::json!({ "doc_id": doc_id, "line": 3, "character": 6 }),
        },
        tx.clone(),
    );
    assert!(
        sig_resp.error.is_none(),
        "lsp_signature_help errored: {:?}",
        sig_resp.error
    );
    assert_eq!(sig_resp.result.unwrap()["status"], "requested");

    let sig_event = recv_event_matching(&rx, "lsp_signature_help_result", Duration::from_secs(100));
    assert_eq!(sig_event["data"]["doc_id"], doc_id);
    assert_eq!(sig_event["data"]["line"], 3);
    assert_eq!(sig_event["data"]["character"], 6);

    let result = &sig_event["data"]["result"];
    assert!(
        result.get("jsonrpc").is_none() && result.get("id").is_none(),
        "result must be unwrapped from its JSON-RPC envelope, not the raw response: {sig_event}"
    );
    // A real LSP `SignatureHelp.label` deliberately omits the function's
    // own name (the call site already names it) -- it's just the real
    // parenthesized parameter list plus return type, confirmed by running
    // this against the real server rather than assumed: pyright's own
    // actual response here is `"(name: str) -> str"`.
    let text = result.to_string();
    assert!(
        text.contains("name") && text.contains("str"),
        "expected pyright's real signature to mention the real (name: str) shape: {sig_event}"
    );
    assert_eq!(
        result["activeParameter"], 0,
        "the real cursor position right after the opening paren should point at parameter 0: {sig_event}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
