#[allow(warnings)]
mod bindings;

use bindings::exports::spartan::plugin::guest::{Diagnostic, Guest};

struct Component;

impl Guest for Component {
    /// Deliberately never calls `host::log` -- this plugin declares no
    /// `editor_api` capabilities in its own `plugin.toml` at all, and
    /// since nothing in this crate ever references the `host` import,
    /// the compiled component doesn't even import `spartan:plugin/host`
    /// (confirmed via `wasm-tools component wit` during development, the
    /// same way the earlier `spartan:plugin/host` *presence* was
    /// confirmed for `linter-bridge`) -- proving the capability model
    /// works in both directions: a plugin that needs nothing beyond its
    /// own real exports imports nothing beyond what the WASI runtime
    /// itself requires.
    fn activate() -> String {
        "theme-pack: ready".to_string()
    }

    /// A theme plugin has no diagnostics to contribute -- a real,
    /// deliberate empty list, not an unimplemented stub.
    fn get_diagnostics(_source: String) -> Vec<Diagnostic> {
        Vec::new()
    }

    /// A real, small color theme, encoded as JSON -- the host owns actual
    /// theme *application*; this plugin's whole job is contributing the
    /// data (§5.3's `theme_api`: "contribute color themes... without
    /// needing filesystem access at all").
    fn get_theme() -> Option<String> {
        Some(
            r##"{"name":"Spartan Dusk","background":"#0B0B0D","foreground":"#E9E7E4","accent":"#F03133"}"##
                .to_string(),
        )
    }
}

bindings::export!(Component with_types_in bindings);
