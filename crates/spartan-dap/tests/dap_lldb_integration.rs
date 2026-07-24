//! Real, executed integration test against a real `lldb-dap` subprocess and
//! a real compiled Rust binary -- no mocked debug adapter, no mocked
//! compiler. Skips (rather than fails) if no `lldb-dap`/`lldb-dap-18`/
//! `rustc` are found, matching this whole repo's own established
//! real-external-tool-integration-test convention (see `spikes/dap-spike`,
//! `spartan-editor-core::tests::dap_integration`, this crate's own sibling
//! `dap_python_integration.rs`).
//!
//! This is a real second exercise of `session::describe_stop`'s own
//! structured `DapStopped` output (not display strings), confirming the
//! `spartan-lsp`-style adaptation for a background-thread consumer holds
//! for DAP too.

use spartan_dap::{DapCommand, DapSession, DapUpdate};
use spartan_languages::CommandSpec;
use std::path::PathBuf;

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

const FIXTURE_SOURCE: &str = r#"fn compute(x: i32) -> i32 {
    let y = x * 2;
    y + 1
}

fn main() {
    let result = compute(21);
    println!("result={}", result);
}
"#;

#[test]
fn real_breakpoint_hits_then_continue_runs_to_a_real_exit() {
    let Some(adapter) = find_adapter() else {
        eprintln!("SKIP: no lldb-dap binary found on this machine");
        return;
    };
    let dir = work_dir("spartan-dap-lldb-test");
    let src_path = dir.join("target.rs");
    let bin_path = dir.join("target_bin");
    compile_fixture(FIXTURE_SOURCE, &src_path, &bin_path);

    let adapter_command = CommandSpec {
        program: adapter.to_string(),
        args: vec![],
    };
    // `let y = x * 2;` is line 2 (1-indexed) of FIXTURE_SOURCE.
    let session = DapSession::launch(
        &adapter_command,
        false, // needs_build: false -- already compiled directly via rustc above
        &dir,
        &bin_path,
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
    assert_eq!(stopped.reason, "breakpoint");
    let frame = stopped.frame.expect("expected a real stack frame");
    assert_eq!(
        frame.line, 2,
        "expected the stop to report line 2: {frame:?}"
    );
    assert!(
        stopped
            .variables
            .iter()
            .any(|v| v.name == "x" && v.value.contains("21")),
        "expected a real local variable x = 21 at the breakpoint, got: {:?}",
        stopped.variables
    );

    session.send_command(DapCommand::Continue);
    // A real, live finding from wiring up DAP `output` events (task #275):
    // unlike `debugpy` (proven separately in
    // `dap_python_integration.rs`'s own logpoint test), `lldb-dap` relays
    // the debuggee's own real stdout (`println!("result={}", result)`)
    // through this exact mechanism too -- so a real `Output` update can
    // now legitimately arrive on this channel before the final
    // Stopped/Exited outcome. Drain any of those first, matching the
    // identical pattern the new logpoint test already established.
    let mut saw_real_stdout = false;
    let final_update = loop {
        match session
            .recv_update()
            .expect("expected a real update after sending Continue")
        {
            DapUpdate::Output { text, .. } => {
                if text.contains("result=43") {
                    saw_real_stdout = true;
                }
            }
            other => break other,
        }
    };
    assert!(
        matches!(final_update, DapUpdate::Exited),
        "expected the program to run to a real exit after Continue, got {final_update:?}"
    );
    assert!(
        saw_real_stdout,
        "expected the debuggee's own real stdout (result=43) to have arrived as a real Output update"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn real_step_over_advances_to_the_real_next_line() {
    let Some(adapter) = find_adapter() else {
        eprintln!("SKIP: no lldb-dap binary found on this machine");
        return;
    };
    let dir = work_dir("spartan-dap-lldb-step-test");
    let src_path = dir.join("target.rs");
    let bin_path = dir.join("target_bin");
    compile_fixture(FIXTURE_SOURCE, &src_path, &bin_path);

    let adapter_command = CommandSpec {
        program: adapter.to_string(),
        args: vec![],
    };
    let session = DapSession::launch(
        &adapter_command,
        false,
        &dir,
        &bin_path,
        &dir,
        &src_path,
        &[spartan_dap::Breakpoint::line(2)],
    );

    let initial = session.recv_update().expect("expected a real initial stop");
    assert!(matches!(initial, DapUpdate::Stopped(_)));

    session.send_command(DapCommand::StepOver);
    let after_step = session
        .recv_update()
        .expect("expected a real update after StepOver");
    let DapUpdate::Stopped(stopped) = after_step else {
        panic!("expected StepOver to land on a real Stopped, got something else");
    };
    let frame = stopped.frame.expect("expected a real stack frame");
    assert_eq!(
        frame.line, 3,
        "expected StepOver to land on real line 3: {frame:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
