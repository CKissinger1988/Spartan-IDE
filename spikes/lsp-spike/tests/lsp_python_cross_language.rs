//! Cross-language reuse check for the *same* `LspClient` from
//! `lsp_integration.rs` — no Python-specific code added to the client itself
//! beyond `spawn_with_args` (needed because `pyright-langserver` requires a
//! `--stdio` flag; `rust-analyzer` needs none). Directly tests the open risk
//! named in docs/architecture-spec.md §47.6: "any language other than Rust...
//! the registry-replication risk §39.2's exit artifact was meant to inform is
//! still open." Probed manually against `pyright-langserver` before writing
//! this file. Skips if `pyright-langserver` isn't installed.

use lsp_spike::*;
use std::path::PathBuf;

/// npm installs its global CLI shims as `<name>.cmd` on Windows (plus a
/// same-named extensionless POSIX shell script, unusable there) rather than
/// a native `.exe`. `std::process::Command` does not emulate a shell's
/// `PATHEXT` search, so spawning the bare name fails with `NotFound` even
/// when the shim is really on `PATH` — found by running this for real on
/// Windows, where `pyright_available()` correctly found the extensionless
/// file but the subsequent spawn still failed.
#[cfg(windows)]
const PYRIGHT_BIN: &str = "pyright-langserver.cmd";
#[cfg(not(windows))]
const PYRIGHT_BIN: &str = "pyright-langserver";

/// `pyright-langserver` has no side-effect-free flag to probe with — every
/// argument except `--stdio`/`--node-ipc`/`--socket=N` makes it print a
/// connection error and exit 1 (found by trying `--version`/`--help` first;
/// neither is supported). So availability is checked the same way a shell's
/// `command -v` would, by searching `$PATH` for the executable directly,
/// rather than by invoking it at all.
fn pyright_available() -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| {
        let candidate = dir.join(PYRIGHT_BIN);
        candidate.is_file()
    })
}

const MAIN_PY: &str = r#"class Point:
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def magnitude_squared(self) -> int:
        return self.x * self.x + self.y * self.y


p = Point(3, 4)
m = p.magnitude_squared()
bad: int = "not a number"
"#;

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
fn same_lsp_client_gets_real_diagnostics_completion_and_hover_from_pyright() {
    if !pyright_available() {
        eprintln!("SKIP: pyright-langserver not found on this machine");
        return;
    }
    let dir = work_dir("lsp-python-cross-lang");
    let file_path = dir.join("main.py");
    std::fs::write(&file_path, MAIN_PY).unwrap();
    let root_uri = path_to_file_uri(&dir);
    let file_uri = path_to_file_uri(&file_path);

    // The exact same client used against rust-analyzer in lsp_integration.rs,
    // spawned with the one argv flag pyright-langserver needs.
    let mut client =
        LspClient::spawn_with_args(PYRIGHT_BIN, &["--stdio"]).expect("spawn pyright-langserver");
    client
        .open_project(&root_uri, &file_uri, MAIN_PY)
        .expect("initialize/didOpen sequence");

    let diagnostics = client
        .wait_real_diagnostics(&file_uri, INDEXING_TIMEOUT)
        .expect("pyright should publish real diagnostics");
    let type_error = diagnostics
        .iter()
        .find(|d| d.get("code").and_then(|c| c.as_str()) == Some("reportAssignmentType"))
        .unwrap_or_else(|| {
            panic!("expected a reportAssignmentType diagnostic, got: {diagnostics:?}")
        });
    // `bad: int = "not a number"` is line 11 (0-indexed) of MAIN_PY.
    assert_eq!(type_error["range"]["start"]["line"].as_i64(), Some(11));

    let (line, character) = MAIN_PY
        .lines()
        .enumerate()
        .find_map(|(i, l)| {
            l.find("p.magnitude_squared")
                .map(|c| (i as i64, (c + 2) as i64))
        })
        .expect("fixture must contain the call site");

    let completion_resp = client
        .completion(&file_uri, line, character)
        .expect("completion response");
    let items = completion_resp["result"]["items"]
        .as_array()
        .expect("completion result should have an items list");
    assert!(
        items
            .iter()
            .any(|it| it["label"].as_str() == Some("magnitude_squared")),
        "expected a magnitude_squared completion item among: {items:?}"
    );

    let hover_resp = client
        .hover(&file_uri, line, character + 2)
        .expect("hover response");
    let hover_text = hover_resp["result"]["contents"]["value"]
        .as_str()
        .expect("hover should return contents");
    assert!(
        hover_text.contains("magnitude_squared"),
        "hover should mention the method: {hover_text}"
    );

    client.shutdown();
}
