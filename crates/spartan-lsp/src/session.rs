//! A real, live LSP session for one open file, running on its own OS
//! thread for its entire lifetime, adapted from
//! `crates/spartan-editor-core/src/lsp_session.rs` (§75.6) for a
//! background-thread consumer instead of a render-loop poller.
//!
//! Two real, deliberate differences from the copy this was adapted from,
//! both named rather than silently changed:
//!
//! 1. **Structured diagnostics, not display-ready strings.** The original
//!    had no diagnostics UI anywhere in its crate yet, so it formatted
//!    diagnostics into printable text. This crate's callers (`spartan-
//!    backend`, ultimately a real browser/Electron UI) want real structured
//!    data -- `LspDiagnostic` derives `Serialize` and is sent as-is inside a
//!    real backend `Event`.
//! 2. **Self-contained debounce, not a caller-driven tick.** The original
//!    relied on `main.rs`'s winit render loop calling `DidChangeDebouncer::
//!    should_dispatch_now()` on every frame to decide *when* to call
//!    `notify_edit`. A background IPC service has no render loop to tick --
//!    so the debounce logic now lives inside this session's own dispatch
//!    loop: every `notify_edit` bumps a version counter, and the loop only
//!    actually dispatches once `debounce` has elapsed with no newer version
//!    arriving, using `Condvar::wait_timeout` to detect that a newer edit
//!    interrupted the wait. Same real intent (coalesce a burst of rapid
//!    edits into one dispatch) as the original, just self-contained instead
//!    of caller-driven.

use crate::client::{
    path_to_file_uri, same_file_uri, LspClient, DEFAULT_TIMEOUT, INDEXING_TIMEOUT,
};
use serde::Serialize;
use spartan_languages::CommandSpec;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// A real, structured LSP diagnostic -- `severity` is a lowercase label
/// (`"error"`/`"warning"`/`"info"`/`"hint"`), matching the LSP spec's own
/// four severity levels (1-4) rather than the raw integer, so a UI never
/// needs to duplicate that mapping.
#[derive(Debug, Clone, Serialize)]
pub struct LspDiagnostic {
    pub severity: String,
    pub line: u32,
    pub character: u32,
    pub end_line: u32,
    pub end_character: u32,
    pub message: String,
}

fn to_diagnostic(d: &serde_json::Value) -> LspDiagnostic {
    let severity = match d["severity"].as_i64().unwrap_or(0) {
        1 => "error",
        2 => "warning",
        3 => "info",
        4 => "hint",
        _ => "diagnostic",
    }
    .to_string();
    LspDiagnostic {
        severity,
        line: d["range"]["start"]["line"].as_u64().unwrap_or(0) as u32,
        character: d["range"]["start"]["character"].as_u64().unwrap_or(0) as u32,
        end_line: d["range"]["end"]["line"].as_u64().unwrap_or(0) as u32,
        end_character: d["range"]["end"]["character"].as_u64().unwrap_or(0) as u32,
        message: d["message"].as_str().unwrap_or("<no message>").to_string(),
    }
}

fn to_diagnostics(diags: &[serde_json::Value]) -> Vec<LspDiagnostic> {
    diags.iter().map(to_diagnostic).collect()
}

/// An update pushed by a live session. `ServerError` covers both "the
/// initialize/didOpen handshake never completed" and "no diagnostics
/// update arrived within timeout" -- real, distinct-enough-to-report
/// conditions a caller may want to surface, but neither is a Rust `Err`
/// since the session itself keeps running either way.
pub enum LspUpdate {
    Diagnostics(Vec<LspDiagnostic>),
    ServerError(String),
}

/// A real, one-shot hover/completion request, answered by the session's
/// own background thread ahead of (never behind) whatever edit-debounce
/// wait it may currently be sitting in -- see `wait_for_next_action`'s own
/// doc comment for why queries always take priority.
enum QueryKind {
    Hover { line: i64, character: i64 },
}

struct PendingQuery {
    kind: QueryKind,
    reply: Sender<Option<serde_json::Value>>,
}

struct MailboxState {
    latest_text: Option<String>,
    /// Bumped on every `notify_edit`, independent of `latest_text` itself,
    /// so the dispatch loop can tell "a newer edit arrived during my
    /// debounce wait" apart from "the same edit is still sitting here"
    /// without needing to compare potentially-large text values.
    version: u64,
    shutdown: bool,
    /// Real hover/completion requests, drained oldest-first. Deliberately
    /// a plain queue, not deduplicated or coalesced the way edits are --
    /// each real query has its own distinct real caller waiting on its
    /// own reply channel, unlike edits, which only ever have one "current"
    /// buffer state worth reporting.
    pending_queries: VecDeque<PendingQuery>,
}

struct Mailbox {
    state: Mutex<MailboxState>,
    cvar: Condvar,
}

