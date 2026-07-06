//! Real, executed integration test against a real `lldb-dap` subprocess and
//! a real compiled Rust binary -- no mocked debug adapter, no mocked
//! compiler. Skips (rather than fails) if `lldb-dap`/`lldb-dap-18`/`rustc`
//! aren't on this machine, matching `spikes/dap-spike/tests/dap_integration.rs`'s
//! exact convention. Exercises `dap_session::launch`/`send_command`/
//! `poll_updates` directly -- no winit/GPU involved, matching this crate's
//! established headless-test split.
//!
//! This is genuinely new test surface, not a copy of the spike's own tests:
//! the spike never calls `continue_`/`step_over`/`step_into` (its two tests
//! only ever hit a breakpoint and inspect, never resume) -- this test is
//! their first real exercise, via this crate's own `DapSession` orchestration.

use spartan_editor_core::dap_session::{DapCommand, DapSession, DapUpdate};
use spartan_languages::CommandSpec;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const ADAPTER_CANDIDATES: &[&str] = &["lldb-dap-18", "lldb-dap"];

fn find_adapter() -> Option<&'static str> {
    ADAPTER_CANDIDATES
        .iter()
        .find(|bin| {
            std::process::Command::new(bin)
                .arg("--help")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
        .copied()
}

fn work_dir(name: &str) -> PathBuf {
    let base = std::env::temp_dir();
    let dir = base.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Same minimal pattern as `spikes/dap-spike`'s own `compile_fixture` --
/// copied rather than imported, since it's test-only harness code that was
/// deliberately not promoted into `src/dap.rs` (see that module's doc
/// comment).
fn compile_fixture(source: &str, src_path: &std::path::Path, bin_path: &std::path::Path) {
    std::fs::write(src_path, source).expect("write fixture source");
    let output = std::process::Command::new("rustc")
        .arg("-g")
        .arg("-o")
        .arg(bin_path)
        .arg(src_path)
        .output()
        .expect("run rustc");
    assert!(
        output.status.success(),
        "rustc failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// A straight-line program: hits the breakpoint on line 2, then runs to
// completion with nothing left to stop at -- exactly what's needed to
// exercise both `Stopped` (initial hit) and `Exited` (after `Continue`).
const FIXTURE_SOURCE: &str = r#"fn compute(x: i32) -> i32 {
    let y = x * 2;
    y + 1
}

fn main() {
    let result = compute(21);
    println!("result={}", result);
}
"#;

fn wait_for_update(session: &DapSession, timeout: Duration) -> Option<DapUpdate> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(update) = session.poll_updates().into_iter().next() {
            return Some(update);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
fn real_breakpoint_hits_then_continue_runs_to_a_real_exit() {
    let Some(adapter) = find_adapter() else {
        eprintln!("SKIP: no lldb-dap binary found on this machine");
        return;
    };
    let dir = work_dir("editor-core-dap-test");
    let src_path = dir.join("target.rs");
    let bin_path = dir.join("target_bin");
    compile_fixture(FIXTURE_SOURCE, &src_path, &bin_path);

    let adapter_command = CommandSpec {
        program: adapter.to_string(),
        args: vec![],
    };
    // `let y = x * 2;` is line 2 (1-indexed) of FIXTURE_SOURCE.
    let session = DapSession::launch(&adapter_command, &bin_path, &dir, &src_path, &[2]);

    let initial = wait_for_update(&session, Duration::from_secs(15))
        .expect("expected an initial Stopped update after launching");
    let DapUpdate::Stopped(lines) = initial else {
        panic!("expected the first update to be Stopped");
    };
    assert!(
        lines.iter().any(|l| l.contains("line 2")),
        "expected the stop to report line 2, got: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains('x') && l.contains("21")),
        "expected a real local variable x = 21 at the breakpoint, got: {lines:?}"
    );

    session.send_command(DapCommand::Continue);
    let after_continue = wait_for_update(&session, Duration::from_secs(15))
        .expect("expected an update after sending Continue");
    assert!(
        matches!(after_continue, DapUpdate::Exited),
        "expected the program to run to a real exit after Continue"
    );

    session.shutdown();
}
