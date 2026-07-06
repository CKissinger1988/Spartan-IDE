//! The genuinely new engineering this pass adds: a real, live DAP session
//! (launch, breakpoint hit, continue/step, stack/variable inspection) wired
//! to the render loop without ever blocking it. Deliberately different
//! threading design from `lsp_session.rs`'s mailbox: LSP coalesces edits
//! because only the *latest* snapshot matters, but every debug command the
//! user issues (continue, step-over, step-into) is a discrete, ordered
//! action that must execute exactly once, in order -- none may be dropped.
//! Uses a plain ordered `mpsc::channel` for commands instead. See §75.8 and
//! the crate README for the full design rationale, including why
//! breakpoints here are plain line numbers, not rope-anchored (an
//! explicitly sanctioned §39.2 fallback, not an oversight).

use crate::dap::{DapClient, DEFAULT_TIMEOUT};
use serde_json::Value;
use spartan_languages::CommandSpec;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub enum DapCommand {
    Continue,
    StepOver,
    StepInto,
}

/// No debug UI (breakpoint gutter, variables panel) exists anywhere in this
/// crate yet -- matching how LSP diagnostics are also just printed --  so
/// this payload is display-ready text.
pub enum DapUpdate {
    Stopped(Vec<String>),
    Exited,
    Error(String),
}

pub struct DapSession {
    cmd_tx: Sender<DapCommand>,
    updates_rx: Receiver<DapUpdate>,
    thread: JoinHandle<()>,
}

impl DapSession {
    /// Never blocks the caller: the subprocess spawn, the launch/breakpoint
    /// handshake, and every subsequent command all happen on a dedicated
    /// background thread, matching `LspSession::spawn`'s "handshake happens
    /// in the background" design.
    pub fn launch(
        adapter: &CommandSpec,
        binary_path: &Path,
        cwd: &Path,
        source_path: &Path,
        break_lines: &[i64],
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<DapCommand>();
        let (updates_tx, updates_rx) = mpsc::channel::<DapUpdate>();

        let adapter_program = adapter.program.clone();
        let binary_path = binary_path.to_string_lossy().to_string();
        let cwd = cwd.to_string_lossy().to_string();
        let source_path = source_path.to_string_lossy().to_string();
        let break_lines = break_lines.to_vec();

        let thread = thread::spawn(move || {
            let mut client = match DapClient::spawn(&adapter_program) {
                Ok(c) => c,
                Err(e) => {
                    let _ = updates_tx.send(DapUpdate::Error(format!(
                        "failed to spawn {adapter_program}: {e}"
                    )));
                    return;
                }
            };

            let mut thread_id =
                match client.launch_and_break(&binary_path, &cwd, &source_path, &break_lines) {
                    Some(result) => {
                        let tid = result["stopped"]["body"]["threadId"].as_i64().unwrap_or(0);
                        let reason = result["stopped"]["body"]["reason"]
                            .as_str()
                            .unwrap_or("breakpoint");
                        let lines = describe_stop(&mut client, tid, reason);
                        let _ = updates_tx.send(DapUpdate::Stopped(lines));
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
                };
                if resp.is_none() {
                    let _ = updates_tx.send(DapUpdate::Error("command request failed".to_string()));
                    continue;
                }
                // Safe against DAP's response/event interleaving because
                // `wait_for`'s own buffering (used throughout `dap.rs`)
                // re-queues any non-matching message for a later,
                // differently-named wait -- trying "stopped" first and
                // falling back to "exited" can't lose a message.
                match wait_for_stop_or_exit(&mut client, DEFAULT_TIMEOUT) {
                    Some(("stopped", ev)) => {
                        thread_id = ev["body"]["threadId"].as_i64().unwrap_or(thread_id);
                        let reason = ev["body"]["reason"].as_str().unwrap_or("stopped");
                        let lines = describe_stop(&mut client, thread_id, reason);
                        if updates_tx.send(DapUpdate::Stopped(lines)).is_err() {
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
            updates_rx,
            thread,
        }
    }

    /// Non-blocking. Commands are never dropped or coalesced -- every one
    /// sent is executed, in order, unlike `LspSession::notify_edit`'s
    /// deliberately-lossy mailbox.
    pub fn send_command(&self, cmd: DapCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Drains every update currently available without blocking.
    pub fn poll_updates(&self) -> Vec<DapUpdate> {
        self.updates_rx.try_iter().collect()
    }

    /// True once the background thread has exited -- the debuggee ran to a
    /// real exit, the launch/breakpoint sequence failed, or the command
    /// channel was disconnected. Callers (see `main.rs`'s `F5` handler)
    /// use this to tell a genuinely-over session apart from a live one,
    /// so a new `F5` press rebuilds/relaunches instead of silently trying
    /// to `Continue` a session nothing is listening on anymore.
    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// Dropping the sender closes the channel, which makes the background
    /// thread's blocking `cmd_rx.recv()` return `Err` and exit its loop
    /// naturally, at which point it calls `DapClient::shutdown()` itself.
    pub fn shutdown(self) {
        println!("Shutting down debug session...");
        drop(self.cmd_tx);
        let _ = self.thread.join();
    }
}

/// Tries "stopped" first (the common case after a command), falling back to
/// a short "exited" check -- a real program with nothing left to stop at
/// runs to completion instead.
fn wait_for_stop_or_exit(
    client: &mut DapClient,
    timeout: Duration,
) -> Option<(&'static str, Value)> {
    if let Some(ev) = client.wait_event("stopped", timeout) {
        return Some(("stopped", ev));
    }
    client
        .wait_event("exited", Duration::from_millis(500))
        .map(|ev| ("exited", ev))
}

/// Fetches the top stack frame and its local variables for a stopped
/// thread, formatting them for printing -- no debug UI exists to hand
/// structured data to yet.
fn describe_stop(client: &mut DapClient, thread_id: i64, reason: &str) -> Vec<String> {
    let mut lines = vec![format!("stopped ({reason}), thread {thread_id}")];

    let Some(frames) = client.stack_trace(thread_id) else {
        lines.push("  (no stack trace available)".to_string());
        return lines;
    };
    let Some(frame) = frames["body"]["stackFrames"]
        .as_array()
        .and_then(|a| a.first())
    else {
        lines.push("  (empty stack trace)".to_string());
        return lines;
    };
    let frame_id = frame["id"].as_i64().unwrap_or(0);
    let frame_name = frame["name"].as_str().unwrap_or("<unknown>");
    let frame_line = frame["line"].as_i64().unwrap_or(0);
    lines.push(format!("  at {frame_name} (line {frame_line})"));

    let Some(scopes) = client.scopes(frame_id) else {
        return lines;
    };
    let Some(scope) = scopes["body"]["scopes"].as_array().and_then(|a| a.first()) else {
        return lines;
    };
    let vars_ref = scope["variablesReference"].as_i64().unwrap_or(0);
    let Some(vars) = client.variables(vars_ref) else {
        return lines;
    };
    if let Some(var_array) = vars["body"]["variables"].as_array() {
        for v in var_array {
            let name = v["name"].as_str().unwrap_or("?");
            let value = v["value"].as_str().unwrap_or("?");
            lines.push(format!("  {name} = {value}"));
        }
    }
    lines
}
