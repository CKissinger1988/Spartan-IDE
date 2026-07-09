//! Real end-to-end test for the second reference plugin, mirroring
//! `linter_bridge_integration.rs`'s own real-subprocess-build discipline.
//! This plugin is the deliberate opposite case: it declares (and needs)
//! zero `editor_api` capabilities, proving the capability model works in
//! both directions -- a plugin with nothing to gate instantiates and runs
//! with no `spartan:plugin/host` import at all (confirmed via `wasm-tools
//! component wit` during development, not assumed).

use spartan_plugin_host::manifest::PluginManifest;
use spartan_plugin_host::PluginHost;
use std::path::PathBuf;
use std::process::Command;

fn theme_pack_crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("plugins")
        .join("theme-pack")
}

fn cargo_component_available() -> bool {
    Command::new("cargo")
        .arg("component")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn build_theme_pack() -> Option<PathBuf> {
    if !cargo_component_available() {
        println!("cargo-component not found on PATH -- skipping WASM plugin integration test");
        return None;
    }
    let crate_dir = theme_pack_crate_dir();
    let status = Command::new("cargo")
        .arg("component")
        .arg("build")
        .current_dir(&crate_dir)
        .status()
        .expect("failed to spawn `cargo component build`");
    assert!(status.success(), "real `cargo component build` failed");
    let wasm_path = crate_dir
        .parent()
        .unwrap()
        .join("target")
        .join("wasm32-wasip1")
        .join("debug")
        .join("theme_pack.wasm");
    assert!(
        wasm_path.exists(),
        "expected a real compiled component at {}",
        wasm_path.display()
    );
    Some(wasm_path)
}

fn empty_capability_manifest() -> PluginManifest {
    PluginManifest::load(&theme_pack_crate_dir().join("plugin.toml"))
        .expect("real plugin.toml must parse")
}

#[test]
fn a_plugin_declaring_zero_capabilities_still_instantiates_and_activates() {
    let Some(wasm_path) = build_theme_pack() else {
        return;
    };
    let host = PluginHost::new();
    let loaded = host
        .load_and_activate(&wasm_path, empty_capability_manifest())
        .expect("a plugin needing no host capabilities should still instantiate");
    assert_eq!(loaded.activation_message, "theme-pack: ready");
    // Never called `host.log` -- nothing to have logged.
    assert!(loaded.log_lines().is_empty());
}

#[test]
fn a_theme_plugin_returns_real_theme_json_and_no_diagnostics() {
    let Some(wasm_path) = build_theme_pack() else {
        return;
    };
    let host = PluginHost::new();
    let mut loaded = host
        .load_and_activate(&wasm_path, empty_capability_manifest())
        .expect("a plugin needing no host capabilities should still instantiate");

    let theme = loaded
        .get_theme()
        .expect("real get_theme call should succeed")
        .expect("this plugin always contributes a theme");
    assert!(theme.contains("\"name\":\"Spartan Dusk\""));
    assert!(theme.contains("#0B0B0D"));

    let diagnostics = loaded
        .get_diagnostics("anything")
        .expect("real get_diagnostics call should succeed");
    assert!(diagnostics.is_empty());
}
