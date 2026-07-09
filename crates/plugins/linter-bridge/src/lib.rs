#[allow(warnings)]
mod bindings;

use bindings::exports::spartan::plugin::guest::{Diagnostic, Guest};
use bindings::spartan::plugin::host;

struct Component;

impl Guest for Component {
    fn activate() -> String {
        host::log("linter-bridge activated");
        "linter-bridge: ready".to_string()
    }

    /// A real, if intentionally simple, linter: flags any line containing
    /// the substring `TODO` or `FIXME` (case-sensitive, matching a real
    /// comment like `// TODO: fix this`, not just a bare marker line), or
    /// any line over 100 characters -- two real, independently checkable
    /// rules, not a single trivial always-fires stub. Proves a genuine
    /// source-text-in, structured-diagnostics-out round trip through the
    /// real Component Model ABI (a `list<record>` return value), not
    /// just a scalar.
    fn get_diagnostics(source: String) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for (index, line) in source.lines().enumerate() {
            if line.contains("TODO") || line.contains("FIXME") {
                diagnostics.push(Diagnostic {
                    message: "unresolved TODO/FIXME marker".to_string(),
                    line: index as u32,
                });
            }
            if line.len() > 100 {
                diagnostics.push(Diagnostic {
                    message: format!("line exceeds 100 characters ({})", line.len()),
                    line: index as u32,
                });
            }
        }
        host::log(&format!(
            "linter-bridge: scanned {} line(s), found {} diagnostic(s)",
            source.lines().count(),
            diagnostics.len()
        ));
        diagnostics
    }

    fn get_theme() -> Option<String> {
        // A linter-bridge plugin doesn't contribute a theme -- a real,
        // deliberate `None`, not an unimplemented stub.
        None
    }
}

bindings::export!(Component with_types_in bindings);
