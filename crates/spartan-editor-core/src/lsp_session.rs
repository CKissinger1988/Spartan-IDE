//! The genuinely new engineering this pass adds: a real, live LSP session
//! for one open file, wired to the render loop without ever blocking it.
//! `lsp::LspClient`'s calls block synchronously, some (indexing) for up to
//! 90s -- calling them directly from `main.rs`'s winit event loop would
//! freeze the whole editor, so this module runs the entire session on its
//! own OS thread and exposes a non-blocking interface to the render thread.
//! See §75.6 and the crate README for the full design rationale.

use crate::lsp::{path_to_file_uri, same_file_uri, LspClient, DEFAULT_TIMEOUT, INDEXING_TIMEOUT};
use spartan_languages::CommandSpec;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

/// No diagnostics UI exists anywhere in this crate yet -- matching how
/// `language::detect_language_for_file`'s result is also just printed --
/// so this payload is display-ready text, not structured LSP data a future
/// UI would want.
pub enum LspUpdate {
    Diagnostics(Vec<String>),
}

struct MailboxState {
    latest_text: Option<String>,
    shutdown: bool,
}

/// A single-slot mailbox, not a channel: the render thread only ever cares
/// about the *most recent* edit, and a plain `mpsc` channel gives the
/// sender no way to evict a value it already queued. Without this, several
/// debounce firings that happen while the background thread is still stuck
/// in the initial (up to 90s) indexing wait would queue up and all get
/// dispatched in order once the thread is free -- stale snapshots the
/// server has to redundantly re-analyze before diagnostics catch up to
/// real time. Overwriting `latest_text` means only the newest snapshot
/// ever actually gets sent, however many debounce firings happened while
/// the background thread was busy.
struct Mailbox {
    state: Mutex<MailboxState>,
    cvar: Condvar,
}

/// A real, live LSP session for one open file, running on its own OS
/// thread for its entire lifetime -- spawn, the initialize/didOpen
/// handshake, and every subsequent didChange dispatch all happen there,
/// never on the caller's thread. `spawn()` itself only blocks long enough
/// to start the subprocess (near-instant); the protocol handshake and
/// indexing wait happen in the background so they never add to cold-open
/// time on the render thread.
pub struct LspSession {
    mailbox: Arc<Mailbox>,
    updates_rx: Receiver<LspUpdate>,
    thread: JoinHandle<()>,
}

impl LspSession {
    /// `initial_text` should be the real, current in-memory buffer content
    /// (not necessarily what's on disk) -- it becomes the `didOpen` payload.
    pub fn spawn(
        command: &CommandSpec,
        project_root: &Path,
        file_path: &Path,
        initial_text: &str,
    ) -> std::io::Result<Self> {
        let args: Vec<&str> = command.args.iter().map(String::as_str).collect();
        let mut client = LspClient::spawn_with_args(&command.program, &args)?;

        let root_uri = path_to_file_uri(project_root);
        let file_uri = path_to_file_uri(file_path);
        let initial_text = initial_text.to_string();

        let mailbox = Arc::new(Mailbox {
            state: Mutex::new(MailboxState {
                latest_text: None,
                shutdown: false,
            }),
            cvar: Condvar::new(),
        });
        let (updates_tx, updates_rx) = mpsc::channel();
        let thread_mailbox = Arc::clone(&mailbox);

        let thread = thread::spawn(move || {
            if client
                .open_project(&root_uri, &file_uri, &initial_text)
                .is_none()
            {
                let _ = updates_tx.send(LspUpdate::Diagnostics(vec![
                    "LSP initialize/didOpen sequence did not complete".to_string(),
                ]));
                return;
            }

            // First wait reuses the spike's own proven, tested cold-open
            // path: long indexing timeout, only resolves on a non-empty
            // diagnostics array. A clean file (or a timeout) both surface
            // as `None` here -- that ambiguity is real, not glossed over,
            // see the message below.
            match client.wait_real_diagnostics(&file_uri, INDEXING_TIMEOUT) {
                Some(diags) => {
                    let _ = updates_tx.send(LspUpdate::Diagnostics(format_diagnostics(&diags)));
                }
                None => {
                    let _ = updates_tx.send(LspUpdate::Diagnostics(vec![format!(
                        "no diagnostics within {}s -- file may be clean, or indexing may not have finished",
                        INDEXING_TIMEOUT.as_secs()
                    )]));
                }
            }

            run_dispatch_loop(&mut client, &file_uri, &thread_mailbox, &updates_tx);

            client.shutdown();
        });

        Ok(Self {
            mailbox,
            updates_rx,
            thread,
        })
    }

