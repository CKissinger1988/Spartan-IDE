//! Real, executed integration test against a real Python debugger via
//! `debugpy`. Self-skips honestly if `python3`/`debugpy` aren't available,
//! matching this repo's own established real-external-tool convention.
//!
//! **A real fix for a previously-documented gap, found by testing.** This
//! project's own history (§75.8, carried into `languages.toml`'s Python
//! `dap_command = { program = "debugpy" }`) already named a real,
//! unresolved problem: the bare `debugpy` executable is a CLI for
//! *launching* a debuggee, not a stdio DAP adapter itself, so
//! `DapClient::spawn("debugpy")` was never actually invokable as one.
//! Confirmed here by reading `debugpy`'s own installed source: the real
//! stdio DAP adapter is `debugpy.adapter`, its own dedicated module,
//! reached via `python3 -m debugpy.adapter` with **no** `--port`/`--host`
//! flag (those switch it into a socket-based `debugServer` mode instead --
//! confirmed via its own `--help` output before writing this test). This
//! test is the first real, live proof that invocation actually works.

use spartan_dap::{DapCommand, DapSession, DapUpdate};
use spartan_languages::CommandSpec;
use std::path::PathBuf;

fn debugpy_adapter_available() -> bool {
    std::process::Command::new("python3")
        .args(["-c", "import debugpy.adapter"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn work_dir(name: &str) -> PathBuf {
    let base = std::env::temp_dir();
    let dir = base.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// A straight-line program mirroring `dap_lldb_integration.rs`'s own
// fixture shape, for the same real reason: hits the breakpoint on line 2,
// then runs to completion with nothing left to stop at.
const FIXTURE_SOURCE: &str = "def compute(x):\n    y = x * 2\n    return y + 1\n\nresult = compute(21)\nprint(f\"result={result}\")\n";

#[test]
fn real_debugpy_breakpoint_hits_then_continue_runs_to_a_real_exit() {
    if !debugpy_adapter_available() {
        eprintln!("SKIP: python3 -c 'import debugpy.adapter' failed -- debugpy not installed");
        return;
    }
    let dir = work_dir("spartan-dap-debugpy-test");
    let src_path = dir.join("target.py");
    std::fs::write(&src_path, FIXTURE_SOURCE).unwrap();

    let adapter_command = CommandSpec {
        program: "python3".to_string(),
        args: vec!["-m".to_string(), "debugpy.adapter".to_string()],
    };
    // `y = x * 2` is line 2 (1-indexed) of FIXTURE_SOURCE.
    let session = DapSession::launch(
        &adapter_command,
        false,
        &dir,
        &src_path,
        &dir,
        &src_path,
        &[2],
    );

    let initial = session
        .recv_update()
        .expect("expected a real initial update after launching");
    let DapUpdate::Stopped(stopped) = initial else {
        panic!("expected the first update to be Stopped");
    };
    let frame = stopped.frame.expect("expected a real stack frame");
    assert_eq!(
        frame.line, 2,
        "expected the stop to report line 2: {frame:?}"
    );
    assert!(
        stopped
            .variables
            .iter()
            .any(|v| v.name == "x" && v.value == "21"),
        "expected a real local variable x = 21 at the breakpoint, got: {:?}",
        stopped.variables
    );

    session.send_command(DapCommand::Continue);
    let after_continue = session
        .recv_update()
        .expect("expected a real update after sending Continue");
    assert!(
        matches!(after_continue, DapUpdate::Exited),
        "expected the program to run to a real exit after Continue"
    );

    std::fs::remove_dir_all(&dir).ok();
}
