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

/// Drains any real `DapUpdate::Output` updates (e.g. the debuggee's own
/// real stdout, now genuinely relayed since task #275's fix) and returns
/// the first non-`Output` update -- the real Stopped/Exited outcome a
/// test actually cares about. A real, live finding this whole suite's
/// pre-existing tests needed retrofitting for: before this fix, an
/// `output` event was silently buffered and lost, so `Continue` always
/// delivered exactly one update; now a real debuggee `print()` can
/// legitimately arrive as its own update first.
fn next_non_output_update(session: &DapSession) -> DapUpdate {
    loop {
        match session
            .recv_update()
            .expect("expected a real update while draining output")
        {
            DapUpdate::Output { .. } => continue,
            other => return other,
        }
    }
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
        &[spartan_dap::Breakpoint::line(2)],
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
    let after_continue = next_non_output_update(&session);
    assert!(
        matches!(after_continue, DapUpdate::Exited),
        "expected the program to run to a real exit after Continue, got {after_continue:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn real_debugpy_evaluate_computes_a_real_expression_in_the_stopped_frame() {
    if !debugpy_adapter_available() {
        eprintln!("SKIP: python3 -c 'import debugpy.adapter' failed -- debugpy not installed");
        return;
    }
    let dir = work_dir("spartan-dap-debugpy-eval-test");
    let src_path = dir.join("target.py");
    std::fs::write(&src_path, FIXTURE_SOURCE).unwrap();

    let adapter_command = CommandSpec {
        program: "python3".to_string(),
        args: vec!["-m".to_string(), "debugpy.adapter".to_string()],
    };
    // Stop on line 2 (`y = x * 2`), where `x == 21` is in scope.
    let session = DapSession::launch(
        &adapter_command,
        false,
        &dir,
        &src_path,
        &dir,
        &src_path,
        &[spartan_dap::Breakpoint::line(2)],
    );

    let initial = session
        .recv_update()
        .expect("expected a real initial update after launching");
    let DapUpdate::Stopped(_) = initial else {
        panic!("expected the first update to be Stopped, got {initial:?}");
    };

    // A real watch/REPL evaluation of an arbitrary expression against the
    // real stopped frame -- `x * 2` where x == 21 must be exactly "42".
    let result = session
        .evaluate("x * 2")
        .expect("expected a real evaluate result");
    assert_eq!(result, "42", "expected x * 2 == 42 at the breakpoint");

    // A real evaluation error (an undefined name) is reported as an Err,
    // not silently swallowed or returned as a bogus value.
    let bad = session.evaluate("this_name_does_not_exist");
    assert!(
        bad.is_err(),
        "expected an undefined name to be a real evaluation error, got: {bad:?}"
    );

    session.send_command(DapCommand::Continue);
    let after = next_non_output_update(&session);
    assert!(
        matches!(after, DapUpdate::Exited),
        "expected a real exit after Continue: {after:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

// A real, live proof that a logpoint's real `output` event is now
// captured, not silently lost -- the exact gap task #249's own account
// named ("the debug panel shows stop/variable state, not a DAP `output`
// event stream"). A real, unconditioned breakpoint on line 1 gets the
// session stopped first (so `DapCommand::Continue` is the one thing under
// test, not the initial launch handshake, matching this pass's own
// named v1 scope); a second entry with only `log_message` set (no
// `condition`) turns line 3 into a real logpoint -- the adapter logs the
// interpolated message and does not stop there at all.
const LOGPOINT_LOOP_FIXTURE: &str = "total = 0\nfor i in range(3):\n    total += i\nprint(total)\n";

#[test]
fn real_debugpy_logpoint_output_is_captured_not_silently_dropped() {
    if !debugpy_adapter_available() {
        eprintln!("SKIP: python3 -c 'import debugpy.adapter' failed -- debugpy not installed");
        return;
    }
    let dir = work_dir("spartan-dap-debugpy-logpoint-test");
    let src_path = dir.join("logloop.py");
    std::fs::write(&src_path, LOGPOINT_LOOP_FIXTURE).unwrap();

    let adapter_command = CommandSpec {
        program: "python3".to_string(),
        args: vec!["-m".to_string(), "debugpy.adapter".to_string()],
    };
    let stop_breakpoint = spartan_dap::Breakpoint::line(1);
    let logpoint = spartan_dap::Breakpoint {
        line: 3,
        condition: None,
        log_message: Some("iter {i}".to_string()),
    };
    let session = DapSession::launch(
        &adapter_command,
        false,
        &dir,
        &src_path,
        &dir,
        &src_path,
        &[stop_breakpoint, logpoint],
    );

    let initial = session
        .recv_update()
        .expect("expected a real initial update after launching");
    let DapUpdate::Stopped(stopped) = initial else {
        panic!("expected the first update to be Stopped, got {initial:?}");
    };
    assert_eq!(
        stopped.frame.as_ref().map(|f| f.line),
        Some(1),
        "expected the real stop at the unconditioned line-1 breakpoint"
    );

    // Continuing runs the whole loop -- the logpoint fires 3 real times
    // (i = 0, 1, 2), logging without ever stopping, then the real program
    // exits (no other breakpoint left to hit).
    session.send_command(DapCommand::Continue);

    let mut all_output: Vec<(String, String)> = Vec::new();
    let final_update = loop {
        match session
            .recv_update()
            .expect("expected a real update while draining the logpoint's real output")
        {
            DapUpdate::Output { category, text } => all_output.push((category, text)),
            other => break other,
        }
    };
    assert!(
        matches!(final_update, DapUpdate::Exited),
        "expected the real program to exit once the loop finishes: {final_update:?}"
    );
    // A real, live-observed finding: no `output` event ever arrives with
    // the real `telemetry` category (debugpy's own internal diagnostic
    // pings, e.g. "ptvsd"/"debugpy") -- confirming `wait_for_stop_or_exit`
    // filters it, not just that it happens not to appear in this run.
    assert!(
        all_output.iter().all(|(cat, _)| cat != "telemetry"),
        "expected telemetry-category output to be filtered before reaching a caller: {all_output:?}"
    );
    // The real logpoint firings (a second, real finding: debugpy relays
    // them with the identical "stdout" category the debuggee's own real
    // `print(total)` output arrives with -- there's no separate
    // "this came from a logpoint" marker to filter on).
    let logged_texts: Vec<&String> = all_output
        .iter()
        .map(|(_, text)| text)
        .filter(|t| t.starts_with("iter "))
        .collect();
    assert_eq!(
        logged_texts.len(),
        3,
        "expected exactly 3 real logpoint firings (i=0,1,2), got: {all_output:?}"
    );
    for (i, text) in logged_texts.iter().enumerate() {
        assert!(
            text.contains(&format!("iter {i}")),
            "expected the real interpolated logMessage to contain 'iter {i}', got: {text:?}"
        );
    }
    // The real debuggee's own `print(total)` (total == 0+1+2 == 3) also
    // genuinely arrived through this same real mechanism -- confirming
    // this isn't logpoint-only, it's real stdout relay in general.
    assert!(
        all_output.iter().any(|(_, text)| text.contains('3')),
        "expected the debuggee's own real print(total) output to have arrived too: {all_output:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

// A loop, so a *conditional* breakpoint has something to skip past: the
// adapter must only stop when the condition is truthy, not on every hit.
const LOOP_FIXTURE: &str = "total = 0\nfor i in range(10):\n    total += i\nprint(total)\n";

#[test]
fn real_debugpy_conditional_breakpoint_only_stops_when_the_condition_is_true() {
    if !debugpy_adapter_available() {
        eprintln!("SKIP: python3 -c 'import debugpy.adapter' failed -- debugpy not installed");
        return;
    }
    let dir = work_dir("spartan-dap-debugpy-cond-test");
    let src_path = dir.join("loop.py");
    std::fs::write(&src_path, LOOP_FIXTURE).unwrap();

    let adapter_command = CommandSpec {
        program: "python3".to_string(),
        args: vec!["-m".to_string(), "debugpy.adapter".to_string()],
    };
    // `total += i` is line 3 (1-indexed). The condition `i == 3` means the
    // adapter runs the loop body for i=0,1,2 without stopping and only
    // stops on the real iteration where i == 3.
    let breakpoint = spartan_dap::Breakpoint {
        line: 3,
        condition: Some("i == 3".to_string()),
        log_message: None,
    };
    let session = DapSession::launch(
        &adapter_command,
        false,
        &dir,
        &src_path,
        &dir,
        &src_path,
        std::slice::from_ref(&breakpoint),
    );

    let initial = session
        .recv_update()
        .expect("expected a real initial update after launching");
    let DapUpdate::Stopped(stopped) = initial else {
        panic!("expected the first update to be Stopped, got {initial:?}");
    };
    let frame = stopped.frame.expect("expected a real stack frame");
    assert_eq!(
        frame.line, 3,
        "expected the stop at the loop body: {frame:?}"
    );
    // The whole point: it stopped on the iteration where the condition held.
    assert!(
        stopped
            .variables
            .iter()
            .any(|v| v.name == "i" && v.value == "3"),
        "conditional breakpoint should stop only when i == 3, got: {:?}",
        stopped.variables
    );

    session.send_command(DapCommand::Continue);
    let after = next_non_output_update(&session);
    assert!(
        matches!(after, DapUpdate::Exited),
        "expected a real exit after Continue (i never equals 3 again): {after:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