    /// Overwrites any not-yet-processed pending snapshot and wakes the
    /// background thread. Never blocks.
    pub fn notify_edit(&self, current_text: String) {
        let mut guard = self.mailbox.state.lock().unwrap();
        guard.latest_text = Some(current_text);
        self.mailbox.cvar.notify_one();
    }

    /// Drains every update currently available without blocking.
    pub fn poll_updates(&self) -> Vec<LspUpdate> {
        self.updates_rx.try_iter().collect()
    }

    /// Signals the background thread to shut down and joins it.
    /// `LspClient::shutdown()`'s own bounded waits can take up to ~7s
    /// worst case (5s shutdown-request timeout + 2s process-reap poll) --
    /// this prints a status line first so a caller on a UI thread (see
    /// `main.rs`'s `CloseRequested` handler) sees a known, bounded wait,
    /// not what looks like a silent hang.
    pub fn shutdown(self) {
        println!("Shutting down language server...");
        {
            let mut guard = self.mailbox.state.lock().unwrap();
            guard.shutdown = true;
        }
        self.mailbox.cvar.notify_one();
        let _ = self.thread.join();
    }
}

/// Waits for the next edit snapshot (or a shutdown signal), dispatches it,
/// and waits for the server's updated diagnostics -- repeating until
/// shutdown. Every dispatch here uses `wait_notification` directly instead
/// of `LspClient::wait_real_diagnostics`: the latter only ever resolves on
/// a *non-empty* diagnostics array by design (that's what the spike's own
/// tests need), so reusing it here verbatim would mean a fixed error could
/// never be reported as fixed -- the old diagnostics would stay printed
/// forever. `did_change_full` itself is the one piece of the spike's
/// "proven" client its own tests never actually exercised against a real
/// server; this loop is its first real exercise.
fn run_dispatch_loop(
    client: &mut LspClient,
    file_uri: &str,
    mailbox: &Mailbox,
    updates_tx: &mpsc::Sender<LspUpdate>,
) {
    let mut next_version: i64 = 2; // didOpen already used version 1
    loop {
        let text = {
            let mut guard = mailbox.state.lock().unwrap();
            loop {
                if guard.shutdown {
                    break None;
                }
                if let Some(text) = guard.latest_text.take() {
                    break Some(text);
                }
                guard = mailbox.cvar.wait(guard).unwrap();
            }
        };
        let Some(text) = text else {
            return;
        };

        if client
            .did_change_full(file_uri, next_version, &text)
            .is_err()
        {
            return;
        }
        next_version += 1;

        let file_uri_owned = file_uri.to_string();
        let diagnostics = client.wait_notification(
            "textDocument/publishDiagnostics",
            move |params| {
                params
                    .get("uri")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|u| same_file_uri(u, &file_uri_owned))
            },
            DEFAULT_TIMEOUT,
        );
        let formatted = match diagnostics {
            Some(msg) => {
                let diags = msg["params"]["diagnostics"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                format_diagnostics(&diags)
            }
            None => vec![format!(
                "no diagnostics update within {}s of last edit",
                DEFAULT_TIMEOUT.as_secs()
            )],
        };
        if updates_tx.send(LspUpdate::Diagnostics(formatted)).is_err() {
            return;
        }
    }
}

fn format_diagnostics(diags: &[serde_json::Value]) -> Vec<String> {
    if diags.is_empty() {
        return vec!["0 diagnostics -- clean".to_string()];
    }
    diags
        .iter()
        .map(|d| {
            let line = d["range"]["start"]["line"].as_i64().unwrap_or(-1);
            let message = d["message"].as_str().unwrap_or("<no message>");
            let severity_label = match d["severity"].as_i64().unwrap_or(0) {
                1 => "error",
                2 => "warning",
                3 => "info",
                4 => "hint",
                _ => "diagnostic",
            };
            format!("line {}: {severity_label} - {message}", line + 1)
        })
        .collect()
}
