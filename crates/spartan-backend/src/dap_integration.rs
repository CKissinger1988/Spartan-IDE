//! Real DAP wiring for `spartan-backend` -- the first time either
//! Electron-based shell (`desktop/`, `web/`) has had real debugging
//! support at all (it has only ever existed in the reference wgpu shell,
//! `spartan-editor-core`). Built entirely on `spartan-dap` (a deliberate
//! second promotion of that shell's own already-tested `dap.rs`/
//! `dap_session.rs`/`build.rs` -- see that crate's own doc comments for
//! the full rationale), mirroring `lsp_integration.rs`'s own shape one
//! module over.
//!
//! **A real, deliberate choice not to fix the shared registry.** Python's
//! configured `dap_command` (`languages.toml`: `program = "debugpy"`, no
//! args) is a real, previously-documented-but-unresolved gap
//! (§75.8/§75.44/§75.45) -- the bare `debugpy` CLI launches a debuggee,
//! it isn't itself a stdio DAP adapter; the real one is
//! `python3 -m debugpy.adapter` (confirmed live by `spartan-dap`'s own
//! `dap_python_integration.rs`). That fix is applied *here*,
//! `resolve_dap_command`, not in `languages.toml` itself, because the
//! reference wgpu shell's own `DapClient::spawn`/`dap_session.rs` has no
//! argv support at all (only `spartan-dap::DapClient::spawn_with_args`,
//! built for this crate, does) -- changing the shared registry would
//! silently swap that shell's current fast-failing "adapter not found"
//! error for a real hang (a bare `python3` with no args and no piped
//! input reads stdin as an interactive REPL). Matches this whole
//! effort's "second promotion, not an extraction" discipline: adapt the
//! new consumer, never risk the already-tested reference.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use spartan_dap::{DapSession, DapUpdate};
use spartan_languages::{CommandSpec, LanguageRegistry};

use crate::Event;

fn resolve_dap_command(profile_id: &str, configured: &CommandSpec) -> CommandSpec {
    if profile_id == "python" && configured.program == "debugpy" && configured.args.is_empty() {
        CommandSpec {
            program: "python3".to_string(),
            args: vec!["-m".to_string(), "debugpy.adapter".to_string()],
        }
    } else {
        configured.clone()
    }
}

fn dap_update_event(doc_id: u64, update: DapUpdate) -> Event {
    match update {
        DapUpdate::BuildFailed(diagnostics) => Event {
            event: "dap_build_failed".to_string(),
            data: serde_json::json!({ "doc_id": doc_id, "diagnostics": diagnostics }),
        },
        DapUpdate::Stopped(stopped) => Event {
            event: "dap_stopped".to_string(),
            data: serde_json::json!({ "doc_id": doc_id, "stopped": stopped }),
        },
        DapUpdate::Exited => Event {
            event: "dap_exited".to_string(),
            data: serde_json::json!({ "doc_id": doc_id }),
        },
        DapUpdate::Error(message) => Event {
            event: "dap_error".to_string(),
            data: serde_json::json!({ "doc_id": doc_id, "message": message }),
        },
        DapUpdate::Output { category, text } => Event {
            event: "dap_output".to_string(),
            data: serde_json::json!({ "doc_id": doc_id, "category": category, "text": text }),
        },
    }
}

/// Real, best-effort DAP launch for a given open file. Unlike
/// `lsp_integration::maybe_spawn_lsp` (which returns `None` for the many
/// real, honest "LSP just isn't available here" cases since every file
/// open silently tries it), this returns a specific `Err(String)`
/// instead -- a user pressing a real debug affordance expects a real,
/// specific reason when nothing happens, not silence.
///
/// Real, named v1 scope, narrower than even the reference wgpu shell's
/// own already-honest "only Cargo is wired" limit (§75.10): only
/// Rust-via-Cargo (real build, then launch the resulting binary) and
/// Python (launch the interpreted source file directly, no build step)
/// are supported. Every other language with a configured `dap_command`
/// (C#, Kotlin, Java, Go, TypeScript) needs a real, already-built
/// program path this increment has no UI to collect yet -- refused with
/// a specific, honest error rather than silently doing nothing.
///
/// `program_override`: an optional real program path supplied by the UI.
/// When present it wins over every default resolution below (no build is
/// ever attempted with it), which is what makes Go/C#/Java/Kotlin/TS --
/// any language whose registry entry carries a real `dap_command` but no
/// wired build path -- launchable at all. Deliberately *not* validated
/// here beyond non-empty: the adapter itself is the real authority on
/// whether the path points at something runnable, and its own launch
/// error becomes the honest `dap_error` the UI already renders.
///
/// On success, spawns one additional real background thread that drains
/// the session's own updates and relays each one as a real backend
/// `Event` (`dap_build_failed`/`dap_stopped`/`dap_exited`/`dap_error`),
/// keyed by `doc_id` so a UI with multiple open tabs can correlate an
/// update to the right one. Returns the live `Arc<DapSession>` for the
/// caller to store and later send `DapCommand`s to.
pub fn dap_launch(
    doc_id: u64,
    path: &Path,
    breakpoints: &[spartan_dap::Breakpoint],
    out_tx: Sender<String>,
    program_override: Option<&str>,
) -> Result<Arc<DapSession>, String> {
    let registry = LanguageRegistry::curated_default();
    let profile = registry
        .profile_for_file(path)
        .ok_or_else(|| "no language profile detected for this file".to_string())?;
    let command = profile
        .dap_command
        .as_ref()
        .ok_or_else(|| format!("`{}` has no configured debug adapter", profile.id))?;
    let project_root = spartan_lsp::find_project_root(path, &profile.marker_files)
        .ok_or_else(|| "no real project root found for this file".to_string())?;

    let (needs_build, program_path): (bool, PathBuf) = match program_override {
        Some(p) if !p.trim().is_empty() => (false, PathBuf::from(p)),
        _ => match profile.id.as_str() {
            "rust"
                if profile.build_systems.iter().any(|s| s == "cargo")
                    && project_root.join("Cargo.toml").is_file() =>
            {
                (true, PathBuf::new())
            }
            "python" => (false, path.to_path_buf()),
            other => {
                return Err(format!(
                    "DAP for `{other}` needs a pre-built program path, which this increment has \
                     no way to supply yet (only Rust-via-Cargo and Python are wired)"
                ));
            }
        },
    };

    let resolved_command = resolve_dap_command(&profile.id, command);
    let session = DapSession::launch(
        &resolved_command,
        needs_build,
        &project_root,
        &program_path,
        &project_root,
        path,
        breakpoints,
    );
    let session = Arc::new(session);

    let drain_session = Arc::clone(&session);
    thread_spawn_drain(doc_id, drain_session, out_tx);

    Ok(session)
}

