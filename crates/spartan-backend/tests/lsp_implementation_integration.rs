//! Real, live, end-to-end integration test: `open_file` then
//! `lsp_implementation` over the real `handle_request` dispatch spawns a
//! real `rust-analyzer` session and a real `lsp_implementation_result`
//! event arrives on the real out-channel carrying rust-analyzer's own real
//! implementation target. Self-skips honestly if `rust-analyzer` isn't on
//! `$PATH`, matching every other real-external-tool integration suite in
//! this repo (mirrors `lsp_workspace_symbol_integration.rs`'s own shape).

use spartan_backend::{handle_request, BackendState, Request};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn rust_analyzer_available() -> bool {
    std::process::Command::new("rust-analyzer")
        .arg("--version")
        .output()
        .is_ok()
}

/// A real Cargo crate so a real `implementation` query has a real impl
/// block to find: a trait declared here and implemented by a struct in the
/// same file -- rust-analyzer's answer for the trait name must be that
/// impl block, never `[]`.
fn make_rust_fixture() -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "spartan-backend-lsp-implementation-e2e-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"fixture-crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(
        &file,
        "trait Speak { fn speak(&self); }\nstruct Dog;\nimpl Speak for Dog { fn speak(&self) {} }\nfn main() {}\n",
    )
    .unwrap();
    (dir, file)
}

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
fn lsp_implementation_reports_a_real_rust_analyzer_impl_target() {
    if !rust_analyzer_available() {
        eprintln!("SKIP: rust-analyzer not found on $PATH");
        return;
    }

    let (dir, file) = make_rust_fixture();
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

    // Real `implementation` query on the trait name `Speak` (line 0,
    // character 6, the start of the identifier). rust-analyzer's initial
    // indexing can lag the moment a real answer is resolvable, so this
    // retries honestly until a non-empty `Location[]` arrives rather than
    // mistaking a pre-index `[]` for a real "no implementations" answer --
    // the exact same `[]`-shaped false negative the type-hierarchy
    // investigation taught this project to treat with suspicion.
    let mut target_uri = None;
    for attempt in 0..6 {
        let impl_resp = handle_request(
            &state,
            Request {
                id: 10 + attempt,
                method: "lsp_implementation".to_string(),
                params: serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 6 }),
            },
            tx.clone(),
        );
        assert!(
            impl_resp.error.is_none(),
            "lsp_implementation errored: {:?}",
            impl_resp.error
        );
        assert_eq!(impl_resp.result.unwrap()["status"], "requested");

        let event =
            recv_event_matching(&rx, "lsp_implementation_result", Duration::from_secs(105));
        assert_eq!(event["data"]["doc_id"], doc_id);
        assert_eq!(event["data"]["line"], 0);
        assert_eq!(event["data"]["character"], 6);

        let result = &event["data"]["result"];
        assert!(
            result.get("jsonrpc").is_none() && result.get("id").is_none(),
            "result must be unwrapped from its JSON-RPC envelope, not the raw response: {event}"
        );

        let arr = result
            .as_array()
            .unwrap_or_else(|| panic!("expected a real Location[] result, got: {event}"));
        if !arr.is_empty() {
            target_uri = arr[0]
                .get("uri")
                .and_then(|u| u.as_str())
                .map(str::to_string);
            break;
        }
        std::thread::sleep(Duration::from_secs(3));
    }

    let uri = target_uri.unwrap_or_else(|| {
        panic!("rust-analyzer never returned a real implementation target for the trait: expected a Location[] entry pointing at the fixture's impl block")
    });
    assert!(
        uri.ends_with("src/main.rs"),
        "expected the impl to resolve inside the fixture file, got: {uri}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
