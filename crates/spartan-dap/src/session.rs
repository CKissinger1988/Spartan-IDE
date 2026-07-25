//! A real, live DAP session (launch, breakpoint hit, continue/step,
//! stack/variable inspection), adapted from `crates/spartan-editor-core/
//! src/dap_session.rs` (§75.8) for a background-thread IPC consumer
//! instead of a render-loop poller. Two real, deliberate differences from
//! the copy this was adapted from, both named rather than silently
//! changed (mirroring `spartan-lsp::session`'s own precedent):
//!
//! 1. **Structured stop info, not display-ready strings.** The original
//!    had no debug UI anywhere in its crate yet, so it formatted stack/
//!    variable data into printable text. `DapStopped` derives `Serialize`
//!    and is sent as-is inside a real backend `Event`.
//! 2. **An explicit `Disconnect` command, not drop-triggered shutdown.**
//!    The original's `shutdown(self)` dropped its own `cmd_tx` field to
//!    make the background thread's blocking `recv()` return `Err` and
//!    exit naturally. That relies on being the *sole* owner of the
//!    session -- this crate's real caller (`spartan-backend`) shares one
//!    session via `Arc` between the request-handling thread (sending
//!    commands) and a dedicated thread draining updates, so no single
//!    owner ever legitimately drops last. A real `DapCommand::Disconnect`
//!    variant lets shutdown flow through the exact same `&self` command
//!    channel every other command already uses.

use crate::build::{self, BuildResult};
use crate::client::{Breakpoint, DapClient, DEFAULT_TIMEOUT};
use serde::Serialize;
use serde_json::Value;
use spartan_languages::CommandSpec;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

