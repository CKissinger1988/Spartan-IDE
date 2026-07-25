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

    // Real watch/REPL evaluation through the full dispatch (§250): `x * 2`
    // in the stopped frame (x == 21) must be exactly "42".
    let eval_resp = handle_request(
        &state,
        Request {
            id: 5,
            method: "dap_evaluate".to_string(),
            params: serde_json::json!({ "session_id": session_id, "expression": "x * 2" }),
        },
        tx.clone(),
    );
    assert!(
        eval_resp.error.is_none(),
        "dap_evaluate errored: {:?}",
        eval_resp.error
    );
    assert_eq!(
        eval_resp.result.unwrap()["result"].as_str(),
        Some("42"),
        "expected x * 2 == 42 through the real dispatch"
    );

    // Real live edit of a variable's value while stopped (task #276): set
    // `x` to `100` -- if this genuinely reaches the live debuggee frame,
    // `y = x * 2` (evaluated next) must reflect the real new value, not
    // the original `21`.
    let set_var_resp = handle_request(
        &state,
        Request {
            id: 6,
            method: "dap_set_variable".to_string(),
            params: serde_json::json!({ "session_id": session_id, "name": "x", "value": "100" }),
        },
        tx.clone(),
    );
    assert!(
        set_var_resp.error.is_none(),
        "dap_set_variable errored: {:?}",
        set_var_resp.error
    );
    assert_eq!(
        set_var_resp.result.unwrap()["value"].as_str(),
        Some("100"),
        "expected the real adapter-confirmed new value"
    );

    // The fresh Stopped update `set_variable` queues internally must
    // arrive as a real `dap_stopped` event with the real new value.
    let refreshed = recv_event_matching(&rx, "dap_stopped", Duration::from_secs(10));
    let refreshed_vars = refreshed["data"]["stopped"]["variables"]
        .as_array()
        .unwrap();
    assert!(
        refreshed_vars
            .iter()
            .any(|v| v["name"] == "x" && v["value"] == "100"),
        "expected a real refreshed local variable x = 100: {refreshed}"
    );

    // And a real evaluate against the live frame must now see the edited
    // value, not the original one, proving the edit genuinely reached the
    // debuggee's own execution state, not just the display.
    let eval_after_set = handle_request(
        &state,
        Request {
            id: 7,
            method: "dap_evaluate".to_string(),
            params: serde_json::json!({ "session_id": session_id, "expression": "x * 2" }),
        },
        tx.clone(),
    );
    assert_eq!(
        eval_after_set.result.unwrap()["result"].as_str(),
        Some("200"),
        "expected x * 2 == 200 after the real live edit"
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
