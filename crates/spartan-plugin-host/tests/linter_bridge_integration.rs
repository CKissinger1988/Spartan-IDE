//! Real end-to-end test: runs a real `cargo component build` against the
//! real `linter-bridge` reference plugin crate (`crates/plugins/linter-
//! bridge`), then loads and exercises the real resulting `.wasm`
//! component through `spartan_plugin_host::PluginHost` -- the same "spawn
//! the real external tool, don't mock it" discipline this workspace's own
//! `dap_integration.rs`/`lsp_integration.rs`/`build.rs` tests already use.
//! Self-skips (rather than failing) if `cargo component` isn't installed,
//! matching those same tests' own self-skip convention for `lldb-dap`/
//! `rust-analyzer`/`debugpy`.

use spartan_plugin_host::manifest::PluginManifest;
use spartan_plugin_host::PluginHost;
use std::path::PathBuf;
use std::process::Command;

fn linter_bridge_crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("plugins")
        .join("linter-bridge")
}

fn cargo_component_available() -> bool {
    Command::new("cargo")
        .arg("component")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Builds the real plugin and returns the real path to the compiled
/// `.wasm` component -- `None` if `cargo component` isn't on `PATH`.
fn build_linter_bridge() -> Option<PathBuf> {
    if !cargo_component_available() {
        println!("cargo-component not found on PATH -- skipping WASM plugin integration test");
        return None;
    }
    let crate_dir = linter_bridge_crate_dir();
    let status = Command::new("cargo")
        .arg("component")
        .arg("build")
        .current_dir(&crate_dir)
        .status()
        .expect("failed to spawn `cargo component build`");
    assert!(status.success(), "real `cargo component build` failed");
    // `crates/plugins/` is its own separate cargo workspace (see its own
    // `Cargo.toml`), so the compiled artifact lands in *its* shared
    // `target/`, not a per-crate one.
    let wasm_path = crate_dir
        .parent()
        .unwrap()
        .join("target")
        .join("wasm32-wasip1")
        .join("debug")
        .join("linter_bridge.wasm");
    assert!(
        wasm_path.exists(),
        "expected a real compiled component at {}",
        wasm_path.display()
    );
    Some(wasm_path)
}

fn full_manifest() -> PluginManifest {
    PluginManifest::load(&linter_bridge_crate_dir().join("plugin.toml"))
        .expect("real plugin.toml must parse")
}

#[test]
fn a_real_compiled_plugin_activates_and_reports_ready() {
    let Some(wasm_path) = build_linter_bridge() else {
        return;
    };
    let host = PluginHost::new();
    let loaded = host
        .load_and_activate(&wasm_path, full_manifest())
        .expect("real plugin should instantiate and activate");
    assert_eq!(loaded.activation_message, "linter-bridge: ready");
    assert!(loaded
        .log_lines()
        .iter()
        .any(|l| l == "linter-bridge activated"));
}

#[test]
fn a_real_compiled_plugin_finds_real_diagnostics_in_real_source_text() {
    let Some(wasm_path) = build_linter_bridge() else {
        return;
    };
    let host = PluginHost::new();
    let mut loaded = host
        .load_and_activate(&wasm_path, full_manifest())
        .expect("real plugin should instantiate and activate");

    let source = "fn main() {\n    // TODO\n    println!(\"hi\");\n}\n";
    let diagnostics = loaded
        .get_diagnostics(source)
        .expect("real get_diagnostics call should succeed");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].line, 1);
    assert!(diagnostics[0].message.contains("TODO"));

    let clean_source = "fn main() {}\n";
    let clean_diagnostics = loaded
        .get_diagnostics(clean_source)
        .expect("real get_diagnostics call should succeed");
    assert!(clean_diagnostics.is_empty());
}

#[test]
fn a_real_compiled_plugin_reports_no_theme() {
    let Some(wasm_path) = build_linter_bridge() else {
        return;
    };
    let host = PluginHost::new();
    let mut loaded = host
        .load_and_activate(&wasm_path, full_manifest())
        .expect("real plugin should instantiate and activate");
    assert_eq!(loaded.get_theme().unwrap(), None);
}

#[test]
fn a_manifest_that_never_declared_the_log_capability_fails_real_instantiation() {
    // Real capability enforcement, exercised end-to-end: the compiled
    // component's own binary literally imports `spartan:plugin/host@0.1.0
    // #log` (confirmed via `wasm-tools component wit` during development)
    // -- with no manifest-declared `"log"` capability, this host never
    // adds that import to the `Linker`, so real instantiation fails with
    // a real "unknown import" error rather than the plugin silently
    // running without logging.
    let Some(wasm_path) = build_linter_bridge() else {
        return;
    };
    let bare_manifest = PluginManifest::parse(
        r#"
            [plugin]
            name = "spartan-linter-bridge"
            version = "0.1.0"
        "#,
    )
    .unwrap();
    let host = PluginHost::new();
    let result = host.load_and_activate(&wasm_path, bare_manifest);
    assert!(
        result.is_err(),
        "instantiation should fail without the declared log capability"
    );
}