pub enum DapCommand {
    Continue,
    StepOver,
    StepInto,
    Disconnect,
    /// Evaluate an expression in the current top stack frame (a watch
    /// expression or a REPL eval). Only meaningful while stopped -- the
    /// real result (or an error string) is returned over `reply`, out of
    /// band from the one-way `DapUpdate` stream.
    Evaluate {
        expression: String,
        reply: Sender<Result<String, String>>,
    },
    /// Edit a variable's live value in the current top scope while
    /// stopped, via a real DAP `setVariable`. A discrete request/response
    /// like `Evaluate` (never steps or continues), but unlike `Evaluate`
    /// it also pushes a fresh `DapUpdate::Stopped` on success so the
    /// Variables panel (and any open Watches, which already re-evaluate
    /// on every `Stopped`) both reflect the real new value immediately.
    SetVariable {
        name: String,
        value: String,
        reply: Sender<Result<String, String>>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct DapVariable {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DapFrame {
    pub name: String,
    pub line: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DapStopped {
    pub thread_id: i64,
    pub reason: String,
    pub frame: Option<DapFrame>,
    pub variables: Vec<DapVariable>,
}

#[derive(Debug)]
pub enum DapUpdate {
    /// A real Cargo (or other configured build system) compile error --
    /// reported before any debug adapter is even spawned.
    BuildFailed(Vec<String>),
    Stopped(DapStopped),
    Exited,
    Error(String),
    /// A real DAP `output` event -- a logpoint firing, or (on adapters
    /// that relay it this way) the debuggee's own stdout/stderr.
    /// `category` is the adapter's own real DAP category string
    /// (`"console"`/`"stdout"`/`"stderr"`, etc., defaulting to
    /// `"console"` per the spec when the adapter omits it); `text` is
    /// the real output text, unmodified. A real, live-confirmed
    /// `debugpy` behavior, not assumed from the spec: a logpoint's own
    /// interpolated message arrives with the *identical* `"stdout"`
    /// category as the debuggee's genuine `print()` output -- there is
    /// no separate "this came from a logpoint" marker, so a caller can't
    /// (and doesn't need to) distinguish the two. A real `"telemetry"`
    /// category (adapter-internal diagnostic pings, no user value) is
    /// already filtered out before this variant is ever constructed.
    Output {
        category: String,
        text: String,
    },
}

pub struct DapSession {
    cmd_tx: Sender<DapCommand>,
    updates_rx: Mutex<Receiver<DapUpdate>>,
}

impl DapSession {
    /// Never blocks the caller: a real Cargo build (if `needs_build` is
    /// true -- Python and other non-compiled languages skip straight to
    /// spawning the adapter), the subprocess spawn, and the full launch/
    /// breakpoint handshake all happen on a dedicated background thread.
    #[allow(clippy::too_many_arguments)]
    pub fn launch(
        adapter: &CommandSpec,
        needs_build: bool,
        project_root: &Path,
        program_path: &Path,
        cwd: &Path,
        source_path: &Path,
        breakpoints: &[Breakpoint],
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<DapCommand>();
        let (updates_tx, updates_rx) = mpsc::channel::<DapUpdate>();

        let adapter_program = adapter.program.clone();
        let adapter_args = adapter.args.clone();
        let project_root = project_root.to_path_buf();
        let mut program_path = program_path.to_path_buf();
        let cwd = cwd.to_string_lossy().to_string();
        let source_path = source_path.to_string_lossy().to_string();
        let breakpoints = breakpoints.to_vec();

        thread::spawn(move || {
            if needs_build {
                match build::build_debug_binary(&project_root) {
                    BuildResult::Success(exe) => program_path = exe,
                    BuildResult::Failure(diags) => {
                        let _ = updates_tx.send(DapUpdate::BuildFailed(diags));
                        return;
                    }
                }
            }
            let program = program_path.to_string_lossy().to_string();

            let args: Vec<&str> = adapter_args.iter().map(String::as_str).collect();
            let mut client = match DapClient::spawn_with_args(&adapter_program, &args) {
                Ok(c) => c,
                Err(e) => {
                    let _ = updates_tx.send(DapUpdate::Error(format!(
                        "failed to spawn {adapter_program}: {e}"
                    )));
                    return;
                }
            };

            let mut thread_id =
                match client.launch_and_break(&program, &cwd, &source_path, &breakpoints) {
                    Some(result) => {
                        let tid = result["stopped"]["body"]["threadId"].as_i64().unwrap_or(0);
                        let reason = result["stopped"]["body"]["reason"]
                            .as_str()
                            .unwrap_or("breakpoint");
                        let stopped = describe_stop(&mut client, tid, reason);
                        let _ = updates_tx.send(DapUpdate::Stopped(stopped));
                        tid
                    }
                    None => {
                        let _ = updates_tx.send(DapUpdate::Error(
                            "launch/breakpoint sequence did not complete".to_string(),
                        ));
                        client.shutdown();
                        return;
                    }
                };

            while let Ok(cmd) = cmd_rx.recv() {
                let resp = match cmd {
                    DapCommand::Continue => client.continue_(thread_id),
                    DapCommand::StepOver => client.step_over(thread_id),
                    DapCommand::StepInto => client.step_into(thread_id),
                    DapCommand::Disconnect => break,
                    DapCommand::Evaluate { expression, reply } => {
                        // Evaluate is a discrete request/response against the
                        // stopped adapter -- it never steps or continues, so
                        // it deliberately skips `wait_for_stop_or_exit`.
                        let result = evaluate_in_current_frame(&mut client, thread_id, &expression);
                        let _ = reply.send(result);
                        continue;
                    }
                    DapCommand::SetVariable { name, value, reply } => {
                        // Same discrete-request shape as Evaluate. On a real
                        // success, also push a fresh Stopped update (reason
                        // "variable_edit") so the Variables panel and any
                        // open Watches both pick up the real new value
                        // through the exact same event path a normal stop
                        // already uses -- no second, parallel refresh
                        // mechanism needed.
                        let result =
                            set_variable_in_current_frame(&mut client, thread_id, &name, &value);
                        if result.is_ok() {
                            let stopped = describe_stop(&mut client, thread_id, "variable_edit");
                            if updates_tx.send(DapUpdate::Stopped(stopped)).is_err() {
                                let _ = reply.send(result);
                                break;
                            }
                        }
                        let _ = reply.send(result);
                        continue;
                    }
                };
                if resp.is_none() {
                    let _ = updates_tx.send(DapUpdate::Error("command request failed".to_string()));
                    continue;
                }
                let mut output_events = Vec::new();
                let outcome =
                    wait_for_stop_or_exit(&mut client, DEFAULT_TIMEOUT, &mut output_events);
                for ev in output_events {
                    let category = ev["body"]["category"]
                        .as_str()
                        .unwrap_or("console")
                        .to_string();
                    // A real, live-observed finding, not assumed: `debugpy`
                    // relays its own internal diagnostic pings ("ptvsd",
                    // "debugpy") as real `output` events with category
                    // `telemetry` -- pure adapter-implementation noise, of
                    // zero value to a user watching real logpoint/stdout
                    // output. Filtered here so it never reaches a caller.
                    if category == "telemetry" {
                        continue;
                    }
                    let text = ev["body"]["output"].as_str().unwrap_or("").to_string();
                    if updates_tx
                        .send(DapUpdate::Output { category, text })
                        .is_err()
                    {
                        break;
                    }
                }
                match outcome {
                    Some(("stopped", ev)) => {
                        thread_id = ev["body"]["threadId"].as_i64().unwrap_or(thread_id);
                        let reason = ev["body"]["reason"].as_str().unwrap_or("stopped");
                        let stopped = describe_stop(&mut client, thread_id, reason);
                        if updates_tx.send(DapUpdate::Stopped(stopped)).is_err() {
                            break;
                        }
                    }
                    Some(_) => {
                        let _ = updates_tx.send(DapUpdate::Exited);
                        break;
                    }
                    None => {
                        let _ = updates_tx.send(DapUpdate::Error(format!(
                            "no stop/exit event within {}s",
                            DEFAULT_TIMEOUT.as_secs()
                        )));
                    }
                }
            }

            client.shutdown();
        });

        Self {
            cmd_tx,
            updates_rx: Mutex::new(updates_rx),
        }
    }

    /// Non-blocking. Commands are never dropped or coalesced -- every one
    /// sent is executed, in order.
    pub fn send_command(&self, cmd: DapCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Blocks until the next real update (or the session ends). A
    /// background-thread consumer drives an event loop with this
    /// directly rather than polling, mirroring `spartan_lsp::LspSession::
    /// recv_update`.
    pub fn recv_update(&self) -> Option<DapUpdate> {
        self.updates_rx.lock().unwrap().recv().ok()
    }

    /// Evaluate an expression in the current top stack frame and block for
    /// the real result. Only meaningful while the session is stopped at a
    /// breakpoint; a caller that evaluates a running/ended session gets a
    /// real, honest error rather than a hang (bounded by a 10s reply
    /// timeout). Sends the command over the same ordered channel every
    /// other command uses, then waits on a one-shot reply channel.
    pub fn evaluate(&self, expression: &str) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        self.cmd_tx
            .send(DapCommand::Evaluate {
                expression: expression.to_string(),
                reply: tx,
            })
            .map_err(|_| "debug session is no longer running".to_string())?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| "no evaluate response (session may have ended)".to_string())?
    }

    /// Edit a variable's live value in the current top scope and block for
    /// the real result -- the same real, bounded-timeout, honest-error
    /// contract `evaluate` already established. On success, a fresh
    /// `Stopped` update is queued for `recv_update`'s own consumer before
    /// this call returns.
    pub fn set_variable(&self, name: &str, value: &str) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        self.cmd_tx
            .send(DapCommand::SetVariable {
                name: name.to_string(),
                value: value.to_string(),
                reply: tx,
            })
            .map_err(|_| "debug session is no longer running".to_string())?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| "no setVariable response (session may have ended)".to_string())?
    }
}

/// Fetches the current top stack frame and evaluates `expression` in it.
/// Returns the real DAP `body.result` display string, or an honest error
/// (no frame, adapter failure, or a real evaluation error the adapter
/// itself reports).
fn evaluate_in_current_frame(
    client: &mut DapClient,
    thread_id: i64,
    expression: &str,
) -> Result<String, String> {
    let frame_id = client
        .stack_trace(thread_id)
        .and_then(|f| {
            f["body"]["stackFrames"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|fr| fr["id"].as_i64())
        })
        .ok_or_else(|| "no active stack frame to evaluate in".to_string())?;
    match client.evaluate(expression, frame_id) {
        Some(resp) if resp["success"].as_bool() == Some(true) => {
            Ok(resp["body"]["result"].as_str().unwrap_or("").to_string())
        }
        Some(resp) => {
            // A real evaluation error (bad expression, name error, etc.) --
            // the adapter reports it in `message` or `body.result`.
            let msg = resp["message"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| resp["body"]["result"].as_str())
                .unwrap_or("evaluation failed")
                .to_string();
            Err(msg)
        }
        None => Err("no response from the debug adapter".to_string()),
    }
}

/// Real DAP `setVariable`, scoped to the current top frame's first real
/// scope -- the same "re-derive fresh from `thread_id` every call" shape
/// `evaluate_in_current_frame` already established (correct even if a
/// step happened between two edits, not just a one-time cached lookup).
/// A real, deliberate v1 scope limit: only edits a variable directly in
/// the top scope (locals), not a nested field of a compound value -- that
/// would need the *variable's own* `variablesReference` as the container,
/// which this crate's `DapVariable` doesn't carry yet (only `name`/
/// `value`). Returns the adapter's own real re-formatted value string on
/// success (which may differ from what was typed), or a real error
/// (no frame, no scope, or the adapter rejecting the edit -- e.g. a
/// read-only or type-mismatched value).
fn set_variable_in_current_frame(
    client: &mut DapClient,
    thread_id: i64,
    name: &str,
    value: &str,
) -> Result<String, String> {
    let frame_id = client
        .stack_trace(thread_id)
        .and_then(|f| {
            f["body"]["stackFrames"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|fr| fr["id"].as_i64())
        })
        .ok_or_else(|| "no active stack frame to set a variable in".to_string())?;
    let vars_ref = client
        .scopes(frame_id)
        .and_then(|s| {
            s["body"]["scopes"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|sc| sc["variablesReference"].as_i64())
        })
        .ok_or_else(|| "no active scope to set a variable in".to_string())?;
    match client.set_variable(vars_ref, name, value) {
        Some(resp) if resp["success"].as_bool() == Some(true) => {
            Ok(resp["body"]["value"].as_str().unwrap_or(value).to_string())
        }
        Some(resp) => {
            let msg = resp["message"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("setVariable failed")
                .to_string();
            Err(msg)
        }
        None => Err("no response from the debug adapter".to_string()),
    }
}

/// Waits for a real `stopped` or `exited` event, collecting every real
/// `output` event seen along the way into `output_sink` instead of
/// silently dropping it (the previous two-phase `wait_event("stopped",
/// ...)` then `wait_event("exited", ...)` implementation never issued a
/// matching wait for `output` at all, so any real output/logpoint event
/// that arrived was buffered and then never read back out by anything).
fn wait_for_stop_or_exit(
    client: &mut DapClient,
    timeout: Duration,
    output_sink: &mut Vec<Value>,
) -> Option<(&'static str, Value)> {
    let ev = client.wait_for_collecting_output(
        |m| {
            m.get("type").and_then(Value::as_str) == Some("event")
                && matches!(
                    m.get("event").and_then(Value::as_str),
                    Some("stopped") | Some("exited")
                )
        },
        timeout,
        output_sink,
    )?;
    if ev.get("event").and_then(Value::as_str) == Some("stopped") {
        Some(("stopped", ev))
    } else {
        Some(("exited", ev))
    }
}

fn describe_stop(client: &mut DapClient, thread_id: i64, reason: &str) -> DapStopped {
    let mut stopped = DapStopped {
        thread_id,
        reason: reason.to_string(),
        frame: None,
        variables: Vec::new(),
    };

    let Some(frames) = client.stack_trace(thread_id) else {
        return stopped;
    };
    let Some(frame) = frames["body"]["stackFrames"]
        .as_array()
        .and_then(|a| a.first())
    else {
        return stopped;
    };
    let frame_id = frame["id"].as_i64().unwrap_or(0);
    let frame_name = frame["name"].as_str().unwrap_or("<unknown>").to_string();
    let frame_line = frame["line"].as_i64().unwrap_or(0);
    stopped.frame = Some(DapFrame {
        name: frame_name,
        line: frame_line,
    });

    let Some(scopes) = client.scopes(frame_id) else {
        return stopped;
    };
    let Some(scope) = scopes["body"]["scopes"].as_array().and_then(|a| a.first()) else {
        return stopped;
    };
    let vars_ref = scope["variablesReference"].as_i64().unwrap_or(0);
    let Some(vars) = client.variables(vars_ref) else {
        return stopped;
    };
    if let Some(var_array) = vars["body"]["variables"].as_array() {
        for v in var_array {
            let name = v["name"].as_str().unwrap_or("?").to_string();
            let value = v["value"].as_str().unwrap_or("?").to_string();
            stopped.variables.push(DapVariable { name, value });
        }
    }
    stopped
}
