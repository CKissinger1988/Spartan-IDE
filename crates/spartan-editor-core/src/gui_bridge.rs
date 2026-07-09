//! Real §6.2 dev-server bridge (task #12): spawns the real `gui-builder`
//! CLI (a real Node subprocess, `gui-builder/dist/cli.js`) for both real
//! sync directions -- parsing the active file's real JSX/TSX into a real
//! `ComponentNode` tree (§75.41, "Code -> Canvas"), and, as of §75.42,
//! applying a real structured `CanvasEdit` and getting back real
//! regenerated source ("Canvas -> Code") -- and delivers each result back
//! to the render loop without ever blocking it, the exact same
//! spawn-on-a-thread, `mpsc::channel`, non-blocking-poll pattern
//! `build.rs`'s own DAP build integration (§75.10) already established for
//! a different real subprocess.
//!
//! Deliberately a v1: one file/edit in, one JSON result out, no persistent
//! server, no file watching, no HMR -- §6.2 step 1's own "diffed against
//! last-known tree, re-renders only changed nodes" remains unbuilt. See
//! `gui-builder/README.md` and this crate's own README for the full,
//! honest scope.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;

pub struct ComponentTreeRequest {
    pub receiver: mpsc::Receiver<Result<String, String>>,
}

/// A real, in-flight `bundle` request (§75.52) -- `Ok(code)` is the real,
/// self-contained JS bundle text `gui-builder`'s own `bundleComponent`
/// produced (esbuild-bundled, ready to embed in a real HTML page and
/// run). Never written to disk.
pub struct BundleRequest {
    pub receiver: mpsc::Receiver<Result<String, String>>,
}

/// A real, in-flight `CanvasEdit` application (§75.42) -- `Ok(new_source)`
/// is the real, regenerated whole-file source `gui-builder`'s own
/// `applyCanvasEdit` produced (never written to disk by the CLI itself;
/// the caller is responsible for feeding it into the live `Document`, the
/// same as any other in-memory edit, so it correctly goes through undo/
/// dirty-tracking/Ctrl+S like a keystroke would).
pub struct ApplyEditRequest {
    pub receiver: mpsc::Receiver<Result<String, String>>,
}

/// Real extension-based check for "does `gui-builder`'s CLI know how to
/// parse this file" -- deliberately narrower than the full
/// `spartan-languages` `typescript` profile (which also covers plain
/// `.ts`/`.js`), matching exactly what `gui-builder`'s own real parser
/// targets.
pub fn is_component_file(path: &str) -> bool {
    path.ends_with(".jsx") || path.ends_with(".tsx")
}

/// Real, deliberately simple location strategy for `gui-builder`'s
/// compiled CLI: `$SPARTAN_GUI_BUILDER_DIR/dist/cli.js` if set, else
/// `./gui-builder/dist/cli.js` relative to the current working directory
/// -- a real, named development-only heuristic (this repo's own layout
/// during `cargo run`), not a shipped packaging story. A real production
/// build would need `gui-builder` bundled as an installed asset next to
/// the binary, a separate, real packaging decision not attempted here.
fn locate_cli() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SPARTAN_GUI_BUILDER_DIR") {
        let candidate = PathBuf::from(dir).join("dist").join("cli.js");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let candidate = PathBuf::from("gui-builder").join("dist").join("cli.js");
    if candidate.is_file() {
        return Some(candidate);
    }
    None
}

/// Spawns the real `node <cli.js> <file>` subprocess on its own thread,
/// returning immediately with a receiver the render loop polls
/// non-blockingly (`AboutToWait`) -- never blocks the render thread
/// itself. `Ok(json)` is the real stdout payload (already confirmed to be
/// valid JSON before being sent, so a caller can safely splice it into an
/// `evaluate_script` call without a second validation pass); `Err(message)`
/// covers every real failure mode (`node`/`gui-builder` not found, a real
/// parse error, malformed subprocess output) with a human-readable
/// message, never a panic.
pub fn spawn_component_tree_request(file_path: &Path) -> ComponentTreeRequest {
    let (sender, receiver) = mpsc::channel();
    let file_path = file_path.to_path_buf();
    std::thread::spawn(move || {
        let result = run_cli(&file_path);
        let _ = sender.send(result);
    });
    ComponentTreeRequest { receiver }
}

