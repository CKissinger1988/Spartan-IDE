//! Real, executed integration test against a real `rust-analyzer`
//! subprocess indexing a real (if tiny) Cargo crate -- no mocked language
//! server. Skips (rather than fails) if `rust-analyzer` isn't on this
//! machine, matching `spikes/lsp-spike/tests/lsp_integration.rs`'s exact
//! convention. Exercises `lsp_session::spawn`/`notify_edit`/`poll_updates`
//! directly -- no winit/GPU involved, matching this crate's established
//! headless-test split (see `viewport_and_language.rs`).
//!
//! This is genuinely new test surface, not a copy of the spike's own test:
//! the spike's four tests never call `did_change_full` or exercise a
//! diagnostics-*clearing* transition (see `lsp_session.rs`'s doc comment on
//! `run_dispatch_loop` for why `wait_real_diagnostics` alone can't detect
//! that).

use spartan_editor_core::lsp::path_to_file_uri;
use spartan_editor_core::lsp_session::{LspSession, LspUpdate};
use spartan_languages::CommandSpec;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn rust_analyzer_available() -> bool {
    std::process::Command::new("rust-analyzer")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

const MAIN_RS_WITH_ERROR: &str = r#"struct Point {
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

const MAIN_RS_CORRECTED: &str = r#"struct Point {
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
    let bad: i32 = 42;
}
"#;

/// Same pattern as `spikes/lsp-spike/tests/lsp_integration.rs`'s
/// `write_fixture_crate`/`work_dir`: a real, minimal Cargo crate generated
/// at runtime under the system temp dir (not `CARGO_TARGET_TMPDIR`, which
/// resolves inside this workspace and makes `cargo metadata` refuse the
/// manifest as an undeclared workspace member -- found by running the
/// spike's own tests for real).
fn write_fixture_crate(dir: &std::path::Path, main_rs: &str) -> (String, PathBuf) {
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"fixture-crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let main_path = src_dir.join("main.rs");
    std::fs::write(&main_path, main_rs).unwrap();
    (path_to_file_uri(dir), main_path)
}

fn work_dir(name: &str) -> PathBuf {
    let base = std::env::temp_dir();
    let dir = base.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Blocks (with its own timeout, separate from any internal LSP timeout)
/// until `poll_updates` yields at least one `Diagnostics` update, or
/// returns `None` if nothing arrives in time.
fn wait_for_next_update(session: &LspSession, timeout: Duration) -> Option<Vec<String>> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(update) = session.poll_updates().into_iter().next() {
            let LspUpdate::Diagnostics(lines) = update;
            return Some(lines);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
fn real_diagnostics_appear_on_open_and_clear_after_a_live_correction() {
    if !rust_analyzer_available() {
        eprintln!("SKIP: rust-analyzer not found on this machine");
        return;
    }
    let dir = work_dir("editor-core-lsp-test");
    let (_root_uri, main_path) = write_fixture_crate(&dir, MAIN_RS_WITH_ERROR);

    let command = CommandSpec {
        program: "rust-analyzer".to_string(),
        args: vec![],
    };
    let session = LspSession::spawn(&command, &dir, &main_path, MAIN_RS_WITH_ERROR)
        .expect("spawn real rust-analyzer session");

    // First update: the real, initial diagnostics pass after indexing.
    // Budget generously -- this is real rust-analyzer indexing a real (if
    // tiny) crate from cold, the same up-to-90s cost the spike's own tests
    // budget for.
    let initial = wait_for_next_update(&session, Duration::from_secs(95))
        .expect("expected an initial diagnostics update within the indexing budget");
    assert!(
        initial.iter().any(|l| l.contains("error")),
        "expected a real type error to be reported for the fixture's deliberate bug, got: {initial:?}"
    );

    // Live-edit: send the corrected text, matching what `main.rs` does when
    // the debounce timer fires after a real edit.
    session.notify_edit(MAIN_RS_CORRECTED.to_string());

    let after_fix = wait_for_next_update(&session, Duration::from_secs(15))
        .expect("expected a diagnostics update after sending the corrected text");
    assert!(
        after_fix.iter().any(|l| l.contains("0 diagnostics")),
        "expected diagnostics to clear to empty after the live correction, got: {after_fix:?}"
    );

    session.shutdown();
}
