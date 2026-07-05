//! Real, executed integration tests against a real `rust-analyzer` subprocess
//! indexing a real (if tiny) Cargo crate — no mocked language server. Skips
//! (rather than fails) if `rust-analyzer` isn't on this machine. See
//! docs/architecture-spec.md §47.6 for what was actually run and observed.

use lsp_spike::*;
use std::path::PathBuf;
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

const MAIN_RS: &str = r#"struct Point {
    x: i32,
    y: i32,
}

impl Point {
    fn magnitude_squared(&self) -> i32 {
        self.x * self.x + self.y * self.y
    }
}

fn main() {
    let p = Point { x: 3, y: 4 };
    let m = p.magnitude_squared();
    println!("{}", m);
    let bad: i32 = "not a number";
}
"#;

/// Writes a real, minimal Cargo crate into a fresh directory under the
/// per-test-binary tmp dir (not a static fixture checked into the repo —
/// nesting a second Cargo.toml under a workspace member's tree causes cargo
/// to treat it as an orphaned/ambiguous package, so it's generated at
/// runtime instead, same as dap-spike does for its compiled binaries).
fn write_fixture_crate(dir: &std::path::Path) -> (String, String) {
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"fixture-crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let main_path = src_dir.join("main.rs");
    std::fs::write(&main_path, MAIN_RS).unwrap();
    (
        format!("file://{}", dir.display()),
        format!("file://{}", main_path.display()),
    )
}