fn run_cli(file_path: &Path) -> Result<String, String> {
    let Some(cli_path) = locate_cli() else {
        return Err(
            "gui-builder CLI not found (set SPARTAN_GUI_BUILDER_DIR or run from the repo root, \
             after `cd gui-builder && npm install && npm run build`)"
                .to_string(),
        );
    };
    let output = Command::new("node").arg(&cli_path).arg(file_path).output();
    let output = match output {
        Ok(o) => o,
        Err(e) => return Err(format!("failed to spawn node: {e}")),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gui-builder CLI failed: {stderr}"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if serde_json::from_str::<serde_json::Value>(&stdout).is_err() {
        return Err("gui-builder CLI produced invalid JSON".to_string());
    }
    Ok(stdout)
}

/// Spawns the real `node <cli.js> bundle <file>` subprocess on its own
/// thread (§75.52, the real live-visual-rendering mechanism) -- same
/// non-blocking-poll contract as `spawn_component_tree_request`.
/// `Ok(code)` is the real, self-contained JS bundle text; `Err(message)`
/// covers every real failure (missing dependency in the target project,
/// a real syntax error, subprocess spawn failure) with the real,
/// unmodified `gui-builder` error text.
pub fn spawn_bundle_request(file_path: &Path) -> BundleRequest {
    let (sender, receiver) = mpsc::channel();
    let file_path = file_path.to_path_buf();
    std::thread::spawn(move || {
        let result = run_bundle_cli(&file_path);
        let _ = sender.send(result);
    });
    BundleRequest { receiver }
}

fn run_bundle_cli(file_path: &Path) -> Result<String, String> {
    let Some(cli_path) = locate_cli() else {
        return Err(
            "gui-builder CLI not found (set SPARTAN_GUI_BUILDER_DIR or run from the repo root, \
             after `cd gui-builder && npm install && npm run build`)"
                .to_string(),
        );
    };
    let output = Command::new("node")
        .arg(&cli_path)
        .arg("bundle")
        .arg(file_path)
        .output();
    let output = match output {
        Ok(o) => o,
        Err(e) => return Err(format!("failed to spawn node: {e}")),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gui-builder CLI bundle failed: {stderr}"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let value: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(v) => v,
        Err(_) => return Err("gui-builder CLI bundle produced invalid JSON".to_string()),
    };
    match value.get("code").and_then(|s| s.as_str()) {
        Some(s) => Ok(s.to_string()),
        None => Err("gui-builder CLI bundle response missing a 'code' field".to_string()),
    }
}

/// Spawns the real `node <cli.js> apply <editJson>` subprocess on its own
/// thread, piping `current_source` (the live in-memory buffer, not
/// whatever's on disk) to its stdin -- the same non-blocking-poll contract
/// as `spawn_component_tree_request`. `Ok(new_source)` is the real
/// regenerated whole-file source; `Err(message)` covers every real failure
/// mode (unknown node id, an unsupported edit shape, subprocess spawn
/// failure) with a human-readable message.
pub fn spawn_apply_edit_request(current_source: String, edit_json: String) -> ApplyEditRequest {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = run_apply_cli(&current_source, &edit_json);
        let _ = sender.send(result);
    });
    ApplyEditRequest { receiver }
}

fn run_apply_cli(current_source: &str, edit_json: &str) -> Result<String, String> {
    let Some(cli_path) = locate_cli() else {
        return Err(
            "gui-builder CLI not found (set SPARTAN_GUI_BUILDER_DIR or run from the repo root, \
             after `cd gui-builder && npm install && npm run build`)"
                .to_string(),
        );
    };
    let mut child = match Command::new("node")
        .arg(&cli_path)
        .arg("apply")
        .arg(edit_json)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return Err(format!("failed to spawn node: {e}")),
    };

    // Write and drop stdin in its own scope so the pipe closes (signaling
    // real EOF to the CLI's `readFileSync(0, ...)` read) before
    // `wait_with_output` blocks for the real subprocess's exit.
    {
        let Some(mut stdin) = child.stdin.take() else {
            return Err("failed to open stdin for gui-builder CLI".to_string());
        };
        if let Err(e) = stdin.write_all(current_source.as_bytes()) {
            return Err(format!(
                "failed to write source to gui-builder CLI stdin: {e}"
            ));
        }
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => return Err(format!("failed to wait for gui-builder CLI: {e}")),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gui-builder CLI apply failed: {stderr}"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let value: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(v) => v,
        Err(_) => return Err("gui-builder CLI apply produced invalid JSON".to_string()),
    };
    match value.get("source").and_then(|s| s.as_str()) {
        Some(s) => Ok(s.to_string()),
        None => Err("gui-builder CLI apply response missing a 'source' field".to_string()),
    }
}
