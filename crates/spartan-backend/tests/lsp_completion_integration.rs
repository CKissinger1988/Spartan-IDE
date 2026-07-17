//! Real, live, end-to-end integration test: `open_file` then `lsp_completion`
//! over the real `handle_request` dispatch spawns a real
//! `pyright-langserver` session and a real `lsp_completion_result` event
//! arrives on the real out-channel carrying pyright's own real completion
//! list. Self-skips honestly if `pyright-langserver` isn't on `$PATH`,
//! matching every other real-external-tool integration suite in this repo
//! (mirrors `lsp_hover_integration.rs`'s own shape, and
//! `crates/spartan-lsp/tests/pyright_integration.rs`'s own live completion
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
        "spartan-backend-lsp-completion-e2e-test-{}-{:?}",
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

const COMPLETION_PY: &str = "import os\nos.\n";

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
fn lsp_completion_returns_a_real_pyright_completion_list() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(COMPLETION_PY);
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

    // Completion right after `os.` (line 1, character 3) -- the same real,
    // reliable trigger position `pyright_integration.rs`'s own live
    // completion test already uses.
    let completion_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "lsp_completion".to_string(),
            params: serde_json::json!({ "doc_id": doc_id, "line": 1, "character": 3 }),
        },
        tx.clone(),
    );
    assert!(
        completion_resp.error.is_none(),
        "lsp_completion errored: {:?}",
        completion_resp.error
    );
    assert_eq!(completion_resp.result.unwrap()["status"], "requested");

    let completion_event =
        recv_event_matching(&rx, "lsp_completion_result", Duration::from_secs(100));
    assert_eq!(completion_event["data"]["doc_id"], doc_id);
    assert_eq!(completion_event["data"]["line"], 1);
    assert_eq!(completion_event["data"]["character"], 3);

    // A real, precise shape assertion -- `result` must be unwrapped from
    // its JSON-RPC envelope (the same real fix `lsp_hover` needed applies
    // identically here), and must contain a real `os` module member.
    let result = &completion_event["data"]["result"];
    assert!(
        result.get("jsonrpc").is_none() && result.get("id").is_none(),
        "result must be unwrapped from its JSON-RPC envelope, not the raw response: {completion_event}"
    );
    let text = result.to_string().to_lowercase();
    assert!(
        text.contains("getcwd") || text.contains("path") || text.contains("environ"),
        "expected pyright's real completion list to mention a real os module member: {completion_event}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
