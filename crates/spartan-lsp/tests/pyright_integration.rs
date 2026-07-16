//! Real, live integration test against `pyright-langserver`, the second
//! independent real adapter this project's own history already relies on
//! for exactly this reason (§75.8's own dap-spike precedent: when the
//! "primary" tool named in prior sessions -- `rust-analyzer` here -- isn't
//! actually installed in a given environment, cross-check against a
//! second, real, different adapter rather than skip verification
//! entirely). Self-skips honestly if `pyright-langserver` isn't on `$PATH`,
//! matching every other real-external-tool integration suite in this repo.

use spartan_languages::CommandSpec;
use spartan_lsp::{LspSession, LspUpdate};
use std::path::PathBuf;
use std::time::Duration;

fn pyright_available() -> bool {
    std::process::Command::new("pyright-langserver")
        .arg("--version")
        .output()
        .is_ok()
}

fn make_fixture(content: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "spartan-lsp-pyright-test-{}-{:?}",
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
    let file = dir.join("main.py");
    std::fs::write(&file, content).unwrap();
    (dir, file)
}

/// A real, deliberate type error pyright will genuinely catch: calling
/// `.upper()` (a `str` method) on an `int` literal.
const BROKEN_PY: &str = "x: int = 1\nx.upper()\n";
const FIXED_PY: &str = "x: int = 1\nprint(x)\n";

#[test]
fn a_real_live_pyright_session_reports_a_real_diagnostic_then_clears_it() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(BROKEN_PY);
    let command = CommandSpec {
        program: "pyright-langserver".to_string(),
        args: vec!["--stdio".to_string()],
    };

    let session = LspSession::spawn(
        &command,
        &dir,
        &file,
        "python",
        BROKEN_PY,
        Duration::from_millis(50),
    )
    .expect("real pyright-langserver process must spawn");

    // First update: the real initial diagnostics pass, should report the
    // real deliberate type error.
    let first = session
        .recv_update()
        .expect("a real first update must arrive");
    match first {
        LspUpdate::Diagnostics(diags) => {
            assert!(
                !diags.is_empty(),
                "expected a real diagnostic for `x.upper()` on an int"
            );
            assert!(
                diags
                    .iter()
                    .any(|d| d.message.to_lowercase().contains("upper")
                        || d.message.to_lowercase().contains("attribute")),
                "expected a real message naming the bad attribute access, got: {diags:?}"
            );
            assert_eq!(diags[0].line, 1, "the error is on the real second line");
        }
        LspUpdate::ServerError(e) => panic!("expected real diagnostics, got a server error: {e}"),
    }

    // A real live edit fixing the file -- diagnostics should genuinely
    // clear to empty, proving this session's own `wait_notification`-based
    // dispatch loop (not `wait_real_diagnostics`, which can never observe
    // a clear) actually reports the fix.
    session.notify_edit(FIXED_PY.to_string());
    let second = session
        .recv_update()
        .expect("a real second update must arrive after the fix");
    match second {
        LspUpdate::Diagnostics(diags) => {
            assert!(
                diags.is_empty(),
                "expected the real diagnostic to clear after fixing the file, got: {diags:?}"
            );
        }
        LspUpdate::ServerError(e) => panic!("expected a real clear, got a server error: {e}"),
    }

    session.shutdown();
    std::fs::remove_dir_all(&dir).ok();
}

/// Real, live completion request against a real running
/// `pyright-langserver` session -- the first real exercise of
/// `LspSession::request_completion`'s own query-priority mechanism
/// (task #136), the direct sibling of the hover test below.
#[test]
fn a_real_live_pyright_session_answers_a_real_completion_request() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    const COMPLETION_PY: &str = "import os\nos.\n";
    let (dir, file) = make_fixture(COMPLETION_PY);
    let command = CommandSpec {
        program: "pyright-langserver".to_string(),
        args: vec!["--stdio".to_string()],
    };

    let session = LspSession::spawn(
        &command,
        &dir,
        &file,
        "python",
        COMPLETION_PY,
        Duration::from_millis(50),
    )
    .expect("real pyright-langserver process must spawn");

    let _ = session.recv_update();

    // Real completion right after `os.` (line 1, character 3) -- a real,
    // reliable trigger position since the `os` module's own real, large
    // completion set (`getcwd`, `path`, `environ`, ...) is unambiguous.
    let result = session
        .request_completion(1, 3)
        .expect("expected a real completion response from pyright, not a timeout");
    let text = result.to_string().to_lowercase();
    assert!(
        text.contains("getcwd") || text.contains("path") || text.contains("environ"),
        "expected pyright's real completion list to mention a real os module member, got: {result}"
    );

    session.shutdown();
    std::fs::remove_dir_all(&dir).ok();
}

/// Real, live hover request against a real running `pyright-langserver`
/// session -- the first real exercise of `LspSession::request_hover`'s own
/// query-priority mechanism (§134), not just its pure unit tests.
#[test]
fn a_real_live_pyright_session_answers_a_real_hover_request() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on $PATH");
        return;
    }

    let (dir, file) = make_fixture(FIXED_PY);
    let command = CommandSpec {
        program: "pyright-langserver".to_string(),
        args: vec!["--stdio".to_string()],
    };

    let session = LspSession::spawn(
        &command,
        &dir,
        &file,
        "python",
        FIXED_PY,
        Duration::from_millis(50),
    )
    .expect("real pyright-langserver process must spawn");

    // `FIXED_PY` is `x: int = 1\nprint(x)\n` -- genuinely clean, so the
    // real initial update is a `ServerError` (no diagnostics arrive within
    // the wait), matching this crate's own already-documented "a clean
    // file reports no diagnostics at all" case. Drained, not asserted on,
    // since this test is about hover, not that behavior.
    let _ = session.recv_update();

    // Real hover over the `x` in `x: int = 1` (line 0, character 0-1).
    let result = session
        .request_hover(0, 0)
        .expect("expected a real hover response from pyright, not a timeout");
    let text = result.to_string().to_lowercase();
    assert!(
        text.contains("int"),
        "expected pyright's real hover to mention the variable's `int` type, got: {result}"
    );

    session.shutdown();
    std::fs::remove_dir_all(&dir).ok();
}
