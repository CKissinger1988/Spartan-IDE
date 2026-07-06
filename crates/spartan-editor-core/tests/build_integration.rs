//! Real, executed test against a real `cargo build` -- no mocked
//! compiler. Exercises `build::build_debug_binary` directly against a
//! generated fixture crate, matching the same temp-dir-outside-the-
//! workspace pattern already established for `lsp_integration.rs`/
//! `dap_integration.rs` (rust-analyzer/cargo both refuse a manifest that's
//! a descendant of this workspace's own root without being a declared
//! member).

use spartan_editor_core::build::{build_debug_binary, BuildResult};
use std::path::PathBuf;

fn work_dir(name: &str) -> PathBuf {
    let base = std::env::temp_dir();
    let dir = base.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    dir
}

fn write_fixture(dir: &std::path::Path, main_rs: &str) {
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"fixture-crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/main.rs"), main_rs).unwrap();
}

#[test]
fn a_real_successful_build_reports_the_real_executable_path() {
    let dir = work_dir("editor-core-build-success-test");
    write_fixture(&dir, "fn main() { println!(\"hi\"); }\n");

    match build_debug_binary(&dir) {
        BuildResult::Success(path) => {
            assert!(
                path.is_file(),
                "reported executable path should really exist on disk: {}",
                path.display()
            );
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            assert_eq!(name, "fixture-crate");
        }
        BuildResult::Failure(diagnostics) => {
            panic!("expected a successful build, got failures: {diagnostics:?}")
        }
    }
}

#[test]
fn a_real_compile_error_reports_the_real_rendered_diagnostic() {
    let dir = work_dir("editor-core-build-failure-test");
    write_fixture(&dir, "fn main() { let x: i32 = \"not a number\"; }\n");

    match build_debug_binary(&dir) {
        BuildResult::Success(path) => {
            panic!("expected the build to fail on a real type error, got a binary at {path:?}")
        }
        BuildResult::Failure(diagnostics) => {
            assert!(
                diagnostics.iter().any(|d| d.contains("E0308")),
                "expected a real E0308 mismatched-types diagnostic, got: {diagnostics:?}"
            );
        }
    }
}