/// A real, live LSP session for one open file, running on its own OS
/// thread for its entire lifetime -- spawn, the initialize/didOpen
/// handshake, and every subsequent didChange dispatch all happen there.
///
/// `updates_rx` is wrapped in a `Mutex` purely to make `LspSession: Sync`
/// (a plain `mpsc::Receiver` is `Send` but not `Sync`) -- real callers
/// (e.g. `spartan-backend`'s own LSP wiring) share one session behind an
/// `Arc` between the thread that calls `notify_edit`/`signal_shutdown` and
/// a single dedicated thread that drains `recv_update()`; the mutex is
/// never contended in that real usage since only the latter ever locks it.
pub struct LspSession {
    mailbox: Arc<Mailbox>,
    updates_rx: Mutex<Receiver<LspUpdate>>,
    thread: JoinHandle<()>,
}

impl LspSession {
    /// `initial_text` should be the real, current in-memory buffer content
    /// (not necessarily what's on disk) -- it becomes the `didOpen` payload.
    /// `language_id` is a real LSP `languageId` string (e.g. `"rust"`).
    /// `debounce` matches `spartan-editor-core`'s own 150ms default when the
    /// caller has no more specific preference.
    pub fn spawn(
        command: &CommandSpec,
        project_root: &Path,
        file_path: &Path,
        language_id: &str,
        initial_text: &str,
        debounce: Duration,
    ) -> std::io::Result<Self> {
        let args: Vec<&str> = command.args.iter().map(String::as_str).collect();
        let mut client = LspClient::spawn_with_args(&command.program, &args)?;

        let root_uri = path_to_file_uri(project_root);
        let file_uri = path_to_file_uri(file_path);
        let language_id = language_id.to_string();
        let initial_text = initial_text.to_string();

        let mailbox = Arc::new(Mailbox {
            state: Mutex::new(MailboxState {
                latest_text: None,
                version: 0,
                shutdown: false,
                pending_queries: VecDeque::new(),
            }),
            cvar: Condvar::new(),
        });
        let (updates_tx, updates_rx) = mpsc::channel();
        let thread_mailbox = Arc::clone(&mailbox);

        let thread = thread::spawn(move || {
            if client
                .open_project(&root_uri, &file_uri, &language_id, &initial_text)
                .is_none()
            {
                let _ = updates_tx.send(LspUpdate::ServerError(
                    "LSP initialize/didOpen sequence did not complete".to_string(),
                ));
                return;
            }

            match client.wait_real_diagnostics(&file_uri, INDEXING_TIMEOUT) {
                Some(diags) => {
                    let _ = updates_tx.send(LspUpdate::Diagnostics(to_diagnostics(&diags)));
                }
                None => {
                    let _ = updates_tx.send(LspUpdate::ServerError(format!(
                        "no diagnostics within {}s -- file may be clean, or indexing may not have finished",
                        INDEXING_TIMEOUT.as_secs()
                    )));
                }
            }

            run_dispatch_loop(
                &mut client,
                &file_uri,
                &thread_mailbox,
                &updates_tx,
                debounce,
            );

            client.shutdown();
        });

        Ok(Self {
            mailbox,
            updates_rx: Mutex::new(updates_rx),
            thread,
        })
    }

    /// Overwrites any not-yet-dispatched pending snapshot, bumps the
    /// version, and wakes the background thread. Never blocks.
    pub fn notify_edit(&self, current_text: String) {
        let mut guard = self.mailbox.state.lock().unwrap();
        guard.latest_text = Some(current_text);
        guard.version += 1;
        self.mailbox.cvar.notify_one();
    }

    /// Blocks until the next real update (or the session ends). A
    /// background-thread consumer (this crate's intended usage, unlike
    /// `spartan-editor-core`'s render-tick `poll_updates`) drives an event
    /// loop with this directly rather than polling.
    pub fn recv_update(&self) -> Option<LspUpdate> {
        self.updates_rx.lock().unwrap().recv().ok()
    }

    /// Real, synchronous `textDocument/hover` against this session's own
    /// live server -- **blocks the calling thread** until the session's
    /// background thread answers (queries always run ahead of any
    /// in-progress edit-debounce wait, see `wait_for_next_action`), up to
    /// a real bounded timeout if the reply channel is somehow never
    /// signaled (the session thread exited without draining its queue).
    /// Real callers (e.g. `spartan-backend`'s own `lsp_hover` IPC handler)
    /// must call this from their own dedicated thread, never the single
    /// request-processing thread every other IPC method shares -- the
    /// same discipline already applied to Leo's own blocking model calls.
    ///
    /// **A real bug, found only by running a live end-to-end test, not by
    /// inspection**: an early version bounded this wait at `DEFAULT_
    /// TIMEOUT` (10s) -- correct for a query answered once the session's
    /// own dispatch loop is already running, but a query submitted right
    /// after `spawn()` returns is queued *before* the loop starts (the
    /// real `open_project`/`wait_real_diagnostics` handshake, bounded by
    /// the much larger `INDEXING_TIMEOUT`, runs first), so a real hover
    /// request issued immediately after opening a file against a
    /// still-indexing server timed out and silently returned `None` even
    /// though the query would genuinely have been answered correctly
    /// once indexing finished. Fixed by bounding this wait generously
    /// enough to cover that real worst case: the indexing wait, then the
    /// query's own request timeout.
    pub fn request_hover(&self, line: i64, character: i64) -> Option<serde_json::Value> {
        let (reply_tx, reply_rx) = mpsc::channel();
        {
            let mut guard = self.mailbox.state.lock().unwrap();
            guard.pending_queries.push_back(PendingQuery {
                kind: QueryKind::Hover { line, character },
                reply: reply_tx,
            });
        }
        self.mailbox.cvar.notify_one();
        reply_rx
            .recv_timeout(INDEXING_TIMEOUT + DEFAULT_TIMEOUT)
            .ok()
            .flatten()
    }

