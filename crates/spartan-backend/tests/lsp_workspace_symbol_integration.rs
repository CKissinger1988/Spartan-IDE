//! Real, live, end-to-end integration test: `open_file` then
//! `lsp_workspace_symbol` over the real `handle_request` dispatch spawns a
//! real `rust-analyzer` session (the one Tier 1 server a live probe
//! confirmed genuinely answers `workspace/symbol` in this environment --
//! pyright *declares* `true` but returns `[]` for every query, the same
//! class of finding as its code actions) and a real
//! `lsp_workspace_symbol_result` event arrives on the real out-channel
//! carrying rust-analyzer's own real, already-decoded
//! `{name, kind, container_name, uri, line, character}` symbol list. The
//! project root ("workspace") is a real tiny Cargo crate, so
//! `workspace/symbol` is a genuine *workspace-wide* request searching real
//! code, not a per-document one -- unlike every sibling `lsp_*` query,
//! its `doc_id` only supplies the live session, never a cursor position.
//!
//! `cargo metadata`'s well-known ban on undeclared workspace members means
//! the fixture crate must live under the system temp dir, not inside this
//! workspace -- `spartan-editor-core`'s own `write_fixture_crate` records
//! that real finding. Self-skips honestly if `rust-analyzer` isn't on
//! `$PATH`, matching every real-external-tool integration suite in this
//! repo (mirrors `lsp_document_symbol_integration.rs`'s own shape -- the
//! direct sibling query, except that one is per-document against pyright).

use spartan_backend::{handle_request, BackendState, Request};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn rust_analyzer_available() -> bool {
    std::process::Command::new("rust-analyzer")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// A real, minimal Cargo crate generated at runtime under the system temp
/// dir (not `CARGO_TARGET_TMPDIR`, which resolves inside this workspace and
/// makes `cargo metadata` refuse the manifest as an undeclared workspace
/// member -- the real finding `spartan-editor-core`'s own
/// `write_fixture_crate` documents). The fixture is a real Cargo crate so a
/// real empty-query `workspace/symbol` answers a real, non-empty list (the
/// workspace's real module index) rather than `[]`.
fn make_rust_fixture() -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "spartan-backend-lsp-workspace-symbol-e2e-{}-{:?}",
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
        "fn add(a: i32, b: i32) -> i32 { a + b }\n\nfn main() { let _ = add(1, 2); }\n",
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
fn lsp_workspace_symbol_reports_a_real_workspace_wide_symbol_list() {
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

    // A real empty-query search ("list everything") against a real, indexed
    // workspace: rust-analyzer's workspace symbol index genuinely only
    // exists once its initial indexing pass finishes, so this is budgeted
    // generously (the same 90s-class wait every LSP integration test here
    // documents) -- and the answer must be real symbols, not `[]`.
    let symbol_resp = handle_request(
        &state,
        Request {
            id: 2,
            method: "lsp_workspace_symbol".to_string(),
            params: serde_json::json!({ "doc_id": doc_id, "query": "" }),
        },
        tx.clone(),
    );
    assert!(
        symbol_resp.error.is_none(),
        "lsp_workspace_symbol errored: {:?}",
        symbol_resp.error
    );
    assert_eq!(symbol_resp.result.unwrap()["status"], "requested");

    let symbol_event =
        recv_event_matching(&rx, "lsp_workspace_symbol_result", Duration::from_secs(105));
    assert_eq!(symbol_event["data"]["doc_id"], doc_id);
    assert_eq!(symbol_event["data"]["query"], "");

    let result = &symbol_event["data"]["result"];
    assert!(
        result.get("jsonrpc").is_none() && result.get("id").is_none(),
        "result must be the decoded, frontend-ready shape, not a raw JSON-RPC envelope: {symbol_event}"
    );

    let symbols = result
        .as_array()
        .unwrap_or_else(|| panic!("expected a real WorkspaceSymbol[] array, got: {symbol_event}"));
    assert!(
        !symbols.is_empty(),
        "expected real workspace symbols from rust-analyzer for the empty query, got: {symbol_event}"
    );

    // A real live finding that shaped this assertion: rust-analyzer's
    // empty-query `workspace/symbol` answers the *module index* of the
    // loaded workspace once the real initial indexing pass finishes -- the
    // fixture crate's own name plus its real dependencies (`std`, `alloc`,
    // `core`, `proc_macro`, `test`) -- not necessarily every function body,
    // whose per-symbol entries can still be unresolved at this window. The
    // fixture crate's module name being genuinely present (not `[]`, which
    // is exactly the pyright-shaped failure this capability was brought to
    // life to avoid) is the honest, guaranteed answer; both the Cargo.toml
    // package name and its normalized underscore form can appear.
    let names: Vec<&str> = symbols.iter().filter_map(|s| s["name"].as_str()).collect();
    assert!(
        names.contains(&"fixture-crate") || names.contains(&"fixture_crate"),
        "missing the fixture crate's real module name from the workspace symbol list: {names:?}"
    );

    // Every entry is already the flattened, frontend-ready shape the
    // `WorkspaceSymbol` decode produces -- a real string `name`, a real
    // `kind` integer, a real `uri`, and a real `line`/`character` position
    // -- never the raw 3.17 `location`-can-be-bare-`{uri}` wire shape.
    for s in symbols {
        assert!(s["name"].is_string(), "expected a real symbol name: {s}");
        assert!(s["kind"].is_u64(), "expected a real symbol kind: {s}");
        assert!(
            s["uri"].is_string(),
            "expected a real decoded file uri: {s}"
        );
        assert!(
            s["line"].is_u64() && s["character"].is_u64(),
            "expected a real decoded position: {s}"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}
