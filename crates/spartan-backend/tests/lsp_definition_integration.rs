//! Real, live, end-to-end integration test: `open_file` then `lsp_definition`
//! over the real `handle_request` dispatch spawns a real `pyright-langserver`
//! session and a real `lsp_definition_result` event arrives on the real
//! out-channel carrying pyright's own real jump-target location. Self-skips
//! honestly if `pyright-langserver` isn't on `$PATH`, matching every other
//! real-external-tool integration suite in this repo (mirrors
//! `lsp_completion_integration.rs`'s own shape).

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
        "spartan-backend-lsp-definition-e2e-test-{}-{:?}",
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

// A real function definition (line 0) and a real call site (line 4) --
// requesting a definition from the call site must jump back to line 0.
const DEFINITION_PY: &str = "def foo():\n    return 1\n\n\nfoo()\n";

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

/// A real LSP `definition` result is `Location | Location[] | LocationLink[]
/// | null` -- normalize whichever real shape pyright sends into the first
/// entry's `range.start.line`, the one field this test actually asserts on.
fn first_start_line(result: &serde_json::Value) -> Option<u64> {
    let entry = if result.is_array() {
        result.as_array()?.first()?
    } else {
        result
    };
    // `Location` nests under `range`; `LocationLink` nests under
    // `targetRange` -- try both real shapes rather than assuming one.
    entry
        .get("range")
        .or_else(|| entry.get("targetRange"))?
        .get("start")?
        .get("line")?
        .as_u64()
}

#[test]
fn lsp_definition_jumps_to_a_real_pyright_resolved_function_definition() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(DEFINITION_PY);
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

    // Definition request from within "foo" at the real call site (line 4,
    // character 1 -- inside the 3-character identifier "foo").
    let definition_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "lsp_definition".to_string(),
            params: serde_json::json!({ "doc_id": doc_id, "line": 4, "character": 1 }),
        },
        tx.clone(),
    );
    assert!(
        definition_resp.error.is_none(),
        "lsp_definition errored: {:?}",
        definition_resp.error
    );
    assert_eq!(definition_resp.result.unwrap()["status"], "requested");

    let definition_event =
        recv_event_matching(&rx, "lsp_definition_result", Duration::from_secs(100));
    assert_eq!(definition_event["data"]["doc_id"], doc_id);
    assert_eq!(definition_event["data"]["line"], 4);
    assert_eq!(definition_event["data"]["character"], 1);

    let result = &definition_event["data"]["result"];
    assert!(
        result.get("jsonrpc").is_none() && result.get("id").is_none(),
        "result must be unwrapped from its JSON-RPC envelope, not the raw response: {definition_event}"
    );
    assert_eq!(
        first_start_line(result),
        Some(0),
        "expected pyright to jump back to the real `def foo` line (0): {definition_event}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