fn work_dir(name: &str) -> PathBuf {
    let base = std::env::var("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn diagnostics_report_a_real_type_error_at_the_right_line() {
    if !rust_analyzer_available() {
        eprintln!("SKIP: rust-analyzer not found on this machine");
        return;
    }
    let dir = work_dir("lsp-diag-test");
    let (root_uri, file_uri) = write_fixture_crate(&dir);

    let mut client = LspClient::spawn("rust-analyzer").expect("spawn rust-analyzer");
    client
        .open_project(&root_uri, &file_uri, MAIN_RS)
        .expect("initialize/didOpen sequence");

    let diagnostics = client
        .wait_real_diagnostics(&file_uri, INDEXING_TIMEOUT)
        .expect("rust-analyzer should eventually publish real diagnostics");

    let type_error = diagnostics.iter().find(|d| {
        d.get("code")
            .and_then(|c| c.as_str().or_else(|| c.get("value")?.as_str()))
            == Some("E0308")
    });
    let type_error = type_error.unwrap_or_else(|| {
        panic!("expected an E0308 mismatched-types diagnostic, got: {diagnostics:?}")
    });

    // `let bad: i32 = "not a number";` is line 15 (0-indexed) of MAIN_RS.
    assert_eq!(type_error["range"]["start"]["line"].as_i64(), Some(15));

    client.shutdown();
}

#[test]
fn completion_returns_a_real_method_with_its_real_signature() {
    if !rust_analyzer_available() {
        eprintln!("SKIP: rust-analyzer not found on this machine");
        return;
    }
    let dir = work_dir("lsp-completion-test");
    let (root_uri, file_uri) = write_fixture_crate(&dir);

    let mut client = LspClient::spawn("rust-analyzer").expect("spawn rust-analyzer");
    client
        .open_project(&root_uri, &file_uri, MAIN_RS)
        .expect("initialize/didOpen sequence");
    // Force indexing to complete before asking for completions, same as a
    // real editor would wait for the first diagnostics pass.
    client
        .wait_real_diagnostics(&file_uri, INDEXING_TIMEOUT)
        .expect("indexing should complete");

    let (line, character) = MAIN_RS
        .lines()
        .enumerate()
        .find_map(|(i, l)| {
            l.find("p.magnitude_squared")
                .map(|c| (i as i64, (c + 2) as i64))
        })
        .expect("fixture must contain the call site");

    let resp = client
        .completion(&file_uri, line, character)
        .expect("completion response");
    let items = resp["result"]["items"]
        .as_array()
        .or_else(|| resp["result"].as_array())
        .expect("completion result should be a list");

    let hit = items
        .iter()
        .find(|it| it["label"].as_str() == Some("magnitude_squared"))
        .unwrap_or_else(|| panic!("expected a magnitude_squared completion item, got: {items:?}"));
    assert_eq!(hit["detail"].as_str(), Some("fn(&self) -> i32"));

    client.shutdown();
}

#[test]
fn hover_returns_real_type_and_signature_info() {
    if !rust_analyzer_available() {
        eprintln!("SKIP: rust-analyzer not found on this machine");
        return;
    }
    let dir = work_dir("lsp-hover-test");
    let (root_uri, file_uri) = write_fixture_crate(&dir);

    let mut client = LspClient::spawn("rust-analyzer").expect("spawn rust-analyzer");
    client
        .open_project(&root_uri, &file_uri, MAIN_RS)
        .expect("initialize/didOpen sequence");
    client
        .wait_real_diagnostics(&file_uri, INDEXING_TIMEOUT)
        .expect("indexing should complete");

    let (line, character) = MAIN_RS
        .lines()
        .enumerate()
        .find_map(|(i, l)| {
            l.find("magnitude_squared()")
                .map(|c| (i as i64, (c + 4) as i64))
        })
        .expect("fixture must contain the call site");

    // `wait_real_diagnostics` proves rust-analyzer's diagnostics pass has
    // converged, but hover for a specific call site resolves through a
    // separate on-demand type-inference query that can still be mid-flight
    // for a beat afterward — observed as an intermittent empty-contents
    // response under load (~1-in-5), not a real protocol error. Poll the
    // same way `wait_real_diagnostics` itself polls for readiness, rather
    // than papering over it with a single retry-less assert.
    let mut contents = None;
    for _ in 0..20 {
        let resp = client
            .hover(&file_uri, line, character)
            .expect("hover response");
        if let Some(v) = resp["result"]["contents"]["value"].as_str() {
            contents = Some(v.to_string());
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let contents =
        contents.expect("hover should return markdown contents within 5s of indexing completing");
    assert!(
        contents.contains("Point"),
        "hover should mention the receiver type: {contents}"
    );
    assert!(
        contents.contains("magnitude_squared"),
        "hover should mention the method: {contents}"
    );

    client.shutdown();
}

#[test]
fn client_survives_shutdown_cleanly() {
    if !rust_analyzer_available() {
        eprintln!("SKIP: rust-analyzer not found on this machine");
        return;
    }
    let mut client = LspClient::spawn("rust-analyzer").expect("spawn rust-analyzer");
    let resp = client.request(
        "initialize",
        Some(serde_json::json!({
            "processId": std::process::id(),
            "rootUri": null,
            "capabilities": {},
        })),
        DEFAULT_TIMEOUT,
    );
    assert!(resp.is_some());
    client.shutdown(); // must return promptly, not hang
}

#[test]
fn debounced_did_change_coalesces_a_burst_of_edits_into_one_dispatch() {
    // Pure client-side scheduling policy, no server needed — the other half
    // of §2.3's "own request queue with debounced didChange" claim.
    let mut debouncer = DidChangeDebouncer::new(Duration::from_millis(60));

    // Simulate rapid typing: several edits in quick succession, each well
    // inside the debounce window.
    for _ in 0..5 {
        debouncer.on_edit();
        std::thread::sleep(Duration::from_millis(10));
        assert!(
            !debouncer.should_dispatch_now(),
            "must not dispatch while edits are still arriving inside the debounce window"
        );
    }
    assert_eq!(debouncer.dispatched_count(), 0);

    // Now go idle past the debounce window.
    std::thread::sleep(Duration::from_millis(90));
    assert!(
        debouncer.should_dispatch_now(),
        "must dispatch exactly once after the idle window elapses"
    );
    assert_eq!(debouncer.dispatched_count(), 1);

    // No new edit arrived, so polling again must not re-dispatch.
    assert!(!debouncer.should_dispatch_now());
    assert_eq!(debouncer.dispatched_count(), 1);

    // A fresh edit after the idle period starts a new debounce cycle.
    debouncer.on_edit();
    assert!(!debouncer.should_dispatch_now());
    std::thread::sleep(Duration::from_millis(90));
    assert!(debouncer.should_dispatch_now());
    assert_eq!(debouncer.dispatched_count(), 2);
}
