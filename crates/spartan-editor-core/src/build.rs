//! Real build-system integration for DAP -- the gap §75.8 named explicitly
//! as out of scope for that pass ("`dap_command` only names the debug
//! *adapter*, not how to build a debuggable binary"). Runs a real
//! `cargo build --message-format=json` and parses its streamed JSON output
//! for the resulting binary's path, using cargo's own documented message
//! format -- verified against a real `cargo build` run (both a successful
//! build and a real compile error) before writing this, not guessed from
//! documentation alone.

use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub enum BuildResult {
    /// The real path to the compiled debug binary.
    Success(PathBuf),
    /// Real, rendered compiler diagnostics (`message.rendered` from each
    /// `compiler-message` cargo JSON entry at `level: "error"`) -- or a
    /// single explanatory line if cargo itself couldn't be spawned or
    /// produced no diagnostics for some other reason.
    Failure(Vec<String>),
}

/// Spawns `cargo build --message-format=json` in `project_root` and blocks
/// until it finishes -- callers must run this on a background thread, not
/// the render thread, since a real build can take anywhere from under a
/// second to minutes. Only ever looks for a `bin` target's `executable`
/// (a lib-only crate has no debuggable binary to find).
pub fn build_debug_binary(project_root: &Path) -> BuildResult {
    let mut child = match Command::new("cargo")
        .arg("build")
        .arg("--message-format=json")
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return BuildResult::Failure(vec![format!("failed to spawn cargo: {e}")]),
    };

    let stdout = child.stdout.take().expect("piped stdout");
    let reader = BufReader::new(stdout);
    let mut executable: Option<PathBuf> = None;
    let mut diagnostics = Vec::new();
    let mut success = false;

    for line in reader.lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match msg.get("reason").and_then(Value::as_str) {
            Some("compiler-artifact") => {
                let is_bin = msg["target"]["kind"]
                    .as_array()
                    .map(|kinds| kinds.iter().any(|k| k.as_str() == Some("bin")))
                    .unwrap_or(false);
                if is_bin {
                    if let Some(exe) = msg["executable"].as_str() {
                        executable = Some(PathBuf::from(exe));
                    }
                }
            }
            Some("compiler-message") => {
                if msg["message"]["level"].as_str() == Some("error") {
                    if let Some(rendered) = msg["message"]["rendered"].as_str() {
                        diagnostics.push(rendered.to_string());
                    }
                }
            }
            Some("build-finished") => {
                success = msg["success"].as_bool().unwrap_or(false);
            }
            _ => {}
        }
    }
    let _ = child.wait();

    if success {
        match executable {
            Some(path) => BuildResult::Success(path),
            None => BuildResult::Failure(vec![
                "build succeeded but no bin target executable was reported (is this a lib-only \
                 crate?)"
                    .to_string(),
            ]),
        }
    } else {
        if diagnostics.is_empty() {
            diagnostics.push("cargo build failed with no diagnostic messages captured".to_string());
        }
        BuildResult::Failure(diagnostics)
    }
}