fn thread_spawn_drain(doc_id: u64, session: Arc<DapSession>, out_tx: Sender<String>) {
    std::thread::spawn(move || {
        while let Some(update) = session.recv_update() {
            let event = dap_update_event(doc_id, update);
            if let Ok(line) = serde_json::to_string(&event) {
                if out_tx.send(line).is_err() {
                    return;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-dap-integration-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_dap_command_fixes_the_real_bare_debugpy_gap() {
        let configured = CommandSpec {
            program: "debugpy".to_string(),
            args: vec![],
        };
        let resolved = resolve_dap_command("python", &configured);
        assert_eq!(resolved.program, "python3");
        assert_eq!(
            resolved.args,
            vec!["-m".to_string(), "debugpy.adapter".to_string()]
        );
    }

    #[test]
    fn resolve_dap_command_passes_through_every_other_real_profile() {
        let configured = CommandSpec {
            program: "lldb-dap".to_string(),
            args: vec![],
        };
        let resolved = resolve_dap_command("rust", &configured);
        assert_eq!(resolved.program, "lldb-dap");
        assert!(resolved.args.is_empty());
    }

    #[test]
    fn dap_launch_refuses_honestly_for_an_unrecognized_extension() {
        let dir = work_dir("no-profile");
        let file = dir.join("data.unknownext");
        std::fs::write(&file, "hello").unwrap();

        let (tx, _rx) = std::sync::mpsc::channel();
        match dap_launch(0, &file, &[spartan_dap::Breakpoint::line(1)], tx, None) {
            Err(message) => assert!(message.contains("no language profile")),
            Ok(_) => panic!("expected a real Err, got Ok"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dap_launch_refuses_honestly_for_a_real_unwired_compiled_language() {
        // Go has a real, configured `dap_command` (`dlv`) and a real
        // project marker (`go.mod`) -- exercising the real "recognized
        // language, real dap_command, real project root, but no
        // program_path supplied" branch (§75.98: with an override, Go
        // launches; this test covers the no-override refusal that
        // predates it).
        let dir = work_dir("go-unwired");
        std::fs::write(dir.join("go.mod"), "module example.com/x\n").unwrap();
        let file = dir.join("main.go");
        std::fs::write(&file, "package main\nfunc main() {}\n").unwrap();

        let (tx, _rx) = std::sync::mpsc::channel();
        match dap_launch(0, &file, &[spartan_dap::Breakpoint::line(1)], tx, None) {
            Err(message) => {
                assert!(
                    message.contains("go"),
                    "expected the language named in the error: {message}"
                );
                assert!(
                    message.contains("pre-built program path"),
                    "expected the real scope-limit reason: {message}"
                );
            }
            Ok(_) => panic!("expected a real Err, got Ok"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dap_launch_accepts_a_program_override_for_go() {
        // The §75.98 positive branch of the test above: the exact same
        // Go fixture (real profile, real `dlv` dap_command, real
        // `go.mod` project root) is accepted the moment a program path
        // is supplied, without `dlv` needing to be installed -- the real
        // spawn happens on `DapSession::launch`'s own background thread
        // and would surface as an async `dap_error`, never as a sync Err.
        let dir = work_dir("go-override");
        std::fs::write(dir.join("go.mod"), "module example.com/x\n").unwrap();
        let file = dir.join("main.go");
        std::fs::write(&file, "package main\nfunc main() {}\n").unwrap();

        let (tx, _rx) = std::sync::mpsc::channel();
        let session = dap_launch(
            0,
            &file,
            &[spartan_dap::Breakpoint::line(1)],
            tx,
            Some("/tmp/spartan-fake-program"),
        );
        assert!(
            session.is_ok(),
            "a supplied program path should launch: {:?}",
            session.err()
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    // A real .py file with no pyproject.toml/setup.py/requirements.txt
    // anywhere in its ancestry -- `find_project_root` genuinely can't
    // resolve a root, the same honest "no real project root" case
    // `lsp_integration`'s own tests exercise for LSP.
    #[test]
    fn dap_launch_refuses_honestly_for_python_with_no_real_project_marker() {
        let dir = work_dir("python-no-root");
        let file = dir.join("script.py");
        std::fs::write(&file, "print('hi')\n").unwrap();

        let (tx, _rx) = std::sync::mpsc::channel();
        match dap_launch(0, &file, &[spartan_dap::Breakpoint::line(1)], tx, None) {
            Err(message) => assert!(message.contains("no real project root")),
            Ok(_) => panic!("expected a real Err, got Ok"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }
}
