//! Real, live, end-to-end integration test: `open_file` over the real
//! `handle_request` dispatch spawns a real `pyright-langserver` session,
//! a real `lsp_diagnostics` event arrives on the real out-channel, a real
//! `edit` call fixing the file produces a real second event with the
//! diagnostic cleared, and `close_file` doesn't hang. Self-skips honestly
//! if `pyright-langserver` isn't on `$PATH`, matching every other real-
//! external-tool integration suite in this repo (rust-analyzer isn't
//! installed in this environment -- see `crates/spartan-lsp/tests/
//! pyright_integration.rs` for the same real cross-adapter reasoning).

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
        "spartan-backend-lsp-e2e-test-{}-{:?}",
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

const BROKEN_PY: &str = "x: int = 1\nx.upper()\n";
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
        // Any other real event (e.g. a stray one from a prior run) is
        // skipped, not treated as a failure.
    }
}

#[test]
fn open_file_spawns_a_real_lsp_session_and_edit_clears_a_real_diagnostic() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(BROKEN_PY);
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

    // The real first diagnostics event, from the real initial pyright
    // analysis pass.
    let first = recv_event_matching(&rx, "lsp_diagnostics", Duration::from_secs(60));
    assert_eq!(first["data"]["doc_id"], doc_id);
    let diags = first["data"]["diagnostics"].as_array().unwrap();
    assert!(!diags.is_empty(), "expected a real diagnostic: {first}");
    assert!(
        diags[0]["message"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("upper"),
        "expected the real upper() diagnostic: {first}"
    );

    // A real edit through the normal `edit` IPC method, replacing the
    // whole buffer with the fixed content -- this must dispatch a real
    // didChange to the live session and eventually clear the diagnostic.
    let edit_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "edit".to_string(),
            params: serde_json::json!({
                "doc_id": doc_id,
                "start_char": 0,
                "end_char": BROKEN_PY.chars().count(),
                "text": FIXED_PY,
            }),
        },
        tx.clone(),
    );
    assert!(
        edit_resp.error.is_none(),
        "edit errored: {:?}",
        edit_resp.error
    );

    let second = recv_event_matching(&rx, "lsp_diagnostics", Duration::from_secs(10));
    assert_eq!(second["data"]["doc_id"], doc_id);
    assert!(
        second["data"]["diagnostics"].as_array().unwrap().is_empty(),
        "expected the real diagnostic to clear after the fix: {second}"
    );

    // close_file must return promptly (non-blocking teardown -- see
    // `LspSession::signal_shutdown`'s own doc comment) rather than waiting
    // out the real ~7s worst-case graceful LSP shutdown sequence.
    let close_started = std::time::Instant::now();
    let close_resp = handle_request(
        &state,
        Request {
            id: 3,
            method: "close_file".to_string(),
            params: serde_json::json!({ "doc_id": doc_id }),
        },
        tx,
    );
    assert!(close_resp.error.is_none());
    assert!(
        close_started.elapsed() < Duration::from_secs(1),
        "close_file must not block on real LSP shutdown"
    );

    std::fs::remove_dir_all(&dir).ok();
}
