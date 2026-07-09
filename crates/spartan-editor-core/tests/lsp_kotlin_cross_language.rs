//! Real, executed integration test against a real `kotlin-language-server`
//! subprocess -- the first real, live exercise of Kotlin's `lsp_command`
//! anywhere in this project's history (§75.45). Mirrors
//! `lsp_integration.rs`'s own rust-analyzer test shape exactly (same
//! `LspSession::spawn`/`notify_edit`/`poll_updates` path, same
//! wait-for-diagnostics-then-wait-for-clear structure), matching this
//! crate's own established cross-language verification pattern
//! (`dap_python_cross_language.rs` did the same for DAP against a second
//! real adapter). Skips (rather than fails) if `kotlin-language-server`
//! isn't on this machine.

use spartan_editor_core::lsp_session::{LspSession, LspUpdate};
use spartan_languages::CommandSpec;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn kotlin_language_server_available() -> bool {
    std::process::Command::new("kotlin-language-server")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

const MAIN_KT_WITH_ERROR: &str =
    "fun main() {\n    val x: Int = \"not a number\"\n    println(x)\n}\n";

const MAIN_KT_CORRECTED: &str = "fun main() {\n    val x: Int = 42\n    println(x)\n}\n";

fn write_fixture_project(dir: &std::path::Path, main_kt: &str) -> PathBuf {
    let src_dir = dir.join("src").join("main").join("kotlin");
    std::fs::create_dir_all(&src_dir).unwrap();
    // A real, minimal marker file -- matches `languages.toml`'s own
    // Kotlin `marker_files = ["build.gradle.kts"]`, though
    // `LspSession::spawn` itself doesn't require Gradle to actually
    // resolve; this only needs to be present for real project-root
    // detection to look genuine.
    std::fs::write(
        dir.join("build.gradle.kts"),
        "plugins {\n    kotlin(\"jvm\") version \"1.9.0\"\n}\n",
    )
    .unwrap();
    let main_path = src_dir.join("Main.kt");
    std::fs::write(&main_path, main_kt).unwrap();
    main_path
}

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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
fn real_kotlin_language_server_reports_and_clears_a_real_diagnostic() {
    if !kotlin_language_server_available() {
        eprintln!("SKIP: kotlin-language-server not found on this machine");
        return;
    }
    let dir = work_dir("editor-core-lsp-kotlin-test");
    let main_path = write_fixture_project(&dir, MAIN_KT_WITH_ERROR);

    let command = CommandSpec {
        program: "kotlin-language-server".to_string(),
        args: vec![],
    };
    let session = LspSession::spawn(&command, &dir, &main_path, MAIN_KT_WITH_ERROR)
        .expect("spawn real kotlin-language-server session");

    // A real, live finding (see `lsp.rs`'s own new `INITIALIZE_TIMEOUT`
    // doc comment): kotlin-language-server's real JVM cold start needed a
    // dedicated, longer `initialize` timeout than every other real
    // language server this crate has driven. This outer wait must exceed
    // the sum of that internal `INITIALIZE_TIMEOUT` (45s) and the
    // subsequent internal `INDEXING_TIMEOUT` (90s) the background thread
    // runs through sequentially before this test's own channel poll can
    // see anything at all.
    let initial = wait_for_next_update(&session, Duration::from_secs(150));
    if initial.is_none() {
        session.shutdown();
        panic!("expected an initial diagnostics update within the real indexing budget");
    }
    let initial = initial.unwrap();
    println!("real initial diagnostics: {initial:?}");
    // Real, live-confirmed output (not assumed): a bare fixture with no
    // real resolved Gradle classpath also reports real stdlib-resolution
    // noise ("Cannot access built-in declaration 'kotlin.Int'",
    // "Unresolved reference: println") alongside the deliberate type
    // error -- an honest byproduct of this test's minimal, unresolved
    // fixture project, not something a real IDE session with a real
    // classpath would show. The precise, meaningful assertion is that the
    // real deliberate type mismatch is present.
    assert!(
        initial.iter().any(|l| l.contains("Type mismatch")),
        "expected a real 'Type mismatch' diagnostic for the fixture's deliberate error, got: {initial:?}"
    );

    session.notify_edit(MAIN_KT_CORRECTED.to_string());

    let after_fix = wait_for_next_update(&session, Duration::from_secs(30));
    if after_fix.is_none() {
        session.shutdown();
        panic!("expected a diagnostics update after sending the corrected text");
    }
    let after_fix = after_fix.unwrap();
    println!("real diagnostics after correction: {after_fix:?}");
    assert!(
        !after_fix.iter().any(|l| l.contains("Type mismatch")),
        "expected the real 'Type mismatch' diagnostic to clear after the live correction, got: {after_fix:?}"
    );

    session.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