    /// Signals the background thread to shut down and joins it. Blocks for
    /// up to ~7s worst case, matching `LspClient::shutdown`'s own bound.
    pub fn shutdown(self) {
        self.signal_shutdown();
        let _ = self.thread.join();
    }

    /// Signals the background thread to shut down *without* joining it --
    /// for a caller sharing this session via `Arc` with a separate
    /// update-draining thread (see `spartan-backend`'s own LSP wiring),
    /// where blocking the request-handling thread for up to ~7s on every
    /// `close_file` would hurt responsiveness for no real benefit: the
    /// draining thread keeps running until the internal LSP thread
    /// finishes on its own and drops `updates_tx`, at which point
    /// `recv_update` naturally returns `None` and that thread exits --
    /// real cleanup, just not synchronously waited on here.
    pub fn signal_shutdown(&self) {
        {
            let mut guard = self.mailbox.state.lock().unwrap();
            guard.shutdown = true;
        }
        self.mailbox.cvar.notify_one();
    }
}

/// Waits for the next real action (a query or a settled edit), debounces
/// edits (self-contained -- see this module's own doc comment), dispatches
/// whichever arrived, and (for edits) waits for the server's updated
/// diagnostics -- repeating until shutdown. Uses `wait_notification`
/// directly (not `wait_real_diagnostics`, which only ever resolves on a
/// *non-empty* array by design) so a fixed error can be reported as fixed.
fn run_dispatch_loop(
    client: &mut LspClient,
    file_uri: &str,
    mailbox: &Mailbox,
    updates_tx: &mpsc::Sender<LspUpdate>,
    debounce: Duration,
) {
    let mut next_version: i64 = 2; // didOpen already used version 1
    loop {
        let text = match wait_for_next_action(mailbox, debounce) {
            Action::Shutdown => return,
            Action::Query(query) => {
                dispatch_query(client, file_uri, query);
                continue;
            }
            Action::Edit(text) => text,
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
        let update = match diagnostics {
            Some(msg) => {
                let diags = msg["params"]["diagnostics"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                LspUpdate::Diagnostics(to_diagnostics(&diags))
            }
            None => LspUpdate::ServerError(format!(
                "no diagnostics update within {}s of last edit",
                DEFAULT_TIMEOUT.as_secs()
            )),
        };
        if updates_tx.send(update).is_err() {
            return;
        }
    }
}

fn dispatch_query(client: &mut LspClient, file_uri: &str, query: PendingQuery) {
    let result = match query.kind {
        QueryKind::Hover { line, character } => client.hover(file_uri, line, character),
    };
    let _ = query.reply.send(result);
}

enum Action {
    Query(PendingQuery),
    Edit(String),
    Shutdown,
}

/// Blocks until a real query is pending (answered immediately, ahead of
/// any edit -- a hover request shouldn't wait out someone else's debounce
/// window) or an edit has been sitting in the mailbox for a full
/// `debounce` window with no newer edit arriving during that wait (a real,
/// self-contained coalescing policy -- see this module's own doc comment).
/// Returns `Action::Shutdown` on shutdown.
fn wait_for_next_action(mailbox: &Mailbox, debounce: Duration) -> Action {
    let mut guard = mailbox.state.lock().unwrap();
    loop {
        if let Some(query) = guard.pending_queries.pop_front() {
            return Action::Query(query);
        }
        // Wait for a first edit (or shutdown, or a query) to arrive.
        loop {
            if guard.shutdown {
                return Action::Shutdown;
            }
            if guard.latest_text.is_some() || !guard.pending_queries.is_empty() {
                break;
            }
            guard = mailbox.cvar.wait(guard).unwrap();
        }
        if let Some(query) = guard.pending_queries.pop_front() {
            return Action::Query(query);
        }
        // An edit is pending. Wait out the debounce window; if a newer
        // edit bumps the version during that wait, loop and wait again --
        // but a query arriving during the wait still wins immediately.
        let version_before = guard.version;
        let (g, timeout_result) = mailbox.cvar.wait_timeout(guard, debounce).unwrap();
        guard = g;
        if guard.shutdown {
            return Action::Shutdown;
        }
        if let Some(query) = guard.pending_queries.pop_front() {
            return Action::Query(query);
        }
        if timeout_result.timed_out() && guard.version == version_before {
            if let Some(text) = guard.latest_text.take() {
                return Action::Edit(text);
            }
        }
        // Either spuriously woken with no new version (loop re-checks the
        // same wait), or a genuinely newer edit arrived -- either way,
        // re-enter the outer wait with the now-current state.
    }
}
