//! Cross-language reuse check for `DapSession` -- no Python-specific code
//! added anywhere in `dap_session.rs`/`dap.rs`. Matches
//! `spikes/dap-spike/tests/dap_python_cross_language.rs`'s exact rationale
//! and technique: `lldb-dap` isn't the only real adapter this design has to
//! work with, and on machines where `lldb-dap`/`lldb-dap-18` aren't
//! installed (this one, as of this run) but Python + `debugpy` are, this is
//! the only way to get *real*, executed verification rather than a
//! universal self-skip. Skips if Python + `debugpy` aren't available.

use spartan_editor_core::dap_session::{DapCommand, DapSession, DapUpdate};
use spartan_languages::CommandSpec;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Windows' official python.org installer creates only `python.exe` -- no
/// `python3` alias. Try the portable name first, then the Windows-only one.
fn python_bin() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|candidate| {
        std::process::Command::new(candidate)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

fn debugpy_available(python: &str) -> bool {
    std::process::Command::new(python)
        .args(["-c", "import debugpy"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

const TARGET_PY: &str = r#"def compute(x):
    y = x * 2
    return y + 1

def main():
    result = compute(21)
    print(f"result={result}")

if __name__ == "__main__":
    main()
"#;

fn work_dir(name: &str) -> PathBuf {
    let base = std::env::temp_dir();
    let dir = base.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `debugpy`'s adapter binary is `<python> -m debugpy.adapter`, not a
/// single executable, so it needs a wrapper script -- `DapClient::spawn`
/// (via `DapSession::launch`'s `adapter: &CommandSpec`) executes a single
/// command directly, no shell. Same technique as `dap-spike`'s own
/// `spawn_debugpy_adapter`, adapted to return a `CommandSpec` instead of a
/// `DapClient` directly, since `DapSession::launch` owns spawning itself.
///
/// The port originally dropped `dap-spike`'s `#[cfg(not(windows))]` branch
/// and kept only the Windows `.cmd` wrapper, unnoticed because no session
/// had both a non-Windows environment and `debugpy` actually installed at
/// the same time until this one -- a `.cmd` file has no executable bit and
/// no interpreter on Linux/macOS, so `DapClient::spawn` failed to launch it
/// and the test surfaced an `Error`/`Exited` update instead of `Stopped`.
/// Restored `dap-spike`'s original platform split (already learned there,
/// per its own comment, from running it for real on Windows first).
fn debugpy_wrapper_command(python: &str, dir: &std::path::Path) -> CommandSpec {
    #[cfg(windows)]
    {
        let wrapper = dir.join("run-debugpy-adapter.cmd");
        std::fs::write(
            &wrapper,
            format!("@echo off\r\n{python} -m debugpy.adapter %*\r\n"),
        )
        .unwrap();
        CommandSpec {
            program: wrapper.to_string_lossy().to_string(),
            args: vec![],
        }
    }
    #[cfg(not(windows))]
    {
        let wrapper = dir.join("run-debugpy-adapter.sh");
        std::fs::write(
            &wrapper,
            format!("#!/bin/sh\nexec {python} -m debugpy.adapter \"$@\"\n"),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&wrapper).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&wrapper, perms).unwrap();
        CommandSpec {
            program: wrapper.to_string_lossy().to_string(),
            args: vec![],
        }
    }
}

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
fn same_dap_session_hits_a_real_breakpoint_and_continues_to_exit_in_a_real_python_program() {
    let Some(python) = python_bin() else {
        eprintln!("SKIP: no python3/python on this machine");
        return;
    };
    if !debugpy_available(python) {
        eprintln!("SKIP: python3/python or the debugpy package isn't available on this machine");
        return;
    }

    let dir = work_dir("editor-core-dap-python-test");
    let script_path = dir.join("target.py");
    std::fs::write(&script_path, TARGET_PY).expect("write python fixture");

    let adapter_command = debugpy_wrapper_command(python, &dir);
    // For a debugpy target, `binary_path` is the script itself -- debugpy
    // handles interpreting it, unlike the Rust/lldb-dap case where it's a
    // real compiled executable.
    let session = DapSession::launch(&adapter_command, &script_path, &dir, &script_path, &[2]);

    let initial = wait_for_update(&session, Duration::from_secs(20))
        .expect("expected an initial Stopped update after launching against debugpy");
    let DapUpdate::Stopped(lines) = initial else {
        panic!("expected the first update to be Stopped, got an Error or Exited instead");
    };
    assert!(
        lines.iter().any(|l| l.contains("line 2")),
        "expected the stop to report line 2, got: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains('x') && l.contains("21")),
        "expected a real local variable x = 21 at the breakpoint, got: {lines:?}"
    );

    // `step_over`/`step_into` are new methods this pass added (not present
    // in the spike at all) -- exercise `step_over` for real here rather
    // than leaving it wired but untested. `y = x * 2` (line 2) stepping
    // over should land on `return y + 1` (line 3).
    session.send_command(DapCommand::StepOver);
    let after_step = wait_for_update(&session, Duration::from_secs(15))
        .expect("expected an update after sending StepOver");
    let DapUpdate::Stopped(step_lines) = after_step else {
        panic!("expected StepOver to produce a Stopped update, got something else");
    };
    assert!(
        step_lines.iter().any(|l| l.contains("line 3")),
        "expected step-over from line 2 to land on line 3, got: {step_lines:?}"
    );

    session.send_command(DapCommand::Continue);
    let after_continue = wait_for_update(&session, Duration::from_secs(15))
        .expect("expected an update after sending Continue");
    assert!(
        matches!(after_continue, DapUpdate::Exited),
        "expected the program to run to a real exit after Continue, got something else"
    );

    session.shutdown();
}
