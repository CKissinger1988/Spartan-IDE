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
use crate::client::{DapClient, DEFAULT_TIMEOUT};
use serde::Serialize;
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

pub enum DapUpdate {
    /// A real Cargo (or other configured build system) compile error --
    /// reported before any debug adapter is even spawned.
    BuildFailed(Vec<String>),
    Stopped(DapStopped),
    Exited,
    Error(String),
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
        break_lines: &[i64],
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<DapCommand>();
        let (updates_tx, updates_rx) = mpsc::channel::<DapUpdate>();

        let adapter_program = adapter.program.clone();
        let adapter_args = adapter.args.clone();
        let project_root = project_root.to_path_buf();
        let mut program_path = program_path.to_path_buf();
        let cwd = cwd.to_string_lossy().to_string();
        let source_path = source_path.to_string_lossy().to_string();
        let break_lines = break_lines.to_vec();

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
                match client.launch_and_break(&program, &cwd, &source_path, &break_lines) {
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
                };
                if resp.is_none() {
                    let _ = updates_tx.send(DapUpdate::Error("command request failed".to_string()));
                    continue;
                }
                match wait_for_stop_or_exit(&mut client, DEFAULT_TIMEOUT) {
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
}

fn wait_for_stop_or_exit(
    client: &mut DapClient,
    timeout: Duration,
) -> Option<(&'static str, serde_json::Value)> {
    if let Some(ev) = client.wait_event("stopped", timeout) {
        return Some(("stopped", ev));
    }
    client
        .wait_event("exited", Duration::from_millis(500))
        .map(|ev| ("exited", ev))
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
