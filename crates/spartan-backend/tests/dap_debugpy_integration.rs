//! Real, live, end-to-end integration test: `open_file` then `dap_launch`
//! over the real `handle_request` dispatch spawns a real `debugpy.adapter`
//! session (via `spartan-dap`, the real fix for the previously-documented
//! bare-`debugpy` gap -- see `dap_integration::resolve_dap_command`'s own
//! doc comment), a real `dap_stopped` event arrives on the real
//! out-channel with a real breakpoint-hit stack frame and local variable,
//! `dap_continue` runs the real debuggee to completion, and a real
//! `dap_exited` event follows. Self-skips honestly if `debugpy` isn't
//! importable, matching every other real-external-tool integration suite
//! in this repo (mirrors `crates/spartan-dap/tests/
//! dap_python_integration.rs`'s own fixture shape, one layer up at the
//! real IPC dispatch boundary instead of calling `DapSession` directly).

use spartan_backend::{handle_request, BackendState, Request};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn debugpy_adapter_available() -> bool {
    std::process::Command::new("python3")
        .args(["-c", "import debugpy.adapter"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn make_fixture(content: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "spartan-backend-dap-e2e-test-{}-{:?}",
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
    let file = dir.join("target.py");
    std::fs::write(&file, content).unwrap();
    (dir, file)
}

// `y = x * 2` is line 2 (1-indexed) -- the same straight-line shape
// `spartan-dap`'s own dap_python_integration.rs uses.
const FIXTURE_SOURCE: &str = "def compute(x):\n    y = x * 2\n    return y + 1\n\nresult = compute(21)\nprint(f\"result={result}\")\n";

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
fn dap_launch_hits_a_real_breakpoint_then_continue_reaches_a_real_exit() {
    if !debugpy_adapter_available() {
        eprintln!("SKIP: python3 -c 'import debugpy.adapter' failed -- debugpy not installed");
        return;
    }

    let (dir, file) = make_fixture(FIXTURE_SOURCE);
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

    let launch_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "dap_launch".to_string(),
            params: serde_json::json!({ "doc_id": doc_id, "break_lines": [2] }),
        },
        tx.clone(),
    );
    assert!(
        launch_resp.error.is_none(),
        "dap_launch errored: {:?}",
        launch_resp.error
    );
    let session_id = launch_resp.result.unwrap()["session_id"].as_u64().unwrap();

    let stopped = recv_event_matching(&rx, "dap_stopped", Duration::from_secs(30));
    assert_eq!(stopped["data"]["doc_id"], doc_id);
    let frame = &stopped["data"]["stopped"]["frame"];
    assert_eq!(
        frame["line"].as_i64(),
        Some(2),
        "expected a stop on line 2: {stopped}"
    );
    let variables = stopped["data"]["stopped"]["variables"].as_array().unwrap();
    assert!(
        variables
            .iter()
            .any(|v| v["name"] == "x" && v["value"] == "21"),
        "expected a real local variable x = 21: {stopped}"
    );

    let continue_resp = handle_request(
        &state,
        Request {
            id: 3,
            method: "dap_continue".to_string(),
            params: serde_json::json!({ "session_id": session_id }),
        },
        tx.clone(),
    );
    assert!(
        continue_resp.error.is_none(),
        "dap_continue errored: {:?}",
        continue_resp.error
    );

    let exited = recv_event_matching(&rx, "dap_exited", Duration::from_secs(10));
    assert_eq!(exited["data"]["doc_id"], doc_id);

    let disconnect_resp = handle_request(
        &state,
        Request {
            id: 4,
            method: "dap_disconnect".to_string(),
            params: serde_json::json!({ "session_id": session_id }),
        },
        tx,
    );
    assert!(disconnect_resp.error.is_none());

    std::fs::remove_dir_all(&dir).ok();
}
