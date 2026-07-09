//! Real, executed integration test against the real `gui-builder` CLI
//! subprocess (§75.41, task #12). Skips (rather than fails) if `node`
//! isn't on `$PATH` or `gui-builder/dist/cli.js` hasn't been built yet
//! (`cd gui-builder && npm install && npm run build`) -- matching this
//! crate's own established `lsp_integration.rs`/`dap_integration.rs`
//! self-skip convention for real external-tool dependencies.

use spartan_editor_core::gui_bridge;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

fn gui_builder_dir() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is this crate's own directory
    // (`crates/spartan-editor-core`), reliable regardless of `cargo
    // test`'s actual working directory (which is *not* the repo root by
    // default, unlike a real `cargo run` from the repo root).
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("gui-builder")
}

fn cli_available() -> bool {
    let node_found = std::process::Command::new("node")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    node_found && gui_builder_dir().join("dist").join("cli.js").is_file()
}

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("spartan_gui_bridge_test_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_real_jsx_file_produces_a_real_component_tree() {
    if !cli_available() {
        eprintln!("SKIP: node or gui-builder/dist/cli.js not available");
        return;
    }
    std::env::set_var("SPARTAN_GUI_BUILDER_DIR", gui_builder_dir());

    let dir = work_dir("real_tree");
    let file = dir.join("App.jsx");
    std::fs::File::create(&file)
        .unwrap()
        .write_all(br#"const X = () => <div className="app">Hello</div>;"#)
        .unwrap();

    let request = gui_bridge::spawn_component_tree_request(&file);
    let result = request
        .receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("gui-builder CLI subprocess should respond within 30s");

    let _ = std::fs::remove_dir_all(&dir);

    let json = result.expect("a real, valid JSX file should produce Ok(json), not an error");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let roots = value["roots"]
        .as_array()
        .expect("roots should be a real array");
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0]["tagName"], "div");
}

#[test]
fn a_missing_file_produces_a_real_error_not_a_panic() {
    if !cli_available() {
        eprintln!("SKIP: node or gui-builder/dist/cli.js not available");
        return;
    }
    std::env::set_var("SPARTAN_GUI_BUILDER_DIR", gui_builder_dir());

    let request = gui_bridge::spawn_component_tree_request(&PathBuf::from(
        "/nonexistent/path/does/not/exist.jsx",
    ));
    let result = request
        .receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("gui-builder CLI subprocess should respond within 30s");

    assert!(
        result.is_err(),
        "a missing file should produce a real Err, not Ok"
    );
}

/// Real §75.42 Canvas -> Code round trip: a real `PropChange` applied
/// against a real in-memory source string (never written to disk --
/// proving the CLI really reads from stdin, not from whatever's on disk
/// at the path a real caller would otherwise pass).
#[test]
fn a_real_prop_change_produces_real_regenerated_source() {
    if !cli_available() {
        eprintln!("SKIP: node or gui-builder/dist/cli.js not available");
        return;
    }
    std::env::set_var("SPARTAN_GUI_BUILDER_DIR", gui_builder_dir());

    let source = r#"const X = () => <div className="app">Hello</div>;"#.to_string();
    let edit_json =
        r#"{"kind":"PropChange","nodeId":"n0","prop":"className","value":"updated"}"#.to_string();

    let request = gui_bridge::spawn_apply_edit_request(source, edit_json);
    let result = request
        .receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("gui-builder CLI apply subprocess should respond within 30s");

    let new_source = result.expect("a real, valid PropChange should produce Ok(new_source)");
    assert!(
        new_source.contains(r#"className="updated""#),
        "regenerated source should contain the real new prop value, got: {new_source}"
    );
    assert!(
        new_source.contains("const X = () =>"),
        "recast should preserve the untouched surrounding code verbatim, got: {new_source}"
    );
}

/// A real, unknown node id (e.g. the source changed structurally since the
/// id was last computed) must produce a real `Err`, not a panic or a
/// silently unchanged source.
#[test]
fn an_unknown_node_id_produces_a_real_error() {
    if !cli_available() {
        eprintln!("SKIP: node or gui-builder/dist/cli.js not available");
        return;
    }
    std::env::set_var("SPARTAN_GUI_BUILDER_DIR", gui_builder_dir());

    let source = r#"const X = () => <div>Hello</div>;"#.to_string();
    let edit_json =
        r#"{"kind":"PropChange","nodeId":"does-not-exist","prop":"id","value":"x"}"#.to_string();

    let request = gui_bridge::spawn_apply_edit_request(source, edit_json);
    let result = request
        .receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("gui-builder CLI apply subprocess should respond within 30s");

    assert!(
        result.is_err(),
        "an unknown node id should produce a real Err, not Ok"
    );
}
