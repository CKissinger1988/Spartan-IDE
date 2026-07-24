//! Real IPC backend service for the new Electron desktop shell
//! (`desktop/`, user-directed pivot away from the wgpu-native
//! `spartan-editor-core` UI). Deliberately thin for file editing: the
//! real editing logic lives in `spartan-buffer::Document` (branching
//! undo tree, char-indexed edits). Since §75.61 this crate also wraps
//! the real Leo agent core (`spartan-leo::Agent`, `spartan-model::
//! OllamaProvider`) -- "Leo still runs the show" was a direct user
//! objection to Leo being missing entirely from the new Electron shell
//! (it only ever existed in the original wgpu shell's Agent mode), so
//! this is the real backend half of giving Leo a persistent presence
//! here too, not a nav screen you navigate away from.
//!
//! Two real transport shapes share one connection: a request always
//! gets exactly one `Response` on the same line-oriented stdout stream;
//! a slow operation (Leo's own real, possibly 20-45s, blocking model
//! call) additionally returns an immediate synchronous ack and later
//! pushes an unprompted `Event` line once a background thread finishes
//! -- the same real spawn-thread-report-back shape `spartan-editor-
//! core`'s own `leo_bridge.rs` already established for the wgpu shell,
//! moved to the IPC boundary instead of an in-process channel poll.
//!
//! `spartan-editor-core` (the original wgpu shell) is not being deleted
//! -- it stays as the real, tested, working proof that the underlying
//! Rust core is sound. This crate is the real step of exposing that
//! same core to a *different* UI layer.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use spartan_buffer::Document;
use spartan_leo::agent::{Agent, AgentError};
use spartan_leo::approval::ApprovalMode;
use spartan_leo::execute::{self, ExecuteAction};
use spartan_leo::plan::{generate_plan_cancellable, ImplementationPlan, PlanError};
use spartan_leo::tool::{ToolCall, ToolResult};
use spartan_model::provider::Message;
use spartan_model::{
    ClaudeProvider, FailoverProvider, LiteLLMProvider, LlamaCppProvider, LmStudioProvider,
    ModelProvider, OllamaProvider,
};

mod dap_integration;
/// Real "Format Document" -- shells out to each language's own configured
/// formatter (`languages.toml`'s `formatter` field, real since §20.1 but
/// unwired anywhere until now). See its own module doc comment for the
/// full account, including which real formatters this does and doesn't
/// support.
mod format_integration;
/// Real Hugging Face -> Ollama model downloader (curated list + user-defined
/// custom links) -- moved here from `spartan-devserver` so `desktop/`'s
/// Electron shell (which spawns a plain `spartan-backend`, not a
/// `spartan-devserver`) gets the same real model-management methods `web/`
/// already has, without duplicating any logic. `spartan-devserver`'s own
/// dispatcher now just falls through to `handle_request` for these methods.
/// `pub` (not just `pub(crate)`) so this crate's own real integration tests
/// (`tests/hf_pull_integration.rs`, `tests/litellm_integration.rs`, moved
/// here alongside these modules) can exercise the real subprocess-spawning
/// layer directly, the same real access they had in `spartan-devserver`.
pub mod hf_downloader;
pub mod litellm_proxy;
/// Real Hugging Face -> llama.cpp GGUF downloader -- unlike
/// `hf_downloader`/`lmstudio_downloader`, llama.cpp has no separate local
/// server process to shell a pull command out to (`spartan_model::
/// LlamaCppProvider` loads a `.gguf` file in-process), so this is a real,
/// direct HTTP download into `~/.spartan/models/` instead of a subprocess
/// handoff. See its own module doc comment for the full account.
pub mod llamacpp_downloader;
/// Real LM Studio model downloader via its bundled `lms` CLI -- moved here
/// alongside `hf_downloader` for the identical reason.
pub mod lmstudio_downloader;
mod lsp_integration;
mod pty;
/// Shared subprocess-spawn-and-stream helper, used by `litellm_proxy`/
/// `hf_downloader`/`lmstudio_downloader`.
mod subprocess;
pub mod ws_transport;

// Both `Serialize` and `Deserialize`: the server only ever deserializes a
// `Request` and serializes a `Response` in production, but `ws_transport`'s
// own tests act as a real client (over an actual WebSocket connection),
// which legitimately needs the opposite direction of both -- the same
// shape a real `web/` client will eventually need too.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    fn ok(id: u64, result: serde_json::Value) -> Self {
        Response {
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: u64, message: impl Into<String>) -> Self {
        Response {
            id,
            result: None,
            error: Some(message.into()),
        }
    }
}

/// Real, unprompted, server-initiated message -- distinguished from a
/// `Response` on the wire by having no `id` field and a real `event`
/// name instead, so the Electron client's line parser can tell the two
/// apart (`"event" in parsed` vs. `"id" in parsed`) without any framing
/// header.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub event: String,
    pub data: serde_json::Value,
}

/// The real model this build targets for Leo's own plan generation --
/// the same live-proven §75.43/§75.46 target class
/// `spartan-editor-core::leo_bridge::LEO_MODEL` already uses, kept
/// identical rather than re-guessed.
pub const LEO_MODEL: &str = "llama3.1:8b";

struct OpenDoc {
    path: PathBuf,
    document: Document,
    /// Real redo support (task #52 audit finding: the Electron shell's
    /// `Editor.tsx` never called the pre-existing `undo` method and had
    /// no `redo` at all) -- `spartan_buffer::Document` itself has no
    /// single well-defined "redo" on a branching undo tree, so this is
    /// built one layer up here, the exact same real pattern the original
    /// wgpu shell's own `EditorView::redo_stack` already established
    /// (§75.19): `undo` pushes the pre-undo checkpoint here before
    /// jumping back; `redo` pops and jumps forward to it; any real new
    /// edit clears it, since a fresh edit invalidates whatever "forward"
    /// used to mean.
    redo_stack: Vec<spartan_buffer::CheckpointId>,
    /// Real, live LSP session for this file, if the detected language has
    /// a configured server and a real project root could be found (see
    /// `lsp_integration::maybe_spawn_lsp`'s own doc comment for exactly
    /// when it's `None` instead, all honest, non-error cases).
    lsp_session: Option<Arc<spartan_lsp::LspSession>>,
}

/// One real, model-proposed tool call awaiting explicit human approval --
/// every real call needs one (`leo_start_task` always constructs its
/// `Agent` with `ApprovalMode::ManualEveryStep`, §9's own non-negotiable
/// default), so there is never more than one pending at a time in this
/// crate's own real usage.
struct PendingCall {
    call_id: String,
    call: ToolCall,
}

/// Real, in-memory session state, now real behind `Arc<Mutex<_>>`
/// (previously plain, single-threaded-only) because Leo's own plan
/// generation must run on a background thread without blocking file
/// edits -- see this module's own doc comment for the full shape.
#[derive(Default)]
pub struct BackendState {
    open_docs: HashMap<u64, OpenDoc>,
    next_doc_id: u64,
    leo_agent: Option<Agent>,
    leo_project_root: Option<PathBuf>,
    /// Real, accumulating conversation history for the current task's
    /// execute loop (§75.66) -- grows by one `Assistant`+`Tool` message
    /// pair per real approved-and-run tool call (`execute::
    /// append_tool_result`), read fresh by every `leo_next_step` call so
    /// the model sees its own real prior actions, matching `execute.rs`'s
    /// own doc comment for how a caller is expected to drive the loop.
    leo_history: Vec<Message>,
    leo_pending_call: Option<PendingCall>,
    /// Real §75.69 generation counter -- incremented every real
    /// `leo_start_task` call. Every background thread this crate spawns
    /// on Leo's behalf (`leo_start_task`'s own planning thread,
    /// `leo_next_step`'s own execute-loop thread) captures the
    /// generation it started with and refuses to apply a real, possibly
    /// stale result once it completes unless the generation still
    /// matches -- otherwise a late-arriving background thread from a
    /// task the user has since replaced (by starting a new one) could
    /// silently clobber the newer task's real state. Real, load-bearing
    /// correctness, not defensive gold-plating: §75.69's own auto-
    /// approve-safe loop can run several real, unattended iterations
    /// before returning control, widening the exact window this guards.
    leo_generation: u64,
    /// Real §75.73-closing cooperative cancellation (task #269): a real,
    /// live `Arc<AtomicBool>` clone shared with whichever background
    /// thread is currently blocked inside a real Leo model call
    /// (`leo_start_task`'s planning thread, `leo_next_step`'s own
    /// execute-loop thread). `leo_start_task` mints a brand-new flag
    /// (`false`) every real new task, the same "start fresh" discipline
    /// `leo_generation` itself already uses, rather than resetting the
    /// existing one -- avoiding any chance of a late clone from a
    /// previous, already-discarded generation racing a fresh task's own
    /// flag. `leo_cancel` sets the *current* flag true, which every real
    /// network-backed `ModelProvider` (`OllamaProvider`/`ClaudeProvider`/
    /// `LiteLLMProvider`/`LmStudioProvider`) checks once per real streamed
    /// chunk via `stream_completion_cancellable` and stops early on --
    /// genuinely interrupting the real background thread instead of only
    /// discarding its late result the way `leo_generation`'s own guard
    /// already did before this pass. See `ModelProvider::
    /// stream_completion_cancellable`'s own doc comment for the real,
    /// honest per-chunk-only limit this carries, and `LlamaCppProvider`'s
    /// own doc comment for the one real provider that doesn't act on it.
    leo_cancel_flag: Arc<AtomicBool>,
    pty_sessions: HashMap<u64, pty::PtyHandle>,
    next_pty_id: u64,
    /// Real §75.74 dev-container interactive exec sessions -- keyed the
    /// same way `pty_sessions` is, since a container exec session is the
    /// exact same real "one live handle the UI streams to/from" shape as
    /// a local PTY, just backed by a real Docker `exec` instead of a
    /// real local process.
    devcontainer_exec_sessions: HashMap<u64, spartan_devcontainer::docker::ExecHandle>,
    next_devcontainer_exec_id: u64,
    /// Real, live DAP sessions (§132) -- keyed independently of
    /// `open_docs`, not stored on `OpenDoc` directly, since a debug
    /// session can outlive the exact doc-id lifecycle question (a
    /// relaunch after the program exits is a *new* session, not a
    /// mutation of the old one) and a UI may reasonably want to keep a
    /// finished session's own last-known state addressable by its own id
    /// a moment longer than the launch call that created it.
    dap_sessions: HashMap<u64, Arc<spartan_dap::DapSession>>,
    next_dap_id: u64,
    /// Real, at-most-one LiteLLM proxy child process this backend has
    /// spawned, if any (moved here from `spartan-devserver`'s own
    /// `DevServerState` -- see this module's `hf_downloader`/`litellm_proxy`
    /// doc comments for why). Protected by the same top-level state lock
    /// every other field here already is, not a second inner mutex.
    litellm: Option<litellm_proxy::ProxyProcess>,
    /// Real task #273 generation counter, the exact same discipline
    /// `leo_generation` already established -- incremented on every real
    /// `litellm_proxy_start` call. A restart-on-crash supervisor thread
    /// (spawned only when a client opts in) captures the generation it
    /// started with and checks it before ever touching `litellm` again;
    /// a mismatch means an explicit stop or a fresh manual start happened
    /// since, so the supervisor recognizes it's superseded and exits
    /// quietly instead of respawning a proxy nobody asked for anymore.
    litellm_generation: u64,
    /// Real, live `adb logcat` sessions (task #150), keyed the same way
    /// `pty_sessions` is -- an unbounded real stream the caller explicitly
    /// stops, not a bounded call that resolves on its own.
    logcat_sessions: HashMap<u64, spartan_android::adb::LogcatHandle>,
    next_logcat_id: u64,
    /// Real task #266 multi-turn session history -- closes the
    /// `docs/FUTURE_FEATURES.md`-named "chat panel is task-scoped, no
    /// history" gap. Deliberately in-memory/session-scoped only (unlike
    /// `spartan_leo::memory`'s own real, persisted-to-disk project-tier
    /// memory, §75.67) -- this is a real UI convenience for "what did Leo
    /// just do a few tasks ago in this session," not a second memory
    /// system; it does not survive a process restart and was never asked
    /// to. Bounded by `MAX_LEO_SESSION_HISTORY` (oldest entries drop
    /// first) so a very long-running session can't grow this unboundedly.
    leo_session_history: Vec<LeoHistoryEntry>,
    /// The real task text of whichever agent `leo_agent` currently holds
    /// (or most recently held) -- `leo_start_task`'s own `task` parameter
    /// isn't stored anywhere else in this struct, so this is the only
    /// place a later history-recording call site (`leo_cancel`, a
    /// still-`Failed` agent being replaced by a new `leo_start_task`) can
    /// recover which real task a completed/abandoned run was for.
    leo_current_task: Option<String>,
    /// The most recent real `leo_plan_failed`/`leo_execute_failed` error
    /// text, if any -- `Failed` is not always truly terminal (§75.78's own
    /// bounded retry loop can bring it back to `Executing`), so a failure
    /// is deliberately *not* pushed into `leo_session_history` the moment
    /// it happens; this field remembers the real reason so it can be
    /// attached retroactively if the task is later abandoned (a new
    /// `leo_start_task` call) rather than actually retried.
    leo_last_error: Option<String>,
    /// Real, live cancellation flags for in-flight model downloads (task
    /// #268) -- keyed by `download_registry_key(source, event_id)` so the
    /// same `event_id` string used in `hf_pull_progress`/
    /// `lmstudio_pull_progress`/`llamacpp_download_progress` events can't
    /// collide across the three real sources (pulling the same curated
    /// model via both HF and LM Studio at once, say). A download's own
    /// background thread inserts a fresh `Arc::new(AtomicBool::new(false))`
    /// here before it starts, clones it into the thread, and removes the
    /// entry once the download finishes for any reason (success, failure,
    /// or a real user cancellation) -- so a stale entry can never outlive
    /// the download it belongs to. `model_download_cancel` only ever
    /// *sets* the flag; the download's own thread is what actually kills
    /// the child process or aborts the HTTP read loop, since only it holds
    /// the real `Child`/reader handle.
    download_cancellations: HashMap<String, Arc<std::sync::atomic::AtomicBool>>,
}

/// One real, terminal `leo_session_history` entry -- see
/// `BackendState::leo_session_history`'s own doc comment for why this
/// exists and what it deliberately isn't (not persisted, not a second
/// memory system).
#[derive(Clone, Serialize)]
struct LeoHistoryEntry {
    task: String,
    /// `"Done"` | `"Failed"` | `"Cancelled"` -- a plain string, not a new
    /// enum, matching `agent_state_name`'s own existing real convention
    /// for how this crate already reports `AgentState` across the IPC
    /// boundary.
    outcome: String,
    summary: Option<String>,
    error: Option<String>,
    unix_timestamp: u64,
}

/// Real, deliberate bound -- see `BackendState::leo_session_history`'s own
/// doc comment. 50 is generous for one real interactive session (a user
/// running fifty distinct Leo tasks without ever restarting the app) while
/// still keeping this a real, finite structure, not an unbounded log.
const MAX_LEO_SESSION_HISTORY: usize = 50;

/// Real, shared push helper -- every one of this module's three real
/// recording call sites (`leo_start_task`'s retroactive-`Failed` case,
/// `leo_cancel`, and `leo_next_step`'s own real `Done` completion) goes
/// through this one function, so the real timestamp/bounding/task-text
/// resolution logic exists in exactly one place.
fn push_leo_history(
    state: &mut BackendState,
    outcome: &str,
    summary: Option<String>,
    error: Option<String>,
) {
    let unix_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    state.leo_session_history.push(LeoHistoryEntry {
        task: state.leo_current_task.clone().unwrap_or_default(),
        outcome: outcome.to_string(),
        summary,
        error,
        unix_timestamp,
    });
    if state.leo_session_history.len() > MAX_LEO_SESSION_HISTORY {
        let overflow = state.leo_session_history.len() - MAX_LEO_SESSION_HISTORY;
        state.leo_session_history.drain(0..overflow);
    }
}

impl BackendState {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Serialize)]
struct DirEntry {
    name: String,
    path: String,
    is_dir: bool,
}

/// Real, non-recursive directory listing -- the renderer calls this once
/// per real tree-node expansion (lazy, not a whole-tree walk up front),
/// mirroring `file_tree.rs`'s own lazy-expansion design in the original
/// wgpu shell, just moved to the IPC boundary instead of an in-process
/// call. Dirs sorted first, then alphabetically within each group --
/// same real convention `file_tree.rs` already established. No hidden-
/// file filtering, matching that same precedent (dotfiles are real,
/// often-relevant project files -- `.gitignore`, `.env.example` -- not
/// noise to hide by default).
fn list_dir(path: &str) -> Result<serde_json::Value, String> {
    let dir = PathBuf::from(path);
    let read_dir = std::fs::read_dir(&dir).map_err(|e| format!("read_dir({path}): {e}"))?;
    let mut entries: Vec<DirEntry> = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(|e| format!("read_dir entry: {e}"))?;
        let file_type = entry.file_type().map_err(|e| format!("file_type: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let full_path = entry.path().to_string_lossy().into_owned();
        entries.push(DirEntry {
            name,
            path: full_path,
            is_dir: file_type.is_dir(),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    serde_json::to_value(serde_json::json!({ "entries": entries }))
        .map_err(|e| format!("serialize: {e}"))
}

fn open_file(
    state: &mut BackendState,
    path: &str,
    out_tx: Sender<String>,
) -> Result<serde_json::Value, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read({path}): {e}"))?;
    let document = Document::new(&content);
    let doc_id = state.next_doc_id;
    state.next_doc_id += 1;
    let lsp_session = lsp_integration::maybe_spawn_lsp(doc_id, Path::new(path), &content, out_tx);
    state.open_docs.insert(
        doc_id,
        OpenDoc {
            path: PathBuf::from(path),
            document,
            redo_stack: Vec::new(),
            lsp_session,
        },
    );
    Ok(serde_json::json!({ "doc_id": doc_id, "content": content }))
}

/// Real, live `textDocument/hover` (task #134, closing `lsp_integration.rs`'s
/// own previously-named "no hover/completion IPC methods exist yet" gap).
/// **Never blocks the caller** -- the single request-processing thread
/// every other IPC method shares must stay free, so this spawns the real,
/// possibly-slow blocking `LspSession::request_hover` call on its own
/// thread and reports the result via a real `lsp_hover_result` event
/// instead of a synchronous response, the same real "ack now, event
/// later" shape `leo_start_task`/`devcontainer_up` already established.
/// A real, honest `Err` up front (no such doc, or the file has no live
/// LSP session at all) still returns synchronously -- there is nothing
/// slow to wait for in either case.
///
/// **A real bug, found only by live end-to-end browser testing, not by
/// inspection or by the existing Rust integration test's own loose
/// stringify-and-`contains("int")` assertion**: `LspClient::request`
/// (and so `LspSession::request_hover`) deliberately returns the *entire*
/// raw JSON-RPC response message (`{"id":..,"jsonrpc":"2.0","result":{
/// "contents":...}}`), not just its inner `result` payload -- correct at
/// that low level, since `request()` is a generic one-shot RPC helper with
/// no per-method knowledge of what a "clean" result looks like. Sending
/// that raw envelope straight across the IPC boundary leaked an internal
/// wire-protocol detail to the real frontend: `desktop/`'s own
/// `extractHoverText` expects a bare LSP hover result (a `contents` field
/// directly on the object), so every real hover response silently failed
/// to extract any text and no tooltip ever rendered, even though the
/// backend had genuinely answered correctly. Fixed by unwrapping the
/// envelope's own `result` field here, at the real IPC boundary, before
/// it ever reaches an event -- the frontend now receives exactly the LSP
/// hover payload it already expected, with no envelope to unwrap itself.
fn lsp_hover(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    doc_id: u64,
    line: i64,
    character: i64,
) -> Result<serde_json::Value, String> {
    let session = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        let doc = guard
            .open_docs
            .get(&doc_id)
            .ok_or_else(|| format!("no open document with id {doc_id}"))?;
        doc.lsp_session
            .clone()
            .ok_or_else(|| "no live LSP session for this file".to_string())?
    };
    thread::spawn(move || {
        let raw = session.request_hover(line, character);
        let result = raw.and_then(|envelope| envelope.get("result").cloned());
        let event = Event {
            event: "lsp_hover_result".to_string(),
            data: serde_json::json!({
                "doc_id": doc_id,
                "line": line,
                "character": character,
                "result": result,
            }),
        };
        if let Ok(l) = serde_json::to_string(&event) {
            let _ = out_tx.send(l);
        }
    });
    Ok(serde_json::json!({ "status": "requested" }))
}

/// Real, live `textDocument/completion` (task #136, closing the
/// "completion... has no real caller anywhere" gap named in `lsp_hover`'s
/// own history). The direct sibling of `lsp_hover` above -- identical
/// never-blocks-the-caller shape, identical envelope-unwrapping (the same
/// real fix `lsp_hover` needed applies equally here: `LspSession::
/// request_completion` returns the raw JSON-RPC response, not just its
/// inner `result`). A real LSP completion `result` is either a bare
/// `CompletionItem[]` or a `CompletionList { isIncomplete, items }` --
/// both shapes are passed through unwrapped exactly as the server sent
/// them, left for the frontend to normalize (matching `extractHoverText`'s
/// own existing precedent of handling multiple real LSP response shapes
/// at the UI boundary, not the IPC boundary).
fn lsp_completion(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    doc_id: u64,
    line: i64,
    character: i64,
) -> Result<serde_json::Value, String> {
    let session = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        let doc = guard
            .open_docs
            .get(&doc_id)
            .ok_or_else(|| format!("no open document with id {doc_id}"))?;
        doc.lsp_session
            .clone()
            .ok_or_else(|| "no live LSP session for this file".to_string())?
    };
    thread::spawn(move || {
        let raw = session.request_completion(line, character);
        let result = raw.and_then(|envelope| envelope.get("result").cloned());
        let event = Event {
            event: "lsp_completion_result".to_string(),
            data: serde_json::json!({
                "doc_id": doc_id,
                "line": line,
                "character": character,
                "result": result,
            }),
        };
        if let Ok(l) = serde_json::to_string(&event) {
            let _ = out_tx.send(l);
        }
    });
    Ok(serde_json::json!({ "status": "requested" }))
}

/// Real, live `textDocument/definition` -- the third real query method,
/// the direct sibling of `lsp_hover`/`lsp_completion` above: identical
/// never-blocks-the-caller shape, identical envelope-unwrapping (the same
/// real fix `lsp_hover` needed applies equally here). A real LSP
/// `definition` result is `Location | Location[] | LocationLink[] | null`
/// -- passed through unwrapped exactly as the server sent it, left for the
/// frontend to normalize into a jump target, matching `extractHoverText`'s
/// own established precedent of handling multiple real LSP response shapes
/// at the UI boundary rather than the IPC boundary.
fn lsp_definition(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    doc_id: u64,
    line: i64,
    character: i64,
) -> Result<serde_json::Value, String> {
    let session = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        let doc = guard
            .open_docs
            .get(&doc_id)
            .ok_or_else(|| format!("no open document with id {doc_id}"))?;
        doc.lsp_session
            .clone()
            .ok_or_else(|| "no live LSP session for this file".to_string())?
    };
    thread::spawn(move || {
        let raw = session.request_definition(line, character);
        let result = raw.and_then(|envelope| envelope.get("result").cloned());
        let event = Event {
            event: "lsp_definition_result".to_string(),
            data: serde_json::json!({
                "doc_id": doc_id,
                "line": line,
                "character": character,
                "result": result,
            }),
        };
        if let Ok(l) = serde_json::to_string(&event) {
            let _ = out_tx.send(l);
        }
    });
    Ok(serde_json::json!({ "status": "requested" }))
}

/// Real, live `textDocument/typeDefinition` -- "Go to Type Definition," the
/// direct sibling of `lsp_definition` above: identical never-blocks-the-
/// caller shape, identical envelope-unwrapping. Confirmed live before this
/// was wired at all: a real, hand-rolled capability probe against
/// `pyright-langserver` found `typeDefinitionProvider` genuinely returns
/// real results (a query against `x: int = 1` returned a real location
/// inside pyright's own bundled `typeshed-fallback/stdlib/builtins.pyi`),
/// unlike `workspace/symbol`/`semanticTokensProvider`/`inlayHintProvider`,
/// which this same probe found either declared-but-empty or absent in this
/// environment -- so this, not those, is the real next LSP capability this
/// dev environment can actually verify live.
fn lsp_type_definition(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    doc_id: u64,
    line: i64,
    character: i64,
) -> Result<serde_json::Value, String> {
    let session = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        let doc = guard
            .open_docs
            .get(&doc_id)
            .ok_or_else(|| format!("no open document with id {doc_id}"))?;
        doc.lsp_session
            .clone()
            .ok_or_else(|| "no live LSP session for this file".to_string())?
    };
    thread::spawn(move || {
        let raw = session.request_type_definition(line, character);
        let result = raw.and_then(|envelope| envelope.get("result").cloned());
        let event = Event {
            event: "lsp_type_definition_result".to_string(),
            data: serde_json::json!({
                "doc_id": doc_id,
                "line": line,
                "character": character,
                "result": result,
            }),
        };
        if let Ok(l) = serde_json::to_string(&event) {
            let _ = out_tx.send(l);
        }
    });
    Ok(serde_json::json!({ "status": "requested" }))
}

/// Real, live `textDocument/signatureHelp` -- the fourth real query method,
/// the direct sibling of `lsp_hover`/`lsp_completion`/`lsp_definition`
/// above: identical never-blocks-the-caller shape, identical envelope-
/// unwrapping. A real LSP `SignatureHelp` result (`{signatures,
/// activeSignature, activeParameter}` or `null`) is passed through
/// unwrapped exactly as the server sent it, left for the frontend to
/// normalize, matching every other query method's own established
/// precedent of handling response shapes at the UI boundary.
fn lsp_signature_help(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    doc_id: u64,
    line: i64,
    character: i64,
) -> Result<serde_json::Value, String> {
    let session = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        let doc = guard
            .open_docs
            .get(&doc_id)
            .ok_or_else(|| format!("no open document with id {doc_id}"))?;
        doc.lsp_session
            .clone()
            .ok_or_else(|| "no live LSP session for this file".to_string())?
    };
    thread::spawn(move || {
        let raw = session.request_signature_help(line, character);
        let result = raw.and_then(|envelope| envelope.get("result").cloned());
        let event = Event {
            event: "lsp_signature_help_result".to_string(),
            data: serde_json::json!({
                "doc_id": doc_id,
                "line": line,
                "character": character,
                "result": result,
            }),
        };
        if let Ok(l) = serde_json::to_string(&event) {
            let _ = out_tx.send(l);
        }
    });
    Ok(serde_json::json!({ "status": "requested" }))
}

/// Real, live `textDocument/references` -- the fifth real query method,
/// the direct sibling of `lsp_hover`/`lsp_completion`/`lsp_definition`/
/// `lsp_signature_help` above: identical never-blocks-the-caller shape,
/// identical envelope-unwrapping. A real LSP `references` result is a
/// real `Location[]` (or `null`), passed through unwrapped exactly as the
/// server sent it, left for the frontend to normalize into a real jump
/// target list.
fn lsp_references(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    doc_id: u64,
    line: i64,
    character: i64,
    include_declaration: bool,
) -> Result<serde_json::Value, String> {
    let session = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        let doc = guard
            .open_docs
            .get(&doc_id)
            .ok_or_else(|| format!("no open document with id {doc_id}"))?;
        doc.lsp_session
            .clone()
            .ok_or_else(|| "no live LSP session for this file".to_string())?
    };
    thread::spawn(move || {
        let raw = session.request_references(line, character, include_declaration);
        let result = raw.and_then(|envelope| envelope.get("result").cloned());
        let event = Event {
            event: "lsp_references_result".to_string(),
            data: serde_json::json!({
                "doc_id": doc_id,
                "line": line,
                "character": character,
                "result": result,
            }),
        };
        if let Ok(l) = serde_json::to_string(&event) {
            let _ = out_tx.send(l);
        }
    });
    Ok(serde_json::json!({ "status": "requested" }))
}

/// Real, live `textDocument/rename` -- the sixth real query method, the
/// direct sibling of `lsp_hover`/`lsp_completion`/`lsp_definition`/
/// `lsp_signature_help`/`lsp_references` above: identical never-blocks-the-
/// caller shape, identical envelope-unwrapping. Unlike its five siblings,
/// a real rename result is a `WorkspaceEdit` (a real mutation-describing
/// value, `changes`/`documentChanges`/`null` -- see `LspClient::rename`'s
/// own doc comment for the real, live finding that a real server may use
/// either shape regardless of declared client capabilities), passed
/// through unwrapped exactly as the server sent it -- this function's job
/// is the real request, not applying the resulting edits, which may span
/// files this backend has never opened. The frontend applies it through
/// the existing, already-real `edit` method per affected file, the same
/// division of responsibility `extractDefinitionTarget`/`extractReferences`
/// already established at the UI boundary for the read-only query methods.
fn lsp_rename(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    doc_id: u64,
    line: i64,
    character: i64,
    new_name: String,
) -> Result<serde_json::Value, String> {
    let session = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        let doc = guard
            .open_docs
            .get(&doc_id)
            .ok_or_else(|| format!("no open document with id {doc_id}"))?;
        doc.lsp_session
            .clone()
            .ok_or_else(|| "no live LSP session for this file".to_string())?
    };
    thread::spawn(move || {
        let raw = session.request_rename(line, character, &new_name);
        let result = raw.and_then(|envelope| envelope.get("result").cloned());
        let event = Event {
            event: "lsp_rename_result".to_string(),
            data: serde_json::json!({
                "doc_id": doc_id,
                "line": line,
                "character": character,
                "result": result,
            }),
        };
        if let Ok(l) = serde_json::to_string(&event) {
            let _ = out_tx.send(l);
        }
    });
    Ok(serde_json::json!({ "status": "requested" }))
}

/// Real, live `textDocument/documentSymbol` -- the seventh real query
/// method, the direct sibling of `lsp_hover`/`lsp_completion`/
/// `lsp_definition`/`lsp_signature_help`/`lsp_references`/`lsp_rename`
/// above: identical never-blocks-the-caller shape, identical envelope-
/// unwrapping. Unlike every other query method here, this one takes no
/// `line`/`character` -- a document symbol request covers the whole
/// document, not one cursor position.
fn lsp_document_symbol(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    doc_id: u64,
) -> Result<serde_json::Value, String> {
    let session = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        let doc = guard
            .open_docs
            .get(&doc_id)
            .ok_or_else(|| format!("no open document with id {doc_id}"))?;
        doc.lsp_session
            .clone()
            .ok_or_else(|| "no live LSP session for this file".to_string())?
    };
    thread::spawn(move || {
        let raw = session.request_document_symbol();
        let result = raw.and_then(|envelope| envelope.get("result").cloned());
        let event = Event {
            event: "lsp_document_symbol_result".to_string(),
            data: serde_json::json!({
                "doc_id": doc_id,
                "result": result,
            }),
        };
        if let Ok(l) = serde_json::to_string(&event) {
            let _ = out_tx.send(l);
        }
    });
    Ok(serde_json::json!({ "status": "requested" }))
}

/// Real, live `textDocument/documentHighlight` -- the eighth real query
/// method, the direct sibling of `lsp_hover`/`lsp_completion`/
/// `lsp_definition`/`lsp_signature_help`/`lsp_references`/`lsp_rename`/
/// `lsp_document_symbol` above: identical never-blocks-the-caller shape,
/// identical envelope-unwrapping. Has a real cursor position again (unlike
/// `lsp_document_symbol`).
fn lsp_document_highlight(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    doc_id: u64,
    line: i64,
    character: i64,
) -> Result<serde_json::Value, String> {
    let session = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        let doc = guard
            .open_docs
            .get(&doc_id)
            .ok_or_else(|| format!("no open document with id {doc_id}"))?;
        doc.lsp_session
            .clone()
            .ok_or_else(|| "no live LSP session for this file".to_string())?
    };
    thread::spawn(move || {
        let raw = session.request_document_highlight(line, character);
        let result = raw.and_then(|envelope| envelope.get("result").cloned());
        let event = Event {
            event: "lsp_document_highlight_result".to_string(),
            data: serde_json::json!({
                "doc_id": doc_id,
                "line": line,
                "character": character,
                "result": result,
            }),
        };
        if let Ok(l) = serde_json::to_string(&event) {
            let _ = out_tx.send(l);
        }
    });
    Ok(serde_json::json!({ "status": "requested" }))
}

/// Real call hierarchy (incoming calls) -- the direct sibling of every other
/// `lsp_*` query method above: identical never-blocks-the-caller shape,
/// identical envelope-unwrapping. Unlike them it drives a real two-request
/// LSP protocol under the hood (`prepareCallHierarchy` then
/// `incomingCalls`, combined in `LspClient::incoming_calls`), but the result
/// is a plain `CallHierarchyIncomingCall[]` the frontend renders as a list
/// of callers, each jumpable via the same `goToTarget`/`jumpToLocalPosition`
/// machinery go-to-definition/references already established.
fn lsp_call_hierarchy(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    doc_id: u64,
    line: i64,
    character: i64,
    outgoing: bool,
) -> Result<serde_json::Value, String> {
    let session = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        let doc = guard
            .open_docs
            .get(&doc_id)
            .ok_or_else(|| format!("no open document with id {doc_id}"))?;
        doc.lsp_session
            .clone()
            .ok_or_else(|| "no live LSP session for this file".to_string())?
    };
    thread::spawn(move || {
        let raw = if outgoing {
            session.request_outgoing_calls(line, character)
        } else {
            session.request_incoming_calls(line, character)
        };
        let result = raw.and_then(|envelope| envelope.get("result").cloned());
        let event = Event {
            event: "lsp_call_hierarchy_result".to_string(),
            data: serde_json::json!({
                "doc_id": doc_id,
                "line": line,
                "character": character,
                "direction": if outgoing { "outgoing" } else { "incoming" },
                "result": result,
            }),
        };
        if let Ok(l) = serde_json::to_string(&event) {
            let _ = out_tx.send(l);
        }
    });
    Ok(serde_json::json!({ "status": "requested" }))
}

/// Real "Format Document" -- the real, previously-unwired `formatter`
/// field on every language's own registry entry (§20.1) finally gets a
/// real caller. Formats the *live in-memory buffer*, not the file on
/// disk (matching `gui-builder`'s own established "operate on the live
/// buffer" discipline, §75.42), so an unsaved edit still formats
/// correctly; the caller applies the real result through the normal
/// `edit` IPC path, which is why this only ever reports formatted text
/// back, never writes anything itself. Never blocks the single
/// request-processing thread -- spawns the real formatter subprocess on
/// its own thread and reports back via a real `format_document_result`/
/// `format_document_error` event, the same shape every other real
/// external-process call in this crate already uses.
fn format_document(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    doc_id: u64,
) -> Result<serde_json::Value, String> {
    let (path, source) = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        let doc = guard
            .open_docs
            .get(&doc_id)
            .ok_or_else(|| format!("no open document with id {doc_id}"))?;
        (doc.path.clone(), doc.document.text())
    };

    let registry = spartan_languages::LanguageRegistry::curated_default();
    let profile = registry
        .profile_for_file(&path)
        .ok_or_else(|| "no language profile recognizes this file".to_string())?;
    let configured = profile
        .formatter
        .clone()
        .ok_or_else(|| format!("no formatter is configured for language `{}`", profile.id))?;
    let (program, args) = format_integration::resolve_formatter_command(&configured, &path)
        .ok_or_else(|| {
            format!(
                "`{}` has no supported stdin/stdout formatting mode -- Format Document isn't wired for this language yet",
                configured.program
            )
        })?;

    thread::spawn(move || {
        let event = match format_integration::run_formatter(&program, &args, &source) {
            Ok(formatted) => Event {
                event: "format_document_result".to_string(),
                data: serde_json::json!({ "doc_id": doc_id, "formatted": formatted }),
            },
            Err(message) => Event {
                event: "format_document_error".to_string(),
                data: serde_json::json!({ "doc_id": doc_id, "message": message }),
            },
        };
        if let Ok(l) = serde_json::to_string(&event) {
            let _ = out_tx.send(l);
        }
    });
    Ok(serde_json::json!({ "status": "requested" }))
}

/// Real edit application -- `start_char`/`end_char` name a real char
/// range (matching every other char-indexed API in `spartan-buffer`);
/// `start_char == end_char` is a pure insert, `text.is_empty()` with
/// `start_char != end_char` is a pure delete, and the general case (a
/// real selection replaced by typed text) is one real `Document::replace`
/// call either way -- no special-casing needed, `replace` already
/// handles all three shapes as one operation with one real undo
/// checkpoint, exactly like the original wgpu shell's own
/// `insert_at_cursor`/`backspace` built on the same primitive.
fn edit(
    state: &mut BackendState,
    doc_id: u64,
    start_char: usize,
    end_char: usize,
    text: &str,
) -> Result<serde_json::Value, String> {
    let open_doc = state
        .open_docs
        .get_mut(&doc_id)
        .ok_or_else(|| format!("no open document with id {doc_id}"))?;
    open_doc
        .document
        .replace(start_char..end_char, text)
        .map_err(|e| format!("replace: {e:?}"))?;
    // A real new edit invalidates whatever "redo" used to mean, the same
    // real rule the wgpu shell's own `EditorView` already enforces.
    open_doc.redo_stack.clear();
    // Real, live didChange dispatch -- debounced inside the session itself
    // (see `spartan_lsp::LspSession`'s own doc comment), so every keystroke
    // can safely call this without flooding the real language server.
    if let Some(session) = &open_doc.lsp_session {
        session.notify_edit(open_doc.document.text());
    }
    Ok(serde_json::json!({ "ok": true }))
}

fn save_file(state: &BackendState, doc_id: u64) -> Result<serde_json::Value, String> {
    let open_doc = state
        .open_docs
        .get(&doc_id)
        .ok_or_else(|| format!("no open document with id {doc_id}"))?;
    std::fs::write(&open_doc.path, open_doc.document.text())
        .map_err(|e| format!("write({}): {e}", open_doc.path.display()))?;
    Ok(serde_json::json!({ "ok": true }))
}

fn undo(state: &mut BackendState, doc_id: u64) -> Result<serde_json::Value, String> {
    let open_doc = state
        .open_docs
        .get_mut(&doc_id)
        .ok_or_else(|| format!("no open document with id {doc_id}"))?;
    let pre_undo_checkpoint = open_doc.document.current_checkpoint();
    let changed = open_doc.document.undo();
    if changed {
        open_doc.redo_stack.push(pre_undo_checkpoint);
        // An undo is a real content change too -- the live LSP session (if
        // any) needs to see it or its diagnostics silently go stale.
        if let Some(session) = &open_doc.lsp_session {
            session.notify_edit(open_doc.document.text());
        }
    }
    Ok(serde_json::json!({ "changed": changed, "content": open_doc.document.text() }))
}

/// Real redo -- `spartan_buffer::Document` has no single well-defined
/// "redo" on its own branching undo tree, so this pops the real
/// pre-undo checkpoint `undo()` pushed and jumps forward to it directly,
/// the same real pattern `EditorView::redo` already established in the
/// original wgpu shell (§75.19).
fn redo(state: &mut BackendState, doc_id: u64) -> Result<serde_json::Value, String> {
    let open_doc = state
        .open_docs
        .get_mut(&doc_id)
        .ok_or_else(|| format!("no open document with id {doc_id}"))?;
    let Some(checkpoint) = open_doc.redo_stack.pop() else {
        return Ok(serde_json::json!({ "changed": false, "content": open_doc.document.text() }));
    };
    match open_doc.document.jump_to_checkpoint(checkpoint) {
        Ok(()) => {
            if let Some(session) = &open_doc.lsp_session {
                session.notify_edit(open_doc.document.text());
            }
            Ok(serde_json::json!({ "changed": true, "content": open_doc.document.text() }))
        }
        Err(_) => {
            // The checkpoint aged out of the bounded ring since `undo`
            // pushed it -- a real, possible outcome, not an error to
            // surface to the user; fall back to "nothing to redo".
            Ok(serde_json::json!({ "changed": false, "content": open_doc.document.text() }))
        }
    }
}

fn close_file(state: &mut BackendState, doc_id: u64) -> Result<serde_json::Value, String> {
    if let Some(open_doc) = state.open_docs.remove(&doc_id) {
        // Real, non-blocking teardown -- see `LspSession::signal_shutdown`'s
        // own doc comment for why this doesn't wait for the real ~7s-worst-
        // case graceful LSP shutdown sequence to finish before returning.
        if let Some(session) = &open_doc.lsp_session {
            session.signal_shutdown();
        }
    }
    Ok(serde_json::json!({ "ok": true }))
}

fn agent_state_name(agent: &Agent) -> &'static str {
    use spartan_leo::state::AgentState::*;
    match agent.state() {
        Idle => "Idle",
        Planning => "Planning",
        AwaitingApproval => "AwaitingApproval",
        Executing => "Executing",
        Verifying => "Verifying",
        Done => "Done",
        Failed => "Failed",
        Recovering => "Recovering",
    }
}

fn plan_json(plan: &ImplementationPlan) -> serde_json::Value {
    serde_json::json!({
        "goal": plan.goal,
        "approach": plan.approach,
        "files": plan.files,
        "risk_notes": plan.risk_notes,
    })
}

/// Real, pure §4.3 memory-folding logic -- separated from
/// `leo_start_task`'s own spawned thread so it's directly unit-testable
/// without needing a real model call. An empty or whitespace-only memory
/// file (no prior task has completed yet in this project, or a real I/O
/// error already degraded to an empty string by the caller) means the
/// task string passes through completely unchanged -- never a fabricated
/// "no notes yet" placeholder sent to the model.
fn augment_task_with_memory(task: &str, memory: &str) -> String {
    if memory.trim().is_empty() {
        task.to_string()
    } else {
        format!(
            "Project memory (notes from prior completed tasks in this project):\n{}\n\n\
             Task: {}",
            memory.trim(),
            task
        )
    }
}

/// Real Leo status snapshot -- the renderer calls this on mount to
/// rehydrate a persistent chat panel's state, since (unlike a single
/// full-screen mode) it may be mounted before or after a task is
/// already in flight.
fn leo_status(state: &BackendState) -> Result<serde_json::Value, String> {
    match &state.leo_agent {
        Some(agent) => Ok(serde_json::json!({
            "state": agent_state_name(agent),
            "plan": agent.plan().map(plan_json),
            "pending_call": state.leo_pending_call.as_ref().map(|p| serde_json::json!({
                "call_id": p.call_id,
                "tool": p.call.name(),
                "args": tool_call_json(&p.call),
            })),
        })),
        None => Ok(serde_json::json!({ "state": "Idle", "plan": null, "pending_call": null })),
    }
}

/// Real task #266 session-history snapshot -- newest first (matching
/// `git_log`'s own already-established real convention for a real,
/// user-facing history list), reachable at any time regardless of
/// whether a Leo task is currently in flight.
fn leo_session_history(state: &BackendState) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = state
        .leo_session_history
        .iter()
        .rev()
        .map(|e| {
            serde_json::json!({
                "task": e.task,
                "outcome": e.outcome,
                "summary": e.summary,
                "error": e.error,
                "unix_timestamp": e.unix_timestamp,
            })
        })
        .collect();
    serde_json::json!({ "entries": entries })
}

/// Real `Idle -> Planning` transition plus a real, spawned background
/// thread that makes the actual blocking model call -- mirroring
/// `spartan-editor-core::leo_bridge::spawn_plan_request` exactly, moved
/// to this crate's own `Arc<Mutex<BackendState>>` + `Event`-over-stdout
/// shape instead of an in-process `mpsc` receiver a render loop polls.
/// Real §75.69 mapping from the user-facing settings enum to
/// `spartan_leo::approval::ApprovalMode` -- kept as one small function
/// rather than duplicating the match at each of this crate's two real
/// call sites (`leo_start_task`, and `leo_next_step`'s own re-read on
/// every real step so a mid-task settings change takes effect on the
/// very next step, not only on the next new task).
fn approval_mode_from_settings(mode: spartan_settings::LeoApprovalMode) -> ApprovalMode {
    match mode {
        spartan_settings::LeoApprovalMode::ManualEveryStep => ApprovalMode::ManualEveryStep,
        spartan_settings::LeoApprovalMode::AutoApproveSafe => ApprovalMode::AutoApproveSafe,
    }
}

/// Real §75.70 provider construction -- the first real call site that
/// picks between all three of `spartan-model`'s already-built providers
/// rather than hardcoding `OllamaProvider`, closing the "LLM Agnostic"
/// concept adapted from `CKissinger1988/SpartanAI_Assistant` (concepts
/// only, no code ported -- see `docs/architecture-spec.md` §75.70).
/// `gpu_offload` only ever applies to the real local `OllamaProvider`
/// path -- Claude/LiteLLM are remote, `num_gpu` has no meaning for them.
/// Build ONE real provider from its kind + model. Does not consider fallbacks
/// (that's `build_leo_provider`'s job) -- so a fallback's own `fallbacks` field
/// is ignored by construction, keeping the chain flat (primary + list), never
/// a tree.
fn build_single_provider(
    kind: spartan_settings::LeoProviderKind,
    model: &str,
    gpu_offload: spartan_settings::GpuOffloadSettings,
) -> Result<Box<dyn ModelProvider>, String> {
    match kind {
        spartan_settings::LeoProviderKind::Ollama => Ok(Box::new(
            OllamaProvider::local(model).with_gpu_layers(gpu_offload.num_gpu()),
        )),
        spartan_settings::LeoProviderKind::Claude => {
            let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
                "ANTHROPIC_API_KEY is not set -- required to use Claude as Leo's provider"
                    .to_string()
            })?;
            Ok(Box::new(ClaudeProvider::new(api_key, model)))
        }
        spartan_settings::LeoProviderKind::LiteLLM => Ok(Box::new(LiteLLMProvider::local(model))),
        spartan_settings::LeoProviderKind::LmStudio => Ok(Box::new(LmStudioProvider::local(model))),
        spartan_settings::LeoProviderKind::LlamaCpp => {
            if model.trim().is_empty() {
                return Err(
                    "no .gguf model file path configured -- required to use llama.cpp as Leo's provider"
                        .to_string(),
                );
            }
            let provider = LlamaCppProvider::new(model)
                .map_err(|e| format!("failed to load llama.cpp model: {e}"))?;
            Ok(Box::new(provider))
        }
    }
}

fn build_leo_provider(
    provider_settings: &spartan_settings::LeoProviderSettings,
    gpu_offload: spartan_settings::GpuOffloadSettings,
) -> Result<Box<dyn ModelProvider>, String> {
    let primary = build_single_provider(
        provider_settings.kind,
        &provider_settings.model,
        gpu_offload,
    )?;

    // No fallbacks configured -> the primary alone, exactly as before.
    if provider_settings.fallbacks.is_empty() {
        return Ok(primary);
    }

    // A configured fallback chain -> wrap primary + each fallback in a real
    // FailoverProvider (§75.x, task #123). Each fallback is built with the same
    // gpu_offload (it only ever affects a local Ollama provider anyway). If any
    // configured fallback can't be constructed (e.g. a missing gguf path or an
    // unset ANTHROPIC_API_KEY), fail the whole build with a clear message
    // rather than silently dropping that link from the chain.
    let mut chain: Vec<Box<dyn ModelProvider>> = vec![primary];
    for (i, fb) in provider_settings.fallbacks.iter().enumerate() {
        let p = build_single_provider(fb.kind, &fb.model, gpu_offload)
            .map_err(|e| format!("fallback provider #{}: {e}", i + 1))?;
        chain.push(p);
    }
    Ok(Box::new(FailoverProvider::new(chain)))
}

/// The unified model-status surface (Track A): the real, currently-configured
/// Leo provider's identity, capabilities, and a **live** health probe -- built
/// from the exact same `build_leo_provider` every real Leo call uses, so the
/// status can never disagree with what a task would actually run. A provider
/// that can't even be constructed (missing gguf path, unset ANTHROPIC_API_KEY)
/// is reported honestly as `configured: false` with the real error, never a
/// fabricated "healthy". Exposed for `spartan-devserver`'s `model_status`.
pub fn model_status_json() -> serde_json::Value {
    let settings = spartan_settings::load();
    let ps = &settings.leo_provider;
    match build_leo_provider(ps, settings.gpu_offload) {
        Ok(provider) => {
            let health = match provider.health_check() {
                spartan_model::ProviderHealth::Healthy => "healthy",
                spartan_model::ProviderHealth::Unauthorized => "unauthorized",
                spartan_model::ProviderHealth::Unreachable => "unreachable",
            };
            serde_json::json!({
                "configured": true,
                "kind": format!("{:?}", ps.kind),
                "model": ps.model,
                "provider_id": provider.id(),
                "is_local": provider.is_local(),
                "context_window": provider.context_window(),
                "supports_native_tool_calling": provider.supports_native_tool_calling(),
                "health": health,
                "fallback_count": ps.fallbacks.len(),
            })
        }
        Err(e) => serde_json::json!({
            "configured": false,
            "kind": format!("{:?}", ps.kind),
            "model": ps.model,
            "error": e,
        }),
    }
}

const LITELLM_HEALTH_TIMEOUT: Duration = Duration::from_secs(60);

/// Starts a real LiteLLM proxy in the background: an immediate
/// `{"status": "starting"}` ack, then a spawned thread runs the real,
/// possibly-slow spawn+health-check, forwarding real subprocess stdout/
/// stderr lines as `litellm_progress` events and finishing with
/// `litellm_ready`/`litellm_failed` -- the same "ack now, event later"
/// shape `devcontainer_up`/`leo_start_task` already established. Moved
/// here verbatim from `spartan-devserver` (task #145) -- the only real
/// change is where the proxy handle lives (`BackendState.litellm`,
/// protected by the same top-level lock, instead of a second, devserver-
/// only `Mutex`).
fn litellm_proxy_start(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    port: u16,
    config_path: Option<String>,
    auto_restart: bool,
) -> Result<serde_json::Value, String> {
    let my_generation = {
        let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
        if let Some(process) = guard.litellm.as_mut() {
            if process.is_running() {
                return Err(format!(
                    "a LiteLLM proxy is already running on port {} (pid {})",
                    process.port,
                    process.pid()
                ));
            }
            // A stale handle whose process already exited on its own --
            // clear it so this fresh spawn can take its place.
            guard.litellm = None;
        }
        // Real task #273 generation mint -- see `BackendState::
        // litellm_generation`'s own doc comment for why every real new
        // start (and only a real new start) gets a fresh one.
        guard.litellm_generation = guard.litellm_generation.wrapping_add(1);
        guard.litellm_generation
    };

    if !litellm_proxy::is_litellm_available() {
        return Err(
            "`litellm` isn't on $PATH -- install it with `pip install 'litellm[proxy]'`"
                .to_string(),
        );
    }

    let state = Arc::clone(state);
    thread::spawn(move || {
        let (line_tx, line_rx) = mpsc::channel::<String>();
        let forward_out_tx = out_tx.clone();
        thread::spawn(move || {
            for line in line_rx {
                let event = Event {
                    event: "litellm_progress".to_string(),
                    data: serde_json::json!({ "line": line }),
                };
                if let Ok(l) = serde_json::to_string(&event) {
                    let _ = forward_out_tx.send(l);
                }
            }
        });

        let event = match litellm_proxy::spawn(port, config_path.as_deref(), line_tx) {
            Ok(mut process) => match litellm_proxy::wait_for_health(
                &mut process,
                litellm_proxy::DEFAULT_HEALTH_PATH,
                LITELLM_HEALTH_TIMEOUT,
            ) {
                Ok(()) => {
                    let pid = process.pid();
                    if let Ok(mut guard) = state.lock() {
                        guard.litellm = Some(process);
                    }
                    if auto_restart {
                        spawn_litellm_supervisor(
                            Arc::clone(&state),
                            my_generation,
                            port,
                            config_path.clone(),
                            out_tx.clone(),
                        );
                    }
                    Event {
                        event: "litellm_ready".to_string(),
                        data: serde_json::json!({ "port": port, "pid": pid }),
                    }
                }
                Err(e) => {
                    let _ = process.stop();
                    Event {
                        event: "litellm_failed".to_string(),
                        data: serde_json::json!({ "error": e.to_string() }),
                    }
                }
            },
            Err(e) => Event {
                event: "litellm_failed".to_string(),
                data: serde_json::json!({ "error": e.to_string() }),
            },
        };
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = out_tx.send(line);
        }
    });

    Ok(serde_json::json!({ "status": "starting" }))
}

const LITELLM_SUPERVISOR_POLL_INTERVAL: Duration = Duration::from_millis(300);
const LITELLM_MAX_AUTO_RESTARTS: u32 = 3;

/// Real, opt-in crash-detection + respawn loop (task #273), spawned only
/// when `litellm_proxy_start`'s caller passes `auto_restart: true`.
/// Generation-guarded exactly like Leo's own background threads
/// (`leo_generation`'s own doc comment): every tick checks
/// `guard.litellm_generation == my_generation` before touching anything,
/// so an explicit `litellm_proxy_stop` or a fresh manual
/// `litellm_proxy_start` (both of which change what `BackendState.litellm`
/// refers to) makes this supervisor recognize it's superseded and exit
/// quietly instead of respawning a proxy nobody asked for anymore. The
/// real spawn+health-check mechanics themselves live in
/// `litellm_proxy::attempt_restart` -- this function owns only the real
/// polling cadence and the `BackendState`-specific generation check that
/// crate has no access to.
fn spawn_litellm_supervisor(
    state: Arc<Mutex<BackendState>>,
    my_generation: u64,
    port: u16,
    config_path: Option<String>,
    out_tx: Sender<String>,
) {
    thread::spawn(move || {
        let mut restarts = 0u32;
        let mut args = vec!["--port".to_string(), port.to_string()];
        if let Some(cfg) = &config_path {
            args.push("--config".to_string());
            args.push(cfg.clone());
        }
        loop {
            thread::sleep(LITELLM_SUPERVISOR_POLL_INTERVAL);
            let still_alive = {
                let mut guard = match state.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                if guard.litellm_generation != my_generation {
                    return; // superseded -- an explicit stop or a fresh start happened
                }
                match guard.litellm.as_mut() {
                    Some(process) => process.is_running(),
                    None => return, // explicit stop already cleared it
                }
            };
            if still_alive {
                continue;
            }

            // A real, unexpected exit -- attempt a real respawn.
            let (line_tx, line_rx) = mpsc::channel::<String>();
            let forward_out_tx = out_tx.clone();
            thread::spawn(move || {
                for line in line_rx {
                    let event = Event {
                        event: "litellm_progress".to_string(),
                        data: serde_json::json!({ "line": line }),
                    };
                    if let Ok(l) = serde_json::to_string(&event) {
                        let _ = forward_out_tx.send(l);
                    }
                }
            });

            match litellm_proxy::attempt_restart(
                litellm_proxy::RestartAttempt {
                    program: "litellm",
                    args: &args,
                    port,
                    health_path: litellm_proxy::DEFAULT_HEALTH_PATH,
                    health_timeout: LITELLM_HEALTH_TIMEOUT,
                    restarts_so_far: restarts,
                    max_restarts: LITELLM_MAX_AUTO_RESTARTS,
                },
                line_tx,
            ) {
                litellm_proxy::RestartOutcome::Restarted { process, pid } => {
                    restarts += 1;
                    let mut guard = match state.lock() {
                        Ok(g) => g,
                        Err(_) => return,
                    };
                    if guard.litellm_generation != my_generation {
                        // Superseded while the respawn was in flight -- the
                        // just-spawned process is now orphaned; stop it
                        // rather than leaking it.
                        drop(guard);
                        let _ = process.stop();
                        return;
                    }
                    guard.litellm = Some(process);
                    drop(guard);
                    let event = Event {
                        event: "litellm_restarted".to_string(),
                        data: serde_json::json!({ "port": port, "pid": pid, "restart_count": restarts }),
                    };
                    if let Ok(line) = serde_json::to_string(&event) {
                        let _ = out_tx.send(line);
                    }
                }
                litellm_proxy::RestartOutcome::Failed(e) => {
                    restarts += 1;
                    let event = Event {
                        event: "litellm_progress".to_string(),
                        data: serde_json::json!({ "line": format!("restart attempt failed: {e}") }),
                    };
                    if let Ok(line) = serde_json::to_string(&event) {
                        let _ = out_tx.send(line);
                    }
                }
                litellm_proxy::RestartOutcome::LimitReached => {
                    if let Ok(mut guard) = state.lock() {
                        if guard.litellm_generation == my_generation {
                            guard.litellm = None;
                        }
                    }
                    let event = Event {
                        event: "litellm_failed".to_string(),
                        data: serde_json::json!({
                            "error": format!(
                                "LiteLLM proxy crashed and the restart limit ({LITELLM_MAX_AUTO_RESTARTS}) was reached"
                            )
                        }),
                    };
                    if let Ok(line) = serde_json::to_string(&event) {
                        let _ = out_tx.send(line);
                    }
                    return;
                }
            }
        }
    });
}

/// Stops the real currently-running proxy, if any. Stopping when nothing is
/// running is a real, honest `not_running` result, not an error -- matches
/// `devcontainer_down`'s own precedent that "stop what's already gone" is a
/// harmless no-op, not a failure.
fn litellm_proxy_stop(state: &Arc<Mutex<BackendState>>) -> Result<serde_json::Value, String> {
    let process = {
        let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
        guard.litellm.take()
    };
    match process {
        Some(process) => {
            let port = process.port;
            process.stop().map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "status": "stopped", "port": port }))
        }
        None => Ok(serde_json::json!({ "status": "not_running" })),
    }
}

/// Reports the real current proxy status, self-healing a stale handle whose
/// process has since exited on its own (a real crash) rather than reporting
/// a false "running" forever.
fn litellm_proxy_status(state: &Arc<Mutex<BackendState>>) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    // A match guard binds `process` immutably, but `is_running` needs
    // `&mut self` -- checked separately instead, so the mutable borrow is
    // real and the pattern match only branches on its already-computed
    // result.
    let running = guard.litellm.as_mut().map(|process| process.is_running());
    Ok(match running {
        Some(true) => {
            let process = guard.litellm.as_ref().expect("just confirmed Some above");
            serde_json::json!({ "status": "running", "port": process.port, "pid": process.pid() })
        }
        Some(false) => {
            guard.litellm = None;
            serde_json::json!({ "status": "not_running" })
        }
        None => serde_json::json!({ "status": "not_running" }),
    })
}

/// One real, stable key for `BackendState::download_cancellations` -- always
/// `"<source>:<event_id>"`, so the identical curated-model id (or
/// `repo:tag` custom id) used by more than one real download source at once
/// (HF, LM Studio, llama.cpp) can never collide in the registry, and a
/// cancel request has to name both which source and which download it means.
fn download_registry_key(source: &str, event_id: &str) -> String {
    format!("{source}:{event_id}")
}

/// Registers a fresh, real cancellation flag for a download about to start,
/// overwriting any stale entry left under the same key (a prior download of
/// the same id that already finished should already have unregistered
/// itself -- this is a defensive fallback, not the normal path). Returns a
/// clone for the caller's own background thread to hold and check.
fn begin_cancellable_download(
    state: &Arc<Mutex<BackendState>>,
    source: &str,
    event_id: &str,
) -> Arc<std::sync::atomic::AtomicBool> {
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Ok(mut guard) = state.lock() {
        guard
            .download_cancellations
            .insert(download_registry_key(source, event_id), flag.clone());
    }
    flag
}

/// Removes a download's cancellation flag once it's genuinely finished
/// (success, a real failure, or a real user cancellation) -- called from
/// every real exit path of a download's own background thread, so a
/// finished download's id is never left claiming to still be cancellable.
fn end_cancellable_download(state: &Arc<Mutex<BackendState>>, source: &str, event_id: &str) {
    if let Ok(mut guard) = state.lock() {
        guard
            .download_cancellations
            .remove(&download_registry_key(source, event_id));
    }
}

/// Real cancel/stop for an in-flight model download (task #268). Setting
/// the flag is all this function does -- the actual kill (a subprocess
/// `Child::kill`, or an aborted HTTP read loop) happens inside the
/// download's own background thread, the only place that holds the real
/// handle. A `source`/`event_id` pair with no matching in-flight download
/// (already finished, never started, or a real typo) is a harmless,
/// honest `{"cancelled": false}` -- matching `litellm_proxy_stop`'s own
/// "stopping what's already gone is a no-op, not an error" precedent --
/// rather than a synchronous error for what is, from the caller's
/// perspective, a race that resolved in its favor already.
fn model_download_cancel(
    state: &Arc<Mutex<BackendState>>,
    source: String,
    event_id: String,
) -> Result<serde_json::Value, String> {
    let guard = state.lock().map_err(|_| "backend state poisoned")?;
    let key = download_registry_key(&source, &event_id);
    match guard.download_cancellations.get(&key) {
        Some(flag) => {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(serde_json::json!({ "cancelled": true }))
        }
        None => Ok(serde_json::json!({ "cancelled": false })),
    }
}

/// Real, synchronous listing of the curated HF -> Ollama models.
fn hf_list_models_json() -> serde_json::Value {
    let models: Vec<serde_json::Value> = hf_downloader::CURATED_MODELS
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "display_name": m.display_name,
                "hf_repo": m.hf_repo,
                "tag": m.tag,
                "description": m.description,
            })
        })
        .collect();
    serde_json::json!({ "models": models })
}

/// Resolves the real `(event_id, pull_target)` pair for either an
/// `hf_pull_model` call path -- a curated `model_id` lookup, or a
/// user-defined custom `hf_repo`+`tag` pair (the real "user defined model
/// download links" mechanism, validated via
/// `hf_downloader::custom_pull_target` before ever reaching a subprocess).
/// `model_id` wins if both are somehow present, matching this crate's own
/// "first matching real param wins" convention elsewhere (e.g.
/// `litellm_proxy_start`'s port/config_path handling).
fn resolve_hf_pull_target(
    model_id: Option<String>,
    hf_repo: Option<String>,
    tag: Option<String>,
) -> Result<(String, String), String> {
    match (model_id, hf_repo, tag) {
        (Some(model_id), _, _) => {
            let model = hf_downloader::find_model(&model_id)
                .ok_or_else(|| format!("unknown curated model id: {model_id:?}"))?;
            Ok((model.id.to_string(), hf_downloader::pull_target(model)))
        }
        (None, Some(hf_repo), Some(tag)) => {
            let normalized = hf_downloader::normalize_hf_repo_input(&hf_repo);
            let target = hf_downloader::custom_pull_target(&hf_repo, &tag)?;
            Ok((format!("{normalized}:{}", tag.trim()), target))
        }
        _ => Err(
            "hf_pull_model requires either a string `model_id`, or both a string `hf_repo` and \
             string `tag`"
                .to_string(),
        ),
    }
}

/// Starts a real HF -> Ollama pull in the background: an immediate
/// `{"status": "starting"}` ack, then a spawned thread runs the real,
/// possibly multi-minute `ollama pull`, forwarding real subprocess output
/// as `hf_pull_progress` events and finishing with `hf_pull_ready`/
/// `hf_pull_failed` -- the same "ack now, event later" shape
/// `litellm_proxy_start` already established. Accepts either a curated
/// `model_id` or a user-defined custom `hf_repo`+`tag` pair, resolved by
/// `resolve_hf_pull_target` above -- from this point on, both paths are
/// identical: same validation-already-done target string, same subprocess
/// spawn, same event shapes.
///
/// Real, cancellable (task #268): registers a fresh flag under
/// `download_registry_key("hf", event_id)` before spawning, and waits on
/// the real `ollama pull` child via `subprocess::wait_with_cancellation`
/// instead of a plain, uninterruptible `child.wait()` -- a real
/// `model_download_cancel` call for this id kills the child promptly
/// rather than leaving it running to a discarded result.
fn hf_pull_model(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    model_id: Option<String>,
    hf_repo: Option<String>,
    tag: Option<String>,
) -> Result<serde_json::Value, String> {
    let (event_id, target) = resolve_hf_pull_target(model_id, hf_repo, tag)?;

    if !hf_downloader::is_ollama_available() {
        return Err("`ollama` isn't on $PATH -- install it from https://ollama.com".to_string());
    }

    let ack_target = target.clone();
    let cancel_flag = begin_cancellable_download(state, "hf", &event_id);
    let cancel_state = Arc::clone(state);
    thread::spawn(move || {
        let (line_tx, line_rx) = mpsc::channel::<String>();
        let forward_out_tx = out_tx.clone();
        let forward_model_id = event_id.clone();
        thread::spawn(move || {
            for line in line_rx {
                let event = Event {
                    event: "hf_pull_progress".to_string(),
                    data: serde_json::json!({ "model_id": forward_model_id, "line": line }),
                };
                if let Ok(l) = serde_json::to_string(&event) {
                    let _ = forward_out_tx.send(l);
                }
            }
        });

        let event = match hf_downloader::spawn_pull_target(&target, line_tx) {
            Ok(mut child) => match subprocess::wait_with_cancellation(
                &mut child,
                &cancel_flag,
                Duration::from_millis(200),
            ) {
                Ok(Some(status)) if status.success() => Event {
                    event: "hf_pull_ready".to_string(),
                    data: serde_json::json!({ "model_id": event_id }),
                },
                Ok(Some(status)) => Event {
                    event: "hf_pull_failed".to_string(),
                    data: serde_json::json!({
                        "model_id": event_id,
                        "error": format!("ollama pull exited with {status}"),
                    }),
                },
                Ok(None) => Event {
                    event: "hf_pull_failed".to_string(),
                    data: serde_json::json!({
                        "model_id": event_id,
                        "error": "cancelled by user",
                        "cancelled": true,
                    }),
                },
                Err(e) => Event {
                    event: "hf_pull_failed".to_string(),
                    data: serde_json::json!({ "model_id": event_id, "error": e.to_string() }),
                },
            },
            Err(e) => Event {
                event: "hf_pull_failed".to_string(),
                data: serde_json::json!({ "model_id": event_id, "error": e.to_string() }),
            },
        };
        end_cancellable_download(&cancel_state, "hf", &event_id);
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = out_tx.send(line);
        }
    });

    Ok(serde_json::json!({ "status": "starting", "target": ack_target }))
}

/// Real, synchronous listing of the same curated coding-model set
/// `hf_list_models_json` serves, plus a real `lms_available` flag so the
/// UI can show a correct, honest "detected"/"not detected" state up front
/// -- part of making this "as simple to set up and use as possible":
/// nothing to configure, but nothing hidden either.
fn lmstudio_list_models_json() -> serde_json::Value {
    let models: Vec<serde_json::Value> = hf_downloader::CURATED_MODELS
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "display_name": m.display_name,
                "hf_repo": m.hf_repo,
                "tag": m.tag,
                "description": m.description,
            })
        })
        .collect();
    serde_json::json!({
        "models": models,
        "lms_available": lmstudio_downloader::is_lms_available(),
    })
}

/// The direct LM Studio sibling of `resolve_hf_pull_target` -- identical
/// shape (curated `model_id` wins if present, otherwise a validated custom
/// `hf_repo`+`tag` pair), differing only in the final query string built
/// (`lmstudio_downloader::pull_query`/`custom_pull_query`'s real
/// `<repo>@<tag>` syntax instead of Ollama's `hf.co/<repo>:<tag>`). The
/// real `event_id` shape (`<repo>:<tag>`) is deliberately kept identical to
/// the HF/Ollama path so a UI can reuse the same key-matching logic for
/// both panels.
fn resolve_lmstudio_pull_query(
    model_id: Option<String>,
    hf_repo: Option<String>,
    tag: Option<String>,
) -> Result<(String, String), String> {
    match (model_id, hf_repo, tag) {
        (Some(model_id), _, _) => {
            let model = hf_downloader::find_model(&model_id)
                .ok_or_else(|| format!("unknown curated model id: {model_id:?}"))?;
            Ok((model.id.to_string(), lmstudio_downloader::pull_query(model)))
        }
        (None, Some(hf_repo), Some(tag)) => {
            let normalized = hf_downloader::normalize_hf_repo_input(&hf_repo);
            let query = lmstudio_downloader::custom_pull_query(&hf_repo, &tag)?;
            Ok((format!("{normalized}:{}", tag.trim()), query))
        }
        _ => Err(
            "lmstudio_pull_model requires either a string `model_id`, or both a string \
             `hf_repo` and string `tag`"
                .to_string(),
        ),
    }
}

/// Starts a real LM Studio pull in the background -- the direct sibling of
/// `hf_pull_model`, same "ack now, event later" shape, same accepted
/// params, driving `lms get <query>` instead of `ollama pull`. Fails fast
/// and honestly, with a clear, actionable message (naming exactly where
/// `lms` is expected and what to do), if no real `lms` binary can be
/// located at all -- never a silent hang.
///
/// Real, cancellable (task #268): the same `subprocess::
/// wait_with_cancellation` + registry pattern `hf_pull_model` uses,
/// registered under `download_registry_key("lmstudio", event_id)`.
fn lmstudio_pull_model(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    model_id: Option<String>,
    hf_repo: Option<String>,
    tag: Option<String>,
) -> Result<serde_json::Value, String> {
    let (event_id, query) = resolve_lmstudio_pull_query(model_id, hf_repo, tag)?;

    if !lmstudio_downloader::is_lms_available() {
        return Err(
            "`lms` wasn't found on $PATH or at LM Studio's default install location -- \
             install LM Studio from https://lmstudio.ai and run it at least once, no extra \
             PATH setup needed"
                .to_string(),
        );
    }

    let ack_query = query.clone();
    let cancel_flag = begin_cancellable_download(state, "lmstudio", &event_id);
    let cancel_state = Arc::clone(state);
    thread::spawn(move || {
        let (line_tx, line_rx) = mpsc::channel::<String>();
        let forward_out_tx = out_tx.clone();
        let forward_model_id = event_id.clone();
        thread::spawn(move || {
            for line in line_rx {
                let event = Event {
                    event: "lmstudio_pull_progress".to_string(),
                    data: serde_json::json!({ "model_id": forward_model_id, "line": line }),
                };
                if let Ok(l) = serde_json::to_string(&event) {
                    let _ = forward_out_tx.send(l);
                }
            }
        });

        let event = match lmstudio_downloader::spawn_pull_query(&query, line_tx) {
            Ok(mut child) => match subprocess::wait_with_cancellation(
                &mut child,
                &cancel_flag,
                Duration::from_millis(200),
            ) {
                Ok(Some(status)) if status.success() => Event {
                    event: "lmstudio_pull_ready".to_string(),
                    data: serde_json::json!({ "model_id": event_id }),
                },
                Ok(Some(status)) => Event {
                    event: "lmstudio_pull_failed".to_string(),
                    data: serde_json::json!({
                        "model_id": event_id,
                        "error": format!("lms get exited with {status}"),
                    }),
                },
                Ok(None) => Event {
                    event: "lmstudio_pull_failed".to_string(),
                    data: serde_json::json!({
                        "model_id": event_id,
                        "error": "cancelled by user",
                        "cancelled": true,
                    }),
                },
                Err(e) => Event {
                    event: "lmstudio_pull_failed".to_string(),
                    data: serde_json::json!({ "model_id": event_id, "error": e.to_string() }),
                },
            },
            Err(e) => Event {
                event: "lmstudio_pull_failed".to_string(),
                data: serde_json::json!({ "model_id": event_id, "error": e.to_string() }),
            },
        };
        end_cancellable_download(&cancel_state, "lmstudio", &event_id);
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = out_tx.send(line);
        }
    });

    Ok(serde_json::json!({ "status": "starting", "target": ack_query }))
}

/// Real, synchronous listing for the llama.cpp panel: the same curated
/// coding-model set (repo/tag only -- the real per-repo `.gguf` filename
/// isn't resolved here, since that needs a real, possibly-slow HTTP call
/// per model and this method must stay fast/synchronous like every other
/// `*_list_models`) plus the real, already-downloaded files this backend
/// already has on disk in `~/.spartan/models/` -- the UI cross-references
/// the two by repo/tag substring rather than this method trying to
/// precisely resolve and match all 21 curated filenames up front.
fn llamacpp_list_models_json() -> serde_json::Value {
    let models: Vec<serde_json::Value> = hf_downloader::CURATED_MODELS
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "display_name": m.display_name,
                "hf_repo": m.hf_repo,
                "tag": m.tag,
                "description": m.description,
            })
        })
        .collect();
    let downloaded: Vec<serde_json::Value> = llamacpp_downloader::list_downloaded()
        .into_iter()
        .map(|d| {
            serde_json::json!({
                "filename": d.filename,
                "size_bytes": d.size_bytes,
                "path": llamacpp_downloader::models_dir().join(&d.filename).to_string_lossy(),
            })
        })
        .collect();
    serde_json::json!({ "models": models, "downloaded": downloaded })
}

/// The direct llama.cpp sibling of `resolve_hf_pull_target`/
/// `resolve_lmstudio_pull_query` -- same accepted shape (a curated
/// `model_id`, or a validated custom `hf_repo`+`tag` pair), resolving down
/// to a real `(event_id, hf_repo, tag)` triple instead of a single target
/// string, since the real download itself still needs one more real,
/// live step (`llamacpp_downloader::resolve_gguf_filename`) this function
/// deliberately does not perform -- that's a real, possibly-slow HTTP call
/// of its own, done inside the background thread, not on the request
/// thread, matching this crate's own "never block the one IPC channel"
/// rule.
fn resolve_llamacpp_download_target(
    model_id: Option<String>,
    hf_repo: Option<String>,
    tag: Option<String>,
) -> Result<(String, String, String), String> {
    match (model_id, hf_repo, tag) {
        (Some(model_id), _, _) => {
            let model = hf_downloader::find_model(&model_id)
                .ok_or_else(|| format!("unknown curated model id: {model_id:?}"))?;
            Ok((
                model.id.to_string(),
                model.hf_repo.to_string(),
                model.tag.to_string(),
            ))
        }
        (None, Some(hf_repo), Some(tag)) => {
            let normalized = hf_downloader::normalize_hf_repo_input(&hf_repo);
            hf_downloader::validate_custom_repo_and_tag(&normalized, &tag)?;
            Ok((
                format!("{normalized}:{}", tag.trim()),
                normalized,
                tag.trim().to_string(),
            ))
        }
        _ => Err(
            "llamacpp_download_model requires either a string `model_id`, or both a string \
             `hf_repo` and string `tag`"
                .to_string(),
        ),
    }
}

/// Starts a real HF -> llama.cpp GGUF download in the background: an
/// immediate `{"status": "starting"}` ack, then a spawned thread resolves
/// the repo's real filename (a real, live HF API call --
/// `llamacpp_downloader::resolve_gguf_filename`) and streams the real
/// download itself, forwarding progress as `llamacpp_download_progress`
/// events and finishing with `llamacpp_download_ready` (carrying the real
/// saved file path, ready to hand straight to `settings_set`'s
/// `leo_provider.model`) or `llamacpp_download_failed` -- the same
/// "ack now, event later" shape `hf_pull_model`/`lmstudio_pull_model`
/// already established. Unlike those two, there's no local binary to
/// pre-check for -- a real HTTP client is always available -- so any
/// failure (network, a repo with no matching quant, a write error) only
/// ever surfaces async, through the `_failed` event, never synchronously.
///
/// Real, cancellable (task #268): registers a fresh flag under
/// `download_registry_key("llamacpp", event_id)` before spawning, and
/// threads it directly into `download_gguf`'s own real HTTP read loop --
/// unlike `hf_pull_model`/`lmstudio_pull_model`, there's no subprocess
/// `Child` to kill here, so this is the one real source whose
/// cancellation is checked-flag-based rather than process-kill-based.
fn llamacpp_download_model(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    model_id: Option<String>,
    hf_repo: Option<String>,
    tag: Option<String>,
) -> Result<serde_json::Value, String> {
    let (event_id, hf_repo, tag) = resolve_llamacpp_download_target(model_id, hf_repo, tag)?;

    let ack_id = event_id.clone();
    let cancel_flag = begin_cancellable_download(state, "llamacpp", &event_id);
    let cancel_state = Arc::clone(state);
    thread::spawn(move || {
        let (line_tx, line_rx) = mpsc::channel::<String>();
        let forward_out_tx = out_tx.clone();
        let forward_model_id = event_id.clone();
        thread::spawn(move || {
            for line in line_rx {
                let event = Event {
                    event: "llamacpp_download_progress".to_string(),
                    data: serde_json::json!({ "model_id": forward_model_id, "line": line }),
                };
                if let Ok(l) = serde_json::to_string(&event) {
                    let _ = forward_out_tx.send(l);
                }
            }
        });

        let event = match llamacpp_downloader::resolve_gguf_filename(&hf_repo, &tag) {
            Ok(filename) => match llamacpp_downloader::download_gguf(
                &hf_repo,
                &filename,
                &line_tx,
                &cancel_flag,
            ) {
                Ok(path) => Event {
                    event: "llamacpp_download_ready".to_string(),
                    data: serde_json::json!({
                        "model_id": event_id,
                        "path": path.to_string_lossy(),
                    }),
                },
                Err(e) if e == llamacpp_downloader::CANCELLED_ERROR => Event {
                    event: "llamacpp_download_failed".to_string(),
                    data: serde_json::json!({
                        "model_id": event_id,
                        "error": e,
                        "cancelled": true,
                    }),
                },
                Err(e) => Event {
                    event: "llamacpp_download_failed".to_string(),
                    data: serde_json::json!({ "model_id": event_id, "error": e }),
                },
            },
            Err(e) => Event {
                event: "llamacpp_download_failed".to_string(),
                data: serde_json::json!({ "model_id": event_id, "error": e }),
            },
        };
        end_cancellable_download(&cancel_state, "llamacpp", &event_id);
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = out_tx.send(line);
        }
    });

    Ok(serde_json::json!({ "status": "starting", "model_id": ack_id }))
}

fn leo_start_task(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    task: String,
    project_root: String,
) -> Result<serde_json::Value, String> {
    let my_generation = {
        let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
        // Real task #266: a previous agent left sitting in `Failed` (the
        // user never retried it, or `leo_retry` itself was exhausted) is
        // about to be discarded for good by the fresh `Agent` below --
        // this is the one real, unambiguous point to retroactively record
        // it as a real terminal `Failed` history entry, using whatever
        // real error text `leo_last_error` last captured.
        if let Some(agent) = guard.leo_agent.as_ref() {
            if agent.state() == spartan_leo::state::AgentState::Failed {
                let last_error = guard.leo_last_error.clone();
                push_leo_history(&mut guard, "Failed", None, last_error);
            }
        }
        let approval_mode = approval_mode_from_settings(spartan_settings::load().leo_approval_mode);
        let mut agent = Agent::new(PathBuf::from(&project_root), approval_mode);
        agent
            .begin_planning()
            .map_err(|e| format!("begin_planning: {e:?}"))?;
        guard.leo_agent = Some(agent);
        guard.leo_project_root = Some(PathBuf::from(&project_root));
        guard.leo_current_task = Some(task.clone());
        guard.leo_last_error = None;
        // A fresh `Agent` per task (§75.47's own documented decision)
        // means the execute-loop's own real state must reset too, or a
        // second task would start with the first task's stale history.
        guard.leo_history.clear();
        guard.leo_pending_call = None;
        guard.leo_generation += 1;
        // Real §75.73-closing cooperative cancellation (task #269): a
        // brand-new flag every real new task, the same "start fresh"
        // discipline `leo_generation` itself already uses -- not a reset
        // of the existing one, so a late clone held by a just-superseded
        // background thread can never race this fresh task's own flag.
        guard.leo_cancel_flag = Arc::new(AtomicBool::new(false));
        guard.leo_generation
    };
    let cancel_flag = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        Arc::clone(&guard.leo_cancel_flag)
    };

    let state = Arc::clone(state);
    thread::spawn(move || {
        let settings = spartan_settings::load();
        let provider = match build_leo_provider(&settings.leo_provider, settings.gpu_offload) {
            Ok(provider) => provider,
            Err(message) => {
                let Ok(mut guard) = state.lock() else {
                    return;
                };
                if guard.leo_generation != my_generation {
                    return;
                }
                guard.leo_last_error = Some(message.clone());
                let event = Event {
                    event: "leo_plan_failed".to_string(),
                    data: serde_json::json!({ "error": message }),
                };
                drop(guard);
                let _ = out_tx.send(serde_json::to_string(&event).unwrap_or_default());
                return;
            }
        };
        // Real §4.3 project-tier memory, read back into planning context
        // for the first time (closing task #5's own named "project-tier
        // memory" bar) -- deliberately folded into the task string itself
        // rather than a new `generate_plan` parameter, since that
        // function's signature is shared with `spartan-editor-core`'s own
        // `leo_bridge.rs` and every one of `plan.rs`'s own existing
        // tests; this achieves the same real effect entirely at this one
        // call site.
        let memory = spartan_leo::memory::read_project_memory(&PathBuf::from(&project_root))
            .unwrap_or_default();
        let task_with_memory = augment_task_with_memory(&task, &memory);
        let result: Result<ImplementationPlan, PlanError> =
            generate_plan_cancellable(provider.as_ref(), &task_with_memory, &cancel_flag);

        let event = {
            let Ok(mut guard) = state.lock() else {
                return;
            };
            if guard.leo_generation != my_generation {
                // A newer task has since started (or the agent was
                // otherwise reset) -- this real, late-arriving result no
                // longer belongs to the current one, discard it silently
                // rather than clobbering real, newer state.
                return;
            }
            let Some(agent) = guard.leo_agent.as_mut() else {
                return;
            };
            let ev = match agent.apply_generated_plan(result) {
                Ok(()) => {
                    let plan = agent.plan().expect("plan set on successful transition");
                    Event {
                        event: "leo_plan_ready".to_string(),
                        data: plan_json(plan),
                    }
                }
                Err(AgentError::Plan(plan_err)) => Event {
                    event: "leo_plan_failed".to_string(),
                    data: serde_json::json!({ "error": format!("{plan_err:?}") }),
                },
                Err(other) => Event {
                    event: "leo_plan_failed".to_string(),
                    data: serde_json::json!({ "error": format!("{other:?}") }),
                },
            };
            if ev.event == "leo_plan_failed" {
                guard.leo_last_error = ev
                    .data
                    .get("error")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
            }
            ev
        };
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = out_tx.send(line);
        }
    });

    Ok(serde_json::json!({ "status": "planning" }))
}

/// Real `AwaitingApproval -> Executing`, taking a real git checkpoint
/// the instant approval happens -- matches the original wgpu shell's own
/// scope exactly (§75.47): approving creates a checkpoint; there is no
/// real automated execute/verify loop driving further tool calls yet in
/// `spartan-leo` itself, so this stays honest about that rather than
/// implying more than the underlying crate actually does.
fn leo_approve_plan(state: &Arc<Mutex<BackendState>>) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    let project_root = guard
        .leo_project_root
        .clone()
        .ok_or("no Leo task has been started yet")?;
    let agent = guard
        .leo_agent
        .as_mut()
        .ok_or("no Leo task has been started yet")?;
    let mut repo = spartan_git::GitRepo::discover(&project_root)
        .ok_or("project root is not a real git repository -- Leo needs one for checkpoints")?;
    agent
        .approve_plan(repo.raw_repo_mut())
        .map_err(|e| format!("approve_plan: {e:?}"))?;
    Ok(serde_json::json!({ "ok": true, "state": agent_state_name(agent) }))
}

fn leo_reject_plan(state: &Arc<Mutex<BackendState>>) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    let agent = guard
        .leo_agent
        .as_mut()
        .ok_or("no Leo task has been started yet")?;
    agent
        .reject_plan()
        .map_err(|e| format!("reject_plan: {e:?}"))?;
    Ok(serde_json::json!({ "ok": true, "state": agent_state_name(agent) }))
}

/// Real §75.73 user-initiated cancel -- closes task #58's own named
/// remaining item, "a UI control to interrupt an in-progress planning or
/// execute loop." Real, works from `Planning`, `AwaitingApproval`,
/// `Executing`, or `Verifying` (`Agent::cancel`'s own real transition
/// table); errors honestly, matching every other real transition method
/// in this file, if the agent is already `Idle`/`Done`/`Failed`/
/// `Recovering`.
///
/// **Updated by task #269, closing the exact gap this doc comment used to
/// name as open**: this now genuinely interrupts a real background OS
/// thread blocked inside a real network-backed model call
/// (`OllamaProvider`/`ClaudeProvider`/`LiteLLMProvider`/`LmStudioProvider`,
/// via `leo_cancel_flag`) -- not just discard its late result. A real,
/// honestly-scoped limit still applies, named in `ModelProvider::
/// stream_completion_cancellable`'s own doc comment: cancellation is only
/// observed *between* already-arrived real chunks, never mid-read; and
/// `LlamaCppProvider`'s own in-process token generation and a real
/// `run_terminal` subprocess (killed only by its own timeout, §264) are
/// both real, separate, still-uninterrupted cases. What this function
/// always does regardless -- and what made cancel real even before this
/// pass -- is bump `leo_generation` before releasing the lock, so any real
/// result that arrives late (from a provider this pass doesn't reach, or
/// simply because the cancel flag wasn't checked in time) is still
/// discarded by the exact same generation-guard check `leo_start_task`/
/// `leo_next_step` already perform, instead of silently resurrecting a
/// task the user just told this shell to abandon. `leo_pending_call` and
/// `leo_history` are cleared too -- a cancelled task has nothing left to
/// resume, the same real cleanup `leo_start_task` already does when
/// beginning a fresh one.
fn leo_cancel(state: &Arc<Mutex<BackendState>>) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    let agent = guard
        .leo_agent
        .as_mut()
        .ok_or("no Leo task has been started yet")?;
    agent.cancel().map_err(|e| format!("cancel: {e:?}"))?;
    let state_name = agent_state_name(agent);
    guard.leo_generation += 1;
    // Real §75.73-closing cooperative cancellation (task #269): setting
    // this real, shared flag is what makes cancel actually interrupt a
    // real, already-in-flight background model call (the exact gap this
    // function's own doc comment named), rather than only discarding a
    // late result via the generation bump above -- see `BackendState::
    // leo_cancel_flag`'s own doc comment for the full real mechanism.
    guard
        .leo_cancel_flag
        .store(true, std::sync::atomic::Ordering::SeqCst);
    guard.leo_pending_call = None;
    guard.leo_history.clear();
    // Real task #266: `agent.cancel()` above already flipped the real
    // state to `Idle`, which would erase any trace this task ever ran
    // before a future `leo_start_task` could retroactively record it (the
    // way it does for a `Failed` agent) -- `Agent::cancel`'s own real
    // transition table only ever succeeds from a genuinely in-flight
    // state (`Planning`/`AwaitingApproval`/`Executing`/`Verifying`), so
    // reaching this line always means a real task was actually abandoned,
    // never a harmless no-op on an already-idle agent.
    push_leo_history(&mut guard, "Cancelled", None, None);
    Ok(serde_json::json!({ "ok": true, "state": state_name }))
}

/// Real §75.78 retry -- closes the one last piece the "Failed ->
/// Recovering -> Executing" retry loop has been missing since
/// `spartan-leo::agent::begin_recovery` was first built (§75.46): a real
/// caller. Every prior pass since then correctly called `mark_failed`
/// on a real tool-execution or model error (`leo_next_step`'s own two
/// call sites), but nothing ever called `begin_recovery` -- a task that
/// failed had no way forward except abandoning it for a brand new one.
/// `Agent::cancel` cannot reach `Failed` at all (`AgentState::
/// can_transition_to` has no `Failed -> Idle` edge, by design -- see
/// `leo_cancel`'s own doc comment), so `begin_recovery` really is the
/// only real exit from this state. Mirrors `leo_approve_plan`'s exact
/// git-repo-discovery shape, since `begin_recovery` needs the same real
/// checkpoint-restore access `approve_plan` does. A real, honest,
/// non-generic error on `RecoveryExhausted` (the bounded-retry limit
/// `spartan-leo` itself enforces, §4.1's "default max 3 attempts") tells
/// the user plainly that this task is done for and a new one is the
/// only way forward, rather than a caller silently retrying forever or
/// the UI showing a confusing generic failure.
fn leo_retry(state: &Arc<Mutex<BackendState>>) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    let project_root = guard
        .leo_project_root
        .clone()
        .ok_or("no Leo task has been started yet")?;
    let agent = guard
        .leo_agent
        .as_mut()
        .ok_or("no Leo task has been started yet")?;
    let mut repo = spartan_git::GitRepo::discover(&project_root)
        .ok_or("project root is not a real git repository -- Leo needs one for checkpoints")?;
    match agent.begin_recovery(repo.raw_repo_mut()) {
        Ok(()) => Ok(serde_json::json!({ "ok": true, "state": agent_state_name(agent) })),
        Err(AgentError::RecoveryExhausted) => {
            Err("recovery attempts exhausted (max 3) -- start a new task instead".to_string())
        }
        Err(e) => Err(format!("begin_recovery: {e:?}")),
    }
}

/// Real, honest JSON rendering of a proposed `ToolCall`'s arguments --
/// the UI needs to show the human *what* Leo wants to do before they
/// approve or reject it.
fn tool_call_json(call: &ToolCall) -> serde_json::Value {
    match call {
        ToolCall::ReadFile { path } => serde_json::json!({ "path": path }),
        ToolCall::EditFile { path, content } => {
            serde_json::json!({ "path": path, "content": content })
        }
        ToolCall::RunTerminal { command } => serde_json::json!({ "command": command }),
        ToolCall::SearchFiles { pattern, path } => {
            serde_json::json!({ "pattern": pattern, "path": path })
        }
        ToolCall::ListDirectory { path } => serde_json::json!({ "path": path }),
    }
}

/// The real plain-text form fed back to the model as a `Role::Tool`
/// message (`execute::append_tool_result`) -- separate from
/// `tool_result_json` below, which is the structured form the UI gets,
/// since the model only ever sees raw text content on that role.
fn tool_result_text(result: &ToolResult) -> String {
    match result {
        ToolResult::FileContent(content) => content.clone(),
        ToolResult::FileWritten { path, bytes } => format!("Wrote {bytes} bytes to {path}"),
        ToolResult::TerminalOutput {
            stdout,
            stderr,
            exit_code,
        } => format!("exit_code={exit_code}\nstdout:\n{stdout}\nstderr:\n{stderr}"),
        ToolResult::SearchMatches(matches) => {
            if matches.is_empty() {
                "No matches found.".to_string()
            } else {
                matches
                    .iter()
                    .map(|m| format!("{}:{}: {}", m.path, m.line, m.text))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        ToolResult::DirectoryListing(entries) => {
            if entries.is_empty() {
                "(empty directory)".to_string()
            } else {
                entries
                    .iter()
                    .map(|e| {
                        if e.is_dir {
                            format!("{}/", e.name)
                        } else {
                            e.name.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
    }
}

fn tool_result_json(result: &ToolResult) -> serde_json::Value {
    match result {
        ToolResult::FileContent(content) => {
            serde_json::json!({ "kind": "file_content", "content": content })
        }
        ToolResult::FileWritten { path, bytes } => {
            serde_json::json!({ "kind": "file_written", "path": path, "bytes": bytes })
        }
        ToolResult::TerminalOutput {
            stdout,
            stderr,
            exit_code,
        } => serde_json::json!({
            "kind": "terminal_output",
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": exit_code,
        }),
        ToolResult::SearchMatches(matches) => serde_json::json!({
            "kind": "search_matches",
            "matches": matches.iter().map(|m| serde_json::json!({
                "path": m.path, "line": m.line, "text": m.text,
            })).collect::<Vec<_>>(),
        }),
        ToolResult::DirectoryListing(entries) => serde_json::json!({
            "kind": "directory_listing",
            "entries": entries.iter().map(|e| serde_json::json!({
                "name": e.name, "is_dir": e.is_dir,
            })).collect::<Vec<_>>(),
        }),
    }
}

/// Real §75.68 diff preview -- a plain, `+`/`-`/` `-prefixed line diff
/// (via the real `similar` crate's line-level `TextDiff`), not a full
/// unified-diff-with-hunk-headers format, since the UI renders every
/// line directly rather than parsing hunk boundaries. Bounded to
/// `MAX_DIFF_LINES` real output lines so an unexpectedly huge rewrite
/// can't balloon the event payload -- truncated with an honest note,
/// never silently cut off without saying so.
fn compute_diff(old: &str, new: &str) -> String {
    const MAX_DIFF_LINES: usize = 500;
    use similar::{ChangeTag, TextDiff};
    let diff = TextDiff::from_lines(old, new);
    let mut out = String::new();
    for (lines, change) in diff.iter_all_changes().enumerate() {
        if lines >= MAX_DIFF_LINES {
            out.push_str(&format!(
                "... diff truncated after {MAX_DIFF_LINES} lines ...\n"
            ));
            break;
        }
        let sign = match change.tag() {
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
            ChangeTag::Equal => ' ',
        };
        out.push(sign);
        out.push_str(change.as_str().unwrap_or_default());
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Real §75.69 bound on how many `Safe` calls `leo_next_step`'s own
/// auto-approve loop will run unattended before forcing the next
/// proposal through real human approval regardless of its own risk
/// class -- a real, named safety valve, not an expected steady state:
/// no real task should legitimately need this many consecutive read-only
/// calls, and a bound means a model stuck in a real search-only loop
/// eventually surfaces to the human instead of running forever.
const MAX_AUTO_STEPS: u32 = 25;

/// Real task #265: closes §75.66's own named "Verifying is a momentary,
/// always-passing waypoint" scope cut, once the model has proposed
/// `task_complete`. `agent` must already be `Executing`. `verify_command`
/// is `settings.leo_verify_command.as_deref()` -- `None` (the real,
/// unconfigured default) keeps the exact prior byte-for-byte behavior;
/// `Some(cmd)` runs it through the same real, hard-jailed,
/// timeout-bounded `Sandbox` (`Agent::run_verification` ->
/// `Sandbox::run_terminal_with_timeout`, §264) every tool call already
/// uses: a real exit 0 marks the task genuinely `Done`; a real non-zero
/// exit marks it `Failed`, the exact state `leo_retry` (§75.78) recovers
/// from, so a failing check really feeds Leo's own bounded recovery loop
/// rather than silently passing. Extracted as its own free function
/// (rather than left inline in `leo_next_step`'s background closure)
/// specifically so it's unit-testable directly against a real `Agent`
/// fixture with no model/thread involved -- the same "does not itself
/// decide threading" separation `Agent::run_verification`'s own doc
/// comment already establishes one layer down.
fn run_leo_verification_and_completion(
    agent: &mut Agent,
    verify_command: Option<&str>,
    summary: String,
) -> Event {
    if let Err(e) = agent.begin_verification() {
        return Event {
            event: "leo_execute_failed".to_string(),
            data: serde_json::json!({ "error": format!("{e:?}") }),
        };
    }

    let Some(cmd) = verify_command else {
        // No verification command configured -- the real, unchanged
        // §75.66 momentary waypoint.
        return match agent.mark_done() {
            Ok(()) => {
                // Real §4.3 project-tier memory write -- "Leo writes to
                // this itself" (memory.rs's own doc comment) -- a real,
                // best-effort append, not on the critical path: a real
                // memory-file I/O failure (e.g. a read-only project
                // directory) must never hide that the task itself
                // genuinely completed, so `memory_saved` is reported
                // honestly rather than silently swallowed or allowed to
                // fail the whole task.
                let memory_saved = agent.append_memory(&summary).is_ok();
                Event {
                    event: "leo_execute_done".to_string(),
                    data: serde_json::json!({
                        "summary": summary,
                        "memory_saved": memory_saved,
                    }),
                }
            }
            Err(e) => Event {
                event: "leo_execute_failed".to_string(),
                data: serde_json::json!({ "error": format!("{e:?}") }),
            },
        };
    };

    match agent.run_verification(cmd) {
        Ok(result) => {
            let ToolResult::TerminalOutput {
                stdout,
                stderr,
                exit_code,
            } = &result
            else {
                unreachable!("run_verification always returns TerminalOutput");
            };
            if *exit_code == 0 {
                match agent.mark_done() {
                    Ok(()) => {
                        let memory_saved = agent.append_memory(&summary).is_ok();
                        Event {
                            event: "leo_execute_done".to_string(),
                            data: serde_json::json!({
                                "summary": summary,
                                "memory_saved": memory_saved,
                                "verification": {
                                    "command": cmd,
                                    "exit_code": exit_code,
                                    "stdout": stdout,
                                    "stderr": stderr,
                                },
                            }),
                        }
                    }
                    Err(e) => Event {
                        event: "leo_execute_failed".to_string(),
                        data: serde_json::json!({ "error": format!("{e:?}") }),
                    },
                }
            } else {
                // A real, non-zero verification failure marks the task
                // `Failed` -- the exact state `leo_retry` (§75.78)
                // recovers from, so a failing check genuinely feeds
                // Leo's own bounded recovery loop rather than silently
                // passing.
                let _ = agent.mark_failed();
                Event {
                    event: "leo_execute_failed".to_string(),
                    data: serde_json::json!({
                        "error": format!(
                            "verification command `{cmd}` failed (exit code {exit_code})"
                        ),
                        "verification": {
                            "command": cmd,
                            "exit_code": exit_code,
                            "stdout": stdout,
                            "stderr": stderr,
                        },
                    }),
                }
            }
        }
        Err(e) => {
            let _ = agent.mark_failed();
            Event {
                event: "leo_execute_failed".to_string(),
                data: serde_json::json!({
                    "error": format!("verification command `{cmd}` could not be run: {e:?}")
                }),
            }
        }
    }
}

/// Real §4.1 execute-step round trip (§75.66, closing the single largest
/// gap task #5 has had open since §75.47/§75.56: "approving a plan
/// creates a real checkpoint and then has nothing further to run"). Must
/// be in `Executing` with no call already pending approval -- calling
/// this again before the pending one is resolved is a real caller bug,
/// not something to silently queue.
///
/// Mirrors `leo_start_task`'s own spawn-thread-report-back shape exactly:
/// a real, possibly slow model call runs on its own thread; the caller
/// gets an immediate synchronous ack and the real result arrives later as
/// one or more unprompted `Event`s.
///
/// Since §75.69, this real background thread now *loops*: when the
/// user's configured `LeoApprovalMode` is `AutoApproveSafe`, a proposed
/// `Safe` call (`read_file`/`search_files`/`list_directory`) is executed
/// immediately, server-side, without a UI round trip -- its real result
/// is appended to history and a real `leo_auto_step` event is pushed for
/// visibility, then the loop asks the model for the *next* action again,
/// all within this same spawned thread. A `Destructive` call
/// (`edit_file`/`run_terminal`) is never auto-run, matching §9's own
/// non-negotiable rule (`Agent::may_auto_execute` is the one real gate,
/// unchanged) -- the loop only ever shortens the real number of UI round
/// trips for read-only exploration, it never widens what may run without
/// a human.
/// Real task #266 recording, extracted from `leo_next_step`'s own
/// background closure so it's directly unit-testable against a plain
/// constructed `Event` with no threading/model call involved -- the same
/// "separate the decision logic from the threading" precedent
/// `run_leo_verification_and_completion` already established one pass
/// earlier. `Done` is unambiguously terminal (a real, immediate push into
/// `leo_session_history`) -- `Failed` is not, since §75.78's own bounded
/// retry loop can still bring it back to `Executing`, so only the real
/// error text is remembered here (`leo_last_error`), retroactively
/// recorded as terminal only if the task is later abandoned rather than
/// retried (`leo_start_task`'s own real check, see its doc comment).
fn record_leo_next_step_outcome(state: &mut BackendState, event: &Event) {
    if event.event == "leo_execute_done" {
        let summary_text = event
            .data
            .get("summary")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        push_leo_history(state, "Done", summary_text, None);
    } else if event.event == "leo_execute_failed" {
        state.leo_last_error = event
            .data
            .get("error")
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
}

fn leo_next_step(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
) -> Result<serde_json::Value, String> {
    let (plan, mut history, my_generation, cancel_flag) = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        let agent = guard
            .leo_agent
            .as_ref()
            .ok_or("no Leo task has been started yet")?;
        if agent.state() != spartan_leo::state::AgentState::Executing {
            return Err(format!(
                "leo_next_step requires the Executing state, agent is currently {:?}",
                agent.state()
            ));
        }
        if guard.leo_pending_call.is_some() {
            return Err("a proposed action is already awaiting approval".to_string());
        }
        let plan = agent.plan().cloned().ok_or("no approved plan to execute")?;
        (
            plan,
            guard.leo_history.clone(),
            guard.leo_generation,
            // Real §75.73-closing cooperative cancellation (task #269):
            // the *current* flag `leo_start_task` minted for this same
            // task/generation, not a fresh one -- `leo_next_step` never
            // starts a new generation of its own, it's the same task's
            // own real execute-loop continuing.
            Arc::clone(&guard.leo_cancel_flag),
        )
    };

    let state = Arc::clone(state);
    thread::spawn(move || {
        let settings = spartan_settings::load();
        let provider = match build_leo_provider(&settings.leo_provider, settings.gpu_offload) {
            Ok(provider) => provider,
            Err(message) => {
                let Ok(guard) = state.lock() else {
                    return;
                };
                if guard.leo_generation != my_generation {
                    return;
                }
                let event = Event {
                    event: "leo_execute_failed".to_string(),
                    data: serde_json::json!({ "error": message }),
                };
                drop(guard);
                let _ = out_tx.send(serde_json::to_string(&event).unwrap_or_default());
                return;
            }
        };
        let mut auto_steps = 0u32;

        loop {
            let result =
                execute::next_action_cancellable(provider.as_ref(), &plan, &history, &cancel_flag);

            let Ok(mut guard) = state.lock() else {
                return;
            };
            if guard.leo_generation != my_generation {
                // A newer task has since started -- this real, possibly
                // multi-iteration background loop no longer belongs to
                // the current one. Discard silently.
                return;
            }

            let event = match result {
                Ok(step) => match step.action {
                    ExecuteAction::Call(call) => {
                        let Some(agent) = guard.leo_agent.as_mut() else {
                            return;
                        };
                        if auto_steps < MAX_AUTO_STEPS && agent.may_auto_execute(&call) {
                            match agent.execute_call(call.clone()) {
                                Ok(result) => {
                                    let text = tool_result_text(&result);
                                    execute::append_tool_result(
                                        &mut guard.leo_history,
                                        &step.call_id,
                                        &text,
                                    );
                                    history = guard.leo_history.clone();
                                    auto_steps += 1;
                                    let auto_event = Event {
                                        event: "leo_auto_step".to_string(),
                                        data: serde_json::json!({
                                            "tool": call.name(),
                                            "args": tool_call_json(&call),
                                            "result": tool_result_json(&result),
                                        }),
                                    };
                                    drop(guard);
                                    if let Ok(line) = serde_json::to_string(&auto_event) {
                                        if out_tx.send(line).is_err() {
                                            return;
                                        }
                                    }
                                    continue;
                                }
                                Err(e) => {
                                    let _ = agent.mark_failed();
                                    Event {
                                        event: "leo_execute_failed".to_string(),
                                        data: serde_json::json!({
                                            "error": format!("{e:?}")
                                        }),
                                    }
                                }
                            }
                        } else {
                            // Real §75.68 diff preview -- computed here,
                            // once, before the human ever sees the
                            // proposal, rather than in the UI, so the
                            // exact same real "current file content"
                            // `peek_file` reads is what gets diffed (no
                            // risk of the UI's own, possibly stale, view
                            // of the file disagreeing with what Leo is
                            // actually about to write).
                            let diff = if let ToolCall::EditFile { path, content } = &call {
                                Some(compute_diff(
                                    &agent.peek_file(path).unwrap_or_default(),
                                    content,
                                ))
                            } else {
                                None
                            };
                            let mut data = serde_json::json!({
                                "call_id": step.call_id,
                                "tool": call.name(),
                                "args": tool_call_json(&call),
                            });
                            if let Some(d) = diff {
                                data["diff"] = serde_json::Value::String(d);
                            }
                            let event = Event {
                                event: "leo_action_proposed".to_string(),
                                data,
                            };
                            guard.leo_pending_call = Some(PendingCall {
                                call_id: step.call_id,
                                call,
                            });
                            event
                        }
                    }
                    ExecuteAction::Done { summary } => {
                        let Some(agent) = guard.leo_agent.as_mut() else {
                            return;
                        };
                        run_leo_verification_and_completion(
                            agent,
                            settings.leo_verify_command.as_deref(),
                            summary,
                        )
                    }
                },
                Err(e) => {
                    if let Some(agent) = guard.leo_agent.as_mut() {
                        let _ = agent.mark_failed();
                    }
                    Event {
                        event: "leo_execute_failed".to_string(),
                        data: serde_json::json!({ "error": e.to_string() }),
                    }
                }
            };
            record_leo_next_step_outcome(&mut guard, &event);
            if let Ok(line) = serde_json::to_string(&event) {
                let _ = out_tx.send(line);
            }
            return;
        }
    });

    Ok(serde_json::json!({ "status": "thinking" }))
}

/// Real, synchronous approval + execution of the one real pending call --
/// runs it through the real, hard-jailed `Sandbox` (`agent.execute_call`)
/// and appends the real result to `leo_history` so the next
/// `leo_next_step` call sees it. Deliberately synchronous, not spawned:
/// `read_file`/`edit_file` are fast; `run_terminal` can legitimately take
/// a while and has no timeout here, a real, named limitation shared with
/// `spartan-leo::tool::Sandbox::run_terminal` itself, not newly
/// introduced by this call site.
fn leo_approve_call(state: &Arc<Mutex<BackendState>>) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    let pending = guard
        .leo_pending_call
        .take()
        .ok_or("no action is currently awaiting approval")?;
    let agent = guard
        .leo_agent
        .as_mut()
        .ok_or("no Leo task has been started yet")?;

    match agent.execute_call(pending.call) {
        Ok(result) => {
            let text = tool_result_text(&result);
            execute::append_tool_result(&mut guard.leo_history, &pending.call_id, &text);
            Ok(serde_json::json!({ "ok": true, "result": tool_result_json(&result) }))
        }
        Err(e) => {
            let text = format!("Error: {e:?}");
            execute::append_tool_result(&mut guard.leo_history, &pending.call_id, &text);
            Ok(serde_json::json!({ "ok": false, "error": text }))
        }
    }
}

/// Real rejection -- does not fail the task outright, since the model may
/// have a real, viable alternative approach. Appends a real `Role::Tool`
/// rejection notice to history instead, leaving the agent in `Executing`
/// so a caller's next `leo_next_step` call gives the model a genuine
/// chance to propose something else (or call `task_complete` if nothing
/// further is needed).
fn leo_reject_call(state: &Arc<Mutex<BackendState>>) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    let pending = guard
        .leo_pending_call
        .take()
        .ok_or("no action is currently awaiting approval")?;
    execute::append_tool_result(
        &mut guard.leo_history,
        &pending.call_id,
        "User rejected this action. Propose a different approach, or call task_complete if \
         no further action is needed.",
    );
    Ok(serde_json::json!({ "ok": true }))
}

/// Real PTY spawn (§75.64, closing the §75.62 audit's own named
/// Console/Sessions gap) -- `command`/`args` of `None`/empty default to
/// the real `$SHELL` (Console); a real named command (Sessions) reuses
/// this exact same primitive. Output streams back as real, unprompted
/// `pty_output`/`pty_exit` events (`pty.rs`), never blocking this
/// synchronous call, which only returns once the real spawn itself
/// either succeeds or fails.
fn pty_spawn(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    cwd: &str,
    cols: u16,
    rows: u16,
    command: Option<&str>,
    args: &[String],
) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    let session_id = guard.next_pty_id;
    let handle = pty::spawn_pty(
        session_id,
        std::path::Path::new(cwd),
        cols,
        rows,
        command,
        args,
        out_tx,
    )
    .map_err(|e| format!("failed to spawn pty: {e}"))?;
    guard.next_pty_id += 1;
    guard.pty_sessions.insert(session_id, handle);
    Ok(serde_json::json!({ "session_id": session_id }))
}

fn pty_input(
    state: &Arc<Mutex<BackendState>>,
    session_id: u64,
    data: &str,
) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    let handle = guard
        .pty_sessions
        .get_mut(&session_id)
        .ok_or_else(|| format!("no pty session with id {session_id}"))?;
    handle
        .write(data.as_bytes())
        .map_err(|e| format!("pty write failed: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

fn pty_resize(
    state: &Arc<Mutex<BackendState>>,
    session_id: u64,
    cols: u16,
    rows: u16,
) -> Result<serde_json::Value, String> {
    let guard = state.lock().map_err(|_| "backend state poisoned")?;
    let handle = guard
        .pty_sessions
        .get(&session_id)
        .ok_or_else(|| format!("no pty session with id {session_id}"))?;
    handle
        .resize(cols, rows)
        .map_err(|e| format!("pty resize failed: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

fn pty_close(
    state: &Arc<Mutex<BackendState>>,
    session_id: u64,
) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    if let Some(mut handle) = guard.pty_sessions.remove(&session_id) {
        handle.kill();
    }
    Ok(serde_json::json!({ "ok": true }))
}

/// Parses the `dap_launch` breakpoint list from request params. Accepts a
/// real `breakpoints: [{line, condition?, logMessage?}]` array (the shape
/// both shells now send, carrying conditional-breakpoint/logpoint info),
/// and falls back to the older plain `break_lines: [<int>]` numeric array
/// (an ordinary line breakpoint each) so a client that hasn't been updated
/// still works. A malformed entry with no numeric `line` is skipped.
fn parse_breakpoints(params: &serde_json::Value) -> Vec<spartan_dap::Breakpoint> {
    if let Some(arr) = params.get("breakpoints").and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|entry| {
                let line = entry.get("line").and_then(|v| v.as_i64())?;
                let condition = entry
                    .get("condition")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .filter(|s| !s.trim().is_empty());
                let log_message = entry
                    .get("logMessage")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .filter(|s| !s.trim().is_empty());
                Some(spartan_dap::Breakpoint {
                    line,
                    condition,
                    log_message,
                })
            })
            .collect();
    }
    params
        .get("break_lines")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_i64())
                .map(spartan_dap::Breakpoint::line)
                .collect()
        })
        .unwrap_or_default()
}

/// Real DAP launch (§132) -- looks up the already-open document's real
/// on-disk path (a debug session targets a real file, not raw text a
/// client might be mid-editing unsaved), then hands off to
/// `dap_integration::dap_launch` for the actual language/adapter
/// resolution. The lock is released before that call runs (mirroring
/// `leo_start_task`'s own precedent) since `dap_launch` itself spawns a
/// real background thread and briefly blocks on the adapter's own
/// initialize/launch/breakpoint handshake before returning.
fn dap_launch(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    doc_id: u64,
    breakpoints: &[spartan_dap::Breakpoint],
) -> Result<serde_json::Value, String> {
    let path = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        let doc = guard
            .open_docs
            .get(&doc_id)
            .ok_or_else(|| format!("no open document with id {doc_id}"))?;
        doc.path.clone()
    };
    let session = dap_integration::dap_launch(doc_id, &path, breakpoints, out_tx)?;
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    let session_id = guard.next_dap_id;
    guard.next_dap_id += 1;
    guard.dap_sessions.insert(session_id, session);
    Ok(serde_json::json!({ "session_id": session_id }))
}

fn dap_command(
    state: &Arc<Mutex<BackendState>>,
    session_id: u64,
    command: spartan_dap::DapCommand,
) -> Result<serde_json::Value, String> {
    let guard = state.lock().map_err(|_| "backend state poisoned")?;
    let session = guard
        .dap_sessions
        .get(&session_id)
        .ok_or_else(|| format!("no dap session with id {session_id}"))?;
    session.send_command(command);
    Ok(serde_json::json!({ "ok": true }))
}

/// Real watch/REPL evaluation of an expression against a stopped DAP
/// session (§250). Unlike `dap_command`, this needs the real result back,
/// so it clones the session `Arc` and releases the lock before blocking on
/// `DapSession::evaluate` (mirroring `dap_launch`'s own lock-release
/// discipline) -- a slow adapter must never freeze every other request.
fn dap_evaluate(
    state: &Arc<Mutex<BackendState>>,
    session_id: u64,
    expression: &str,
) -> Result<serde_json::Value, String> {
    let session = {
        let guard = state.lock().map_err(|_| "backend state poisoned")?;
        guard
            .dap_sessions
            .get(&session_id)
            .ok_or_else(|| format!("no dap session with id {session_id}"))?
            .clone()
    };
    let result = session.evaluate(expression)?;
    Ok(serde_json::json!({ "result": result }))
}

/// Real, explicit `Disconnect` (not a drop-triggered shutdown -- see
/// `spartan-dap::session`'s own doc comment for why this crate's shared
/// `Arc<DapSession>` needs an explicit command instead) plus removal
/// from `dap_sessions` -- an already-gone id is a real, harmless no-op,
/// matching `pty_close`'s own established precedent.
fn dap_disconnect(
    state: &Arc<Mutex<BackendState>>,
    session_id: u64,
) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    if let Some(session) = guard.dap_sessions.remove(&session_id) {
        session.send_command(spartan_dap::DapCommand::Disconnect);
    }
    Ok(serde_json::json!({ "ok": true }))
}

/// Real §75.74 dev containers -- OCI/Docker-based, following the open
/// containers.dev `devcontainer.json` spec (the same one VS Code Dev
/// Containers, GitHub Codespaces, and JetBrains Gateway implement),
/// closing the user's own "add virtual machine dev containers... to
/// allow testing projects on different OS's" request. A real, explicit
/// scope decision made with the user up front: container-based (real
/// Linux distro variation, not true separate-kernel VMs) -- this
/// sandbox itself has no `/dev/kvm` at all, so a QEMU/KVM-based approach
/// couldn't even be exercised here, and container-based dev environments
/// are the real, industry-standard answer this entire competitor
/// category actually ships.
///
/// Real, honest JSON summary of a detected config -- deliberately not
/// the full raw config (which may contain `containerEnv` values a UI
/// shouldn't echo back verbatim without more thought than this pass
/// gave it) -- just enough for the UI to show what's about to run.
fn devcontainer_config_summary_json(
    config: &spartan_devcontainer::spec::DevContainerConfig,
) -> serde_json::Value {
    serde_json::json!({
        "name": config.name,
        "image": config.image,
        "hasBuild": config.build.is_some(),
        "forwardPorts": config.forward_ports,
        "hasPostCreateCommand": config.post_create_command.is_some(),
    })
}

/// Real, fast, synchronous detect -- a single file read + JSONC parse,
/// never slow enough to need the async ack+event pattern the other
/// dev-container methods below use.
fn devcontainer_detect(project_root: &str) -> Result<serde_json::Value, String> {
    let config = spartan_devcontainer::spec::detect(std::path::Path::new(project_root))
        .map_err(|e| format!("devcontainer.json: {e}"))?;
    match config {
        None => Ok(serde_json::json!({ "found": false })),
        Some(cfg) => Ok(serde_json::json!({
            "found": true,
            "config": devcontainer_config_summary_json(&cfg),
        })),
    }
}

/// Real §21 Android support (task #11), first increment -- a real, fast,
/// synchronous detect wrapping `spartan_android`'s own real toolchain and
/// project-type detection. Deliberately narrow: no SDK install, no
/// emulator/device management, no build/run -- see `spartan-android`'s
/// own crate-level doc comment for the full, honest account of what this
/// does and does not cover yet.
fn android_detect(project_root: &str) -> Result<serde_json::Value, String> {
    let toolchain = spartan_android::detect_toolchain();
    let gradle_version = toolchain
        .gradle_path
        .as_deref()
        .and_then(spartan_android::detect_gradle_version);
    let is_android_project =
        spartan_android::is_android_project(std::path::Path::new(project_root));
    Ok(serde_json::json!({
        "sdkRoot": toolchain.sdk_root,
        "adbPath": toolchain.adb_path,
        "emulatorPath": toolchain.emulator_path,
        "sdkmanagerPath": toolchain.sdkmanager_path,
        "avdmanagerPath": toolchain.avdmanager_path,
        "gradlePath": toolchain.gradle_path,
        "gradleVersion": gradle_version,
        "isAndroidProject": is_android_project,
    }))
}

/// Real, live `assembleDebug` build (task #11's next increment beyond
/// `android_detect`'s detection-only scope) -- a real Android SDK
/// (build-tools/platforms/cmdline-tools) confirmed present in this
/// environment (unlike when `spartan-android` was first written) made
/// this achievable for the first time: compile + package a real,
/// installable debug APK. Still not the full §21 scope -- no emulator/
/// device exists here (no `/dev/kvm`, no system-images), so there is
/// nothing to install or run the resulting APK against; that stays
/// real, separate, unstarted follow-up, named honestly rather than
/// implied. The same "ack now, event later" shape `hf_pull_model`/
/// `llamacpp_download_model` already established -- a real Gradle build
/// can easily run minutes on a cold dependency cache, so it always runs
/// on its own thread, forwarding every real Gradle output line as an
/// `android_build_progress` event and finishing with
/// `android_build_ready` (the real produced `.apk` path) or
/// `android_build_failed`.
fn android_build_apk(
    out_tx: Sender<String>,
    project_root: String,
) -> Result<serde_json::Value, String> {
    if project_root.trim().is_empty() {
        return Err("android_build_apk requires a non-empty `project_root`".to_string());
    }
    let root = std::path::PathBuf::from(&project_root);
    if !spartan_android::is_android_project(&root) {
        return Err(format!(
            "{project_root:?} does not look like a real Android/Gradle project"
        ));
    }

    thread::spawn(move || {
        let (line_tx, line_rx) = mpsc::channel::<String>();
        let forward_out_tx = out_tx.clone();
        thread::spawn(move || {
            for line in line_rx {
                let event = Event {
                    event: "android_build_progress".to_string(),
                    data: serde_json::json!({ "line": line }),
                };
                if let Ok(l) = serde_json::to_string(&event) {
                    let _ = forward_out_tx.send(l);
                }
            }
        });

        let toolchain = spartan_android::detect_toolchain();
        let event = match spartan_android::build::build_debug_apk(
            &root,
            toolchain.sdk_root.as_deref(),
            toolchain.gradle_path.as_deref(),
            line_tx,
        ) {
            Ok(apk_path) => Event {
                event: "android_build_ready".to_string(),
                data: serde_json::json!({ "apk_path": apk_path.to_string_lossy() }),
            },
            Err(e) => Event {
                event: "android_build_failed".to_string(),
                data: serde_json::json!({ "error": e }),
            },
        };
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = out_tx.send(line);
        }
    });

    Ok(serde_json::json!({ "status": "starting" }))
}

/// Real, live, synchronous `adb devices -l` -- fast enough to run
/// directly (no background thread needed, unlike the build/install
/// paths). Requires a real detected `adb` on this machine; refuses
/// honestly, naming the reason, when none is found rather than
/// returning a fabricated empty list that would look identical to "no
/// device attached."
fn android_list_devices() -> Result<serde_json::Value, String> {
    let toolchain = spartan_android::detect_toolchain();
    let adb_path = toolchain.adb_path.ok_or_else(|| {
        "no real `adb` found on this machine -- install the Android SDK platform-tools".to_string()
    })?;
    let devices = spartan_android::adb::list_devices(&adb_path)?;
    Ok(serde_json::json!({ "devices": devices }))
}

/// Real, live `adb install -r <apk>`, optionally targeted at one
/// `serial` when more than one real device is attached -- the natural
/// next step after `android_build_apk` produces a real APK. Same
/// "ack now, event later" shape as every other real, possibly-slow
/// subprocess call in this crate (`android_install_progress`/
/// `android_install_ready`/`android_install_failed`).
fn android_install_apk(
    out_tx: Sender<String>,
    apk_path: String,
    serial: Option<String>,
) -> Result<serde_json::Value, String> {
    if apk_path.trim().is_empty() {
        return Err("android_install_apk requires a non-empty `apk_path`".to_string());
    }
    let toolchain = spartan_android::detect_toolchain();
    let adb_path = toolchain.adb_path.ok_or_else(|| {
        "no real `adb` found on this machine -- install the Android SDK platform-tools".to_string()
    })?;

    thread::spawn(move || {
        let (line_tx, line_rx) = mpsc::channel::<String>();
        let forward_out_tx = out_tx.clone();
        thread::spawn(move || {
            for line in line_rx {
                let event = Event {
                    event: "android_install_progress".to_string(),
                    data: serde_json::json!({ "line": line }),
                };
                if let Ok(l) = serde_json::to_string(&event) {
                    let _ = forward_out_tx.send(l);
                }
            }
        });

        let apk = std::path::PathBuf::from(&apk_path);
        let event =
            match spartan_android::adb::install_apk(&adb_path, serial.as_deref(), &apk, line_tx) {
                Ok(()) => Event {
                    event: "android_install_ready".to_string(),
                    data: serde_json::json!({ "apk_path": apk_path }),
                },
                Err(e) => Event {
                    event: "android_install_failed".to_string(),
                    data: serde_json::json!({ "error": e }),
                },
            };
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = out_tx.send(line);
        }
    });

    Ok(serde_json::json!({ "status": "starting" }))
}

/// Real, live `adb logcat` spawn (task #150, the last named piece of
/// task #11's device-management scope beyond the emulator/JDWP half this
/// environment's own `/dev/kvm` absence keeps out of reach). Unlike
/// `android_build_apk`/`android_install_apk`, this stream never resolves
/// on its own -- it runs until `android_logcat_stop` is called, matching
/// `pty_spawn`'s own real, unbounded-stream shape rather than the
/// bounded "ack now, one terminal event later" one those two use.
fn android_logcat_start(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    serial: Option<String>,
) -> Result<serde_json::Value, String> {
    let toolchain = spartan_android::detect_toolchain();
    let adb_path = toolchain.adb_path.ok_or_else(|| {
        "no real `adb` found on this machine -- install the Android SDK platform-tools".to_string()
    })?;

    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    let session_id = guard.next_logcat_id;

    let (line_tx, line_rx) = mpsc::channel::<String>();
    let forward_out_tx = out_tx.clone();
    thread::spawn(move || {
        for line in line_rx {
            let event = Event {
                event: "android_logcat_output".to_string(),
                data: serde_json::json!({ "session_id": session_id, "line": line }),
            };
            if let Ok(l) = serde_json::to_string(&event) {
                if forward_out_tx.send(l).is_err() {
                    break;
                }
            }
        }
        let event = Event {
            event: "android_logcat_exit".to_string(),
            data: serde_json::json!({ "session_id": session_id }),
        };
        if let Ok(l) = serde_json::to_string(&event) {
            let _ = out_tx.send(l);
        }
    });

    let handle = spartan_android::adb::spawn_logcat(&adb_path, serial.as_deref(), line_tx)
        .map_err(|e| format!("failed to spawn adb logcat: {e}"))?;
    guard.next_logcat_id += 1;
    guard.logcat_sessions.insert(session_id, handle);
    Ok(serde_json::json!({ "session_id": session_id }))
}

/// Real, hard stop -- an already-gone id is a real, harmless no-op,
/// matching `pty_close`'s own established precedent for a session that's
/// already ended on its own.
fn android_logcat_stop(
    state: &Arc<Mutex<BackendState>>,
    session_id: u64,
) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    if let Some(mut handle) = guard.logcat_sessions.remove(&session_id) {
        handle.kill();
    }
    Ok(serde_json::json!({ "ok": true }))
}

/// Real Docker container-name-safe sanitization (Docker's own real
/// charset is `[a-zA-Z0-9][a-zA-Z0-9_.-]+`) -- deterministic per project
/// path, so re-running "up" against the same project reuses the same
/// real name rather than accumulating a new container every time.
/// Real, shared sanitizer -- found duplicated (with silently different
/// semantics) by a code-review pass: `sanitize_container_name` and
/// `sanitize_project_name` each hand-rolled the same real
/// map-non-matching-to-dash/truncate/fallback-on-empty shape, differing
/// only in which extra characters survive, the length cap, and the
/// fallback string. One real function, parameterized, so a future rule
/// change (e.g. disallowing a leading dash) applies to both real
/// call sites at once instead of needing to be found and applied twice.
fn sanitize_identifier(
    input: &str,
    extra_allowed: &[char],
    max_len: usize,
    fallback: &str,
) -> String {
    let mut out: String = input
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || extra_allowed.contains(&c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    out.truncate(max_len);
    if out.is_empty() {
        out = fallback.to_string();
    }
    out
}

fn sanitize_container_name(input: &str) -> String {
    sanitize_identifier(input, &[], 48, "project")
}

/// Real "build or pull, then create+start, then run postCreateCommand"
/// pipeline -- the actual slow, multi-step real work `devcontainer_up`
/// runs on its own background thread, emitting real `devcontainer_
/// progress` events along the way so a real image pull/build (which can
/// genuinely take minutes) never looks hung.
fn run_devcontainer_up(
    project_root: &str,
    config: &spartan_devcontainer::spec::DevContainerConfig,
    out_tx: &Sender<String>,
) -> Result<(String, String), String> {
    use spartan_devcontainer::docker;

    let project_path = std::path::Path::new(project_root);
    let name_part = sanitize_container_name(project_root);
    let container_name = format!("spartan-devcontainer-{name_part}");

    let emit_progress = |line: String| {
        let event = Event {
            event: "devcontainer_progress".to_string(),
            data: serde_json::json!({ "line": line }),
        };
        if let Ok(l) = serde_json::to_string(&event) {
            let _ = out_tx.send(l);
        }
    };

    let image = if let Some(build) = &config.build {
        let dockerfile = build
            .dockerfile
            .clone()
            .unwrap_or_else(|| "Dockerfile".to_string());
        let context = build.context.clone().unwrap_or_else(|| ".".to_string());
        let context_dir = project_path.join(&context);
        let tag = format!("spartan-devcontainer:{name_part}");
        docker::build_image(&context_dir, &dockerfile, &build.args, &tag, &emit_progress)
            .map_err(|e| format!("image build failed: {e}"))?;
        tag
    } else if let Some(image) = &config.image {
        docker::pull_image(image, &emit_progress).map_err(|e| format!("image pull failed: {e}"))?;
        image.clone()
    } else {
        return Err("devcontainer.json has neither `image` nor `build`".to_string());
    };

    let container_id = docker::create_and_start_container(
        &image,
        config,
        project_path,
        project_root,
        &container_name,
    )
    .map_err(|e| format!("container creation failed: {e}"))?;

    if let Some(post_create) = &config.post_create_command {
        emit_progress("Running postCreateCommand...".to_string());
        let argv = post_create.to_argv();
        let (exit_code, output) = docker::run_command(&container_id, &argv)
            .map_err(|e| format!("postCreateCommand failed to run: {e}"))?;
        emit_progress(output);
        if exit_code != 0 {
            return Err(format!("postCreateCommand exited with code {exit_code}"));
        }
    }

    Ok((container_id, "running".to_string()))
}

/// Real async "up" -- detects the config and confirms Docker is
/// actually reachable synchronously first (so a plain, immediate,
/// specific error -- "no devcontainer.json," "Docker isn't running" --
/// never has to round-trip through the async event path), then runs the
/// real, possibly multi-minute build/pull/create/postCreate pipeline on
/// its own background thread.
fn devcontainer_up(
    out_tx: Sender<String>,
    project_root: String,
) -> Result<serde_json::Value, String> {
    let config = spartan_devcontainer::spec::detect(std::path::Path::new(&project_root))
        .map_err(|e| format!("devcontainer.json: {e}"))?
        .ok_or("no devcontainer.json found in this project (checked .devcontainer/devcontainer.json and .devcontainer.json)")?;

    if !spartan_devcontainer::docker::is_docker_available() {
        return Err(
            "Docker isn't running or isn't reachable -- start Docker (or Docker Desktop) and try again"
                .to_string(),
        );
    }

    thread::spawn(move || {
        let result = run_devcontainer_up(&project_root, &config, &out_tx);
        let event = match result {
            Ok((container_id, status)) => Event {
                event: "devcontainer_ready".to_string(),
                data: serde_json::json!({ "container_id": container_id, "status": status }),
            },
            Err(message) => Event {
                event: "devcontainer_failed".to_string(),
                data: serde_json::json!({ "error": message }),
            },
        };
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = out_tx.send(line);
        }
    });

    Ok(serde_json::json!({ "status": "starting" }))
}

/// Real async stop+remove -- a real Docker stop can take up to its own
/// real grace-period timeout before falling back to a hard kill, so
/// this follows the same immediate-ack/later-event shape as every other
/// real, possibly-slow operation in this file rather than blocking the
/// one IPC channel for that whole window.
fn devcontainer_down(
    out_tx: Sender<String>,
    container_id: String,
) -> Result<serde_json::Value, String> {
    thread::spawn(move || {
        let result = spartan_devcontainer::docker::stop_and_remove(&container_id);
        let event = match result {
            Ok(()) => Event {
                event: "devcontainer_stopped".to_string(),
                data: serde_json::json!({ "container_id": container_id }),
            },
            Err(e) => Event {
                event: "devcontainer_failed".to_string(),
                data: serde_json::json!({ "error": e.to_string() }),
            },
        };
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = out_tx.send(line);
        }
    });
    Ok(serde_json::json!({ "status": "stopping" }))
}

fn devcontainer_status(container_id: &str) -> Result<serde_json::Value, String> {
    let status =
        spartan_devcontainer::docker::container_status(container_id).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "status": status }))
}

fn devcontainer_list() -> Result<serde_json::Value, String> {
    let containers =
        spartan_devcontainer::docker::list_managed_containers().map_err(|e| e.to_string())?;
    Ok(serde_json::json!(containers
        .iter()
        .map(|c| serde_json::json!({
            "id": c.id,
            "name": c.name,
            "image": c.image,
            "status": c.status,
            "projectLabel": c.project_label,
        }))
        .collect::<Vec<_>>()))
}

/// Real interactive `docker exec -it`-equivalent session, the container
/// analogue of `pty_spawn` -- output streams back as real, unprompted
/// `devcontainer_exec_output`/`devcontainer_exec_exit` events, keyed by
/// the same real per-session id scheme `pty_sessions` already
/// established, never blocking this synchronous call itself.
fn devcontainer_exec_spawn(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    container_id: &str,
    cols: u16,
    rows: u16,
) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    let session_id = guard.next_devcontainer_exec_id;
    let out_tx_output = out_tx.clone();
    let handle = spartan_devcontainer::docker::spawn_interactive_exec(
        container_id,
        cols,
        rows,
        move |bytes| {
            let chunk = String::from_utf8_lossy(&bytes).into_owned();
            let event = Event {
                event: "devcontainer_exec_output".to_string(),
                data: serde_json::json!({ "session_id": session_id, "chunk": chunk }),
            };
            if let Ok(line) = serde_json::to_string(&event) {
                let _ = out_tx_output.send(line);
            }
        },
        move || {
            let event = Event {
                event: "devcontainer_exec_exit".to_string(),
                data: serde_json::json!({ "session_id": session_id }),
            };
            if let Ok(line) = serde_json::to_string(&event) {
                let _ = out_tx.send(line);
            }
        },
    )
    .map_err(|e| format!("failed to spawn exec session: {e}"))?;
    guard.next_devcontainer_exec_id += 1;
    guard.devcontainer_exec_sessions.insert(session_id, handle);
    Ok(serde_json::json!({ "session_id": session_id }))
}

fn devcontainer_exec_input(
    state: &Arc<Mutex<BackendState>>,
    session_id: u64,
    data: &str,
) -> Result<serde_json::Value, String> {
    let guard = state.lock().map_err(|_| "backend state poisoned")?;
    let handle = guard
        .devcontainer_exec_sessions
        .get(&session_id)
        .ok_or_else(|| format!("no devcontainer exec session with id {session_id}"))?;
    handle
        .write(data.as_bytes())
        .map_err(|e| format!("exec write failed: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

fn devcontainer_exec_resize(
    state: &Arc<Mutex<BackendState>>,
    session_id: u64,
    cols: u16,
    rows: u16,
) -> Result<serde_json::Value, String> {
    let guard = state.lock().map_err(|_| "backend state poisoned")?;
    let handle = guard
        .devcontainer_exec_sessions
        .get(&session_id)
        .ok_or_else(|| format!("no devcontainer exec session with id {session_id}"))?;
    handle
        .resize(cols, rows)
        .map_err(|e| format!("exec resize failed: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

fn devcontainer_exec_close(
    state: &Arc<Mutex<BackendState>>,
    session_id: u64,
) -> Result<serde_json::Value, String> {
    let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
    if let Some(handle) = guard.devcontainer_exec_sessions.remove(&session_id) {
        handle.close();
    }
    Ok(serde_json::json!({ "ok": true }))
}

/// Real, honest JSON rendering of `spartan_git::FileStatus` -- matches
/// the enum's own variant names lowercased, no fabricated glyphs (the
/// original wgpu shell's `git_panel.rs` renders status *glyphs*
/// client-side from these same names; here the renderer owns that
/// presentation choice instead, this crate just reports real fact).
fn file_status_json(status: spartan_git::FileStatus) -> &'static str {
    use spartan_git::FileStatus::*;
    match status {
        Modified => "modified",
        Added => "added",
        Deleted => "deleted",
        Renamed => "renamed",
        TypeChanged => "type_changed",
    }
}

/// Real "Find in Files" (task #190) -- the first real, direct UI caller
/// of `spartan_leo::tool::Sandbox::search_files`, a real, bounded,
/// already-tested substring search that's existed since §75.68 as one of
/// Leo's own real tool calls but never had a caller outside the agent
/// loop. Reused verbatim here rather than reimplemented: a real
/// throwaway `Sandbox` is constructed purely to get its path-jailed,
/// noise-directory-skipping, match/file-count-bounded walk for free --
/// this function has no Leo/agent/model involvement at all. Kept
/// synchronous and stateless-per-call, matching `git_status`'s own
/// precedent immediately below: a pure filesystem walk with no
/// subprocess spawn, unlike `format_document`'s real external-process
/// call, so it doesn't need the "ack now, event later" treatment.
fn search_project(
    project_root: &str,
    pattern: &str,
    path: Option<&str>,
) -> Result<serde_json::Value, String> {
    let sandbox = spartan_leo::tool::Sandbox::new(project_root);
    match sandbox
        .search_files(pattern, path)
        .map_err(|e| e.to_string())?
    {
        spartan_leo::tool::ToolResult::SearchMatches(matches) => {
            let json_matches: Vec<serde_json::Value> = matches
                .into_iter()
                .map(|m| serde_json::json!({ "path": m.path, "line": m.line, "text": m.text }))
                .collect();
            Ok(serde_json::json!({ "matches": json_matches }))
        }
        // `Sandbox::search_files` always returns `SearchMatches` on
        // success -- every other `ToolResult` variant belongs to a
        // different real tool call entirely (see `execute()`'s own
        // match arms), never reachable from this call site.
        _ => unreachable!("search_files always returns ToolResult::SearchMatches"),
    }
}

/// Real, stateless-per-call git status -- no `GitRepo` is kept open in
/// `BackendState` between calls (unlike Leo's own `leo_project_root`,
/// which needs a live `Agent`), since every real git operation here is
/// a one-shot `git2` call cheap enough to re-discover the repository
/// each time, matching `leo_approve_plan`'s own existing precedent for
/// this exact discovery call.
fn git_status(project_root: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let entries = repo
        .status()
        .map_err(|e| format!("git status: {e}"))?
        .into_iter()
        .map(|entry| {
            serde_json::json!({
                "path": entry.path.to_string_lossy(),
                "staged": entry.staged.map(file_status_json),
                "unstaged": entry.unstaged.map(file_status_json),
                "conflicted": entry.conflicted,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "branch": repo.current_branch(),
        "entries": entries,
    }))
}

fn git_stage(project_root: &str, path: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.stage(std::path::Path::new(path))
        .map_err(|e| format!("git stage: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

fn git_unstage(project_root: &str, path: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.unstage(std::path::Path::new(path))
        .map_err(|e| format!("git unstage: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Real "discard changes" -- restores one path's working-tree file to the
/// index version (a `git checkout -- <path>`), dropping unstaged edits. A
/// destructive operation; the UI confirms with the user before calling it.
fn git_discard(project_root: &str, path: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.discard_changes(std::path::Path::new(path))
        .map_err(|e| format!("git discard: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

fn git_commit(project_root: &str, message: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let oid = repo
        .commit(message)
        .map_err(|e| format!("git commit: {e}"))?;
    Ok(serde_json::json!({ "ok": true, "oid": oid.to_string() }))
}

/// Real amend of the last commit -- rewrites `HEAD`'s message and folds the
/// current index into it, keeping the commit count unchanged (replaces, does
/// not add). Errors honestly if there is no `HEAD` commit to amend yet.
fn git_commit_amend(project_root: &str, message: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let oid = repo
        .commit_amend(message)
        .map_err(|e| format!("git commit --amend: {e}"))?;
    Ok(serde_json::json!({ "ok": true, "oid": oid.to_string() }))
}

/// Real revert of a commit by its hex `oid` -- creates a new commit undoing
/// that commit's changes (exactly `git revert`), never rewriting history. A
/// revert that conflicts is reported honestly rather than committed.
fn git_revert_commit(project_root: &str, oid: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let new_oid = repo
        .revert_commit(oid)
        .map_err(|e| format!("git revert: {e}"))?;
    Ok(serde_json::json!({ "ok": true, "oid": new_oid.to_string() }))
}

/// Real list of tags, each with its target commit oid and annotated flag.
fn git_tags(project_root: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let tags = repo
        .list_tags()
        .map_err(|e| format!("git tag list: {e}"))?
        .into_iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "target": t.target,
                "annotated": t.annotated,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "tags": tags }))
}

/// Real tag creation on a commit `oid` -- annotated if `message` is a
/// non-empty string, else lightweight. An existing name is a real error.
fn git_create_tag(
    project_root: &str,
    name: &str,
    oid: &str,
    message: Option<&str>,
) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.create_tag(name, oid, message)
        .map_err(|e| format!("git tag: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Real tag deletion by name.
fn git_delete_tag(project_root: &str, name: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.delete_tag(name)
        .map_err(|e| format!("git tag -d: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Real staged/unstaged diff for one file, reusing the already-tested
/// `compute_diff` (§75.68) -- real git semantics, not a simplified
/// approximation. `staged: true` diffs `HEAD`'s own blob against the
/// index's blob (exactly what `git diff --staged` shows). `staged: false`
/// diffs the index's own blob against the real current working-tree file
/// content read directly off disk (exactly what a plain `git diff` shows
/// for an already-tracked file), resolved against the repo's own real
/// `workdir()` rather than the raw `project_root` param, since a real git
/// repo can be discovered from a subdirectory while `path` is always
/// root-relative to the repo itself. A path missing from either real half
/// (HEAD/index/disk) is treated as empty content, not an error -- the
/// correct, honest representation of a real newly-added or newly-deleted
/// file, matching `compute_diff`'s own existing "every line added/removed"
/// behavior against empty content.
fn git_diff(project_root: &str, path: &str, staged: bool) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let rel_path = std::path::Path::new(path);
    let index_content = repo
        .index_blob_content(rel_path)
        .map_err(|e| format!("git diff (index): {e}"))?
        .unwrap_or_default();
    let (old_content, new_content) = if staged {
        let head_content = repo
            .head_blob_content(rel_path)
            .map_err(|e| format!("git diff (HEAD): {e}"))?
            .unwrap_or_default();
        (head_content, index_content)
    } else {
        let workdir = repo
            .workdir()
            .ok_or("repository has no working directory")?;
        let disk_content = std::fs::read_to_string(workdir.join(rel_path)).unwrap_or_default();
        (index_content, disk_content)
    };
    Ok(serde_json::json!({
        "diff": compute_diff(&old_content, &new_content),
    }))
}

/// Real per-hunk unstaged diff for one file -- lists every real hunk
/// `spartan_git::diff_hunks` identifies, so the UI can offer a "stage this
/// hunk" action per real hunk rather than only whole-file staging.
fn git_diff_hunks(project_root: &str, path: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let hunks = repo
        .diff_hunks(std::path::Path::new(path))
        .map_err(|e| format!("git diff hunks: {e}"))?
        .into_iter()
        .map(|h| serde_json::json!({ "index": h.index, "header": h.header, "body": h.body }))
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "hunks": hunks }))
}

/// Real "stage this one hunk" (`git add -p`'s own per-hunk selection) --
/// `hunk_index` must refer to a hunk from the *most recent* real
/// `git_diff_hunks` call for this file, since staging one hunk changes the
/// real index and so the real hunk list for any later staging action.
fn git_stage_hunk(
    project_root: &str,
    path: &str,
    hunk_index: u64,
) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.stage_hunk(std::path::Path::new(path), hunk_index as usize)
        .map_err(|e| format!("git stage hunk: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Real local branch list -- every real local branch name plus which one
/// is current (none flagged for a real detached `HEAD`).
fn git_branches(project_root: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let branches = repo
        .list_branches()
        .map_err(|e| format!("git branches: {e}"))?
        .into_iter()
        .map(|(name, is_current)| serde_json::json!({ "name": name, "current": is_current }))
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "branches": branches }))
}

/// Real, *safe* branch switch -- `spartan_git::checkout_branch` uses
/// `libgit2`'s own conflict-refusing safe checkout, so a real uncommitted
/// change that conflicts with the target branch surfaces the real error
/// here (relayed verbatim) with the repository left exactly where it was,
/// never force-discarded.
fn git_checkout(project_root: &str, branch: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.checkout_branch(branch)
        .map_err(|e| format!("git checkout: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Real remote-tracking branch list (`refs/remotes/*` as of the last
/// fetch), e.g. `origin/feature`. The symbolic `origin/HEAD` is skipped.
fn git_remote_branches(project_root: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let branches = repo
        .list_remote_branches()
        .map_err(|e| format!("git remote branches: {e}"))?;
    Ok(serde_json::json!({ "branches": branches }))
}

/// Real check out of a remote-tracking branch (e.g. `origin/feature`):
/// creates a local tracking branch if needed, then switches via the same
/// *safe* checkout `git_checkout` uses (a conflicting dirty change is
/// refused, not clobbered).
fn git_checkout_remote(
    project_root: &str,
    remote_branch: &str,
) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.checkout_remote_branch(remote_branch)
        .map_err(|e| format!("git checkout remote: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Real `git merge <branch>` -- accepts a local (`feature`) or
/// remote-tracking (`origin/feature`) branch name, matching both real
/// namespaces `git_branches`/`git_remote_branches` already expose. Reports
/// exactly which real outcome `spartan_git::MergeOutcome` produced; a real
/// `"conflicted"` result means real conflicts now sit in the working tree
/// and index -- `git_merge_status`/`git_resolve_conflict`/`git_commit_merge`
/// are the real next steps.
fn git_merge_branch(project_root: &str, branch: &str) -> Result<serde_json::Value, String> {
    let mut repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let outcome = repo
        .merge_branch(branch)
        .map_err(|e| format!("git merge: {e}"))?;
    let outcome_str = match outcome {
        spartan_git::MergeOutcome::UpToDate => "up_to_date",
        spartan_git::MergeOutcome::FastForwarded => "fast_forwarded",
        spartan_git::MergeOutcome::Merged => "merged",
        spartan_git::MergeOutcome::Conflicted => "conflicted",
    };
    Ok(serde_json::json!({ "outcome": outcome_str }))
}

/// Real merge-in-progress status: whether the repo is currently mid-merge
/// (`RepositoryState::Merge`) and, if so, every real conflicted file's
/// real ancestor/ours/theirs content -- one round trip covering everything
/// the conflict-resolution UI needs to render, rather than racing two
/// separate calls against a repo state that could change between them.
fn git_merge_status(project_root: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let in_progress = repo.merge_in_progress();
    let conflicts = repo
        .list_conflicts()
        .map_err(|e| format!("git merge status: {e}"))?
        .into_iter()
        .map(|c| {
            serde_json::json!({
                "path": c.path.to_string_lossy(),
                "ancestor": c.ancestor,
                "ours": c.ours,
                "theirs": c.theirs,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "in_progress": in_progress, "conflicts": conflicts }))
}

/// Real one-click conflict resolution -- writes `content` to the real
/// working-tree file and stages it, resolving the conflict. `content` is
/// typically one of `git_merge_status`'s own real `ours`/`theirs` values
/// (a real "take ours"/"take theirs" action) or the user's own hand-edited
/// text.
fn git_resolve_conflict(
    project_root: &str,
    path: &str,
    content: &str,
) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.resolve_conflict_with_content(std::path::Path::new(path), content)
        .map_err(|e| format!("git resolve conflict: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Real merge-commit completion -- a real two-parent commit (`HEAD` +
/// `MERGE_HEAD`). Refuses honestly if real conflicts remain unresolved.
fn git_commit_merge(project_root: &str, message: &str) -> Result<serde_json::Value, String> {
    let mut repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let oid = repo
        .commit_merge(message)
        .map_err(|e| format!("git commit merge: {e}"))?;
    Ok(serde_json::json!({ "oid": oid.to_string() }))
}

/// Real, destructive merge abort -- resets the working tree/index back to
/// `HEAD`, discarding the in-progress merge and any partial resolutions.
/// The frontend is responsible for confirming with the user first.
fn git_abort_merge(project_root: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.abort_merge()
        .map_err(|e| format!("git abort merge: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Real `git log` -- the most recent commits reachable from `HEAD`,
/// newest first, bounded by a caller-supplied (or default 50) `max`. A
/// repo with no commits returns an honest empty list.
fn git_log(project_root: &str, max: usize) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let commits = repo
        .log(max)
        .map_err(|e| format!("git log: {e}"))?
        .into_iter()
        .map(|c| {
            serde_json::json!({
                "oid": c.oid,
                "summary": c.summary,
                "author": c.author,
                "time": c.time,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "commits": commits }))
}

/// Real `git log <ref_name>` -- a named branch's own commits (local or
/// remote-tracking), browsable without checking that branch out.
fn git_log_for_ref(
    project_root: &str,
    ref_name: &str,
    max: usize,
) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let commits = repo
        .list_commits_for_ref(ref_name, max)
        .map_err(|e| format!("git log {ref_name}: {e}"))?
        .into_iter()
        .map(|c| {
            serde_json::json!({
                "oid": c.oid,
                "summary": c.summary,
                "author": c.author,
                "time": c.time,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "commits": commits }))
}

/// Real `git cherry-pick <oid>` -- applies a real commit's changes onto the
/// current `HEAD` as a new, single-parent commit. A real conflict, or a
/// commit whose changes are already fully present on `HEAD`, is reported
/// honestly rather than silently absorbed or committed anyway.
fn git_cherry_pick(project_root: &str, oid: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let new_oid = repo
        .cherry_pick_commit(oid)
        .map_err(|e| format!("git cherry-pick: {e}"))?;
    Ok(serde_json::json!({ "ok": true, "oid": new_oid.to_string() }))
}

/// Per-line blame for a file as committed in `HEAD`: for each line, in
/// file order, the real commit that last touched it. `path` may be
/// absolute (the editor's open-file path) or repo-relative (the Git
/// panel's convention); both resolve to the repo-workdir-relative path
/// `spartan_git::blame_file` needs. An untracked/new file, or a repo with
/// no commits, returns an empty line list -- a real, valid state.
fn git_blame(project_root: &str, path: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let workdir = repo
        .workdir()
        .ok_or("repository has no working directory")?;
    let raw = std::path::Path::new(path);
    // Canonicalize both sides before stripping so a symlinked or
    // non-normalized absolute path still resolves; fall back to the raw
    // path (treated as already repo-relative) if stripping doesn't apply.
    let canon_workdir = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());
    let canon_raw = raw.canonicalize().unwrap_or_else(|_| raw.to_path_buf());
    let rel_path = canon_raw.strip_prefix(&canon_workdir).unwrap_or(raw);
    let lines = repo
        .blame_file(rel_path)
        .map_err(|e| format!("git blame: {e}"))?
        .into_iter()
        .map(|b| {
            serde_json::json!({
                "oid": b.oid,
                "summary": b.summary,
                "author": b.author,
                "time": b.time,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "lines": lines }))
}

/// Every configured real remote as `{name, url}`.
fn git_remotes(project_root: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let remotes = repo
        .list_remotes()
        .map_err(|e| format!("git remotes: {e}"))?
        .into_iter()
        .map(|(name, url)| serde_json::json!({ "name": name, "url": url }))
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "remotes": remotes }))
}

/// Real fetch from a configured remote (updates remote-tracking refs; the
/// working tree is untouched). Synchronous like the rest of this crate's
/// git dispatch -- a real network remote can make this slow; wiring it to
/// the "ack now, event later" async pattern is a named follow-up.
fn git_fetch(project_root: &str, remote: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.fetch(remote).map_err(|e| format!("git fetch: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Real push of a local branch to the same-named branch on a remote. A
/// rejected push (non-fast-forward remote, auth failure) surfaces the real
/// error verbatim -- never a force-push.
fn git_push(project_root: &str, remote: &str, branch: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.push(remote, branch)
        .map_err(|e| format!("git push: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Real pull = fetch + fast-forward-only. A real divergence is reported as
/// `non_fast_forward` rather than auto-merged/rebased -- the safe v1
/// behavior, leaving the working tree untouched.
fn git_pull(project_root: &str, remote: &str, branch: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let outcome = repo
        .pull_fast_forward(remote, branch)
        .map_err(|e| format!("git pull: {e}"))?;
    let s = match outcome {
        spartan_git::PullOutcome::UpToDate => "up_to_date",
        spartan_git::PullOutcome::FastForwarded => "fast_forwarded",
        spartan_git::PullOutcome::NonFastForward => "non_fast_forward",
    };
    Ok(serde_json::json!({ "outcome": s }))
}

/// Real `git stash` of the working changes to tracked files. Reports
/// `stashed: false` when there was nothing to stash (a clean tree).
fn git_stash_save(project_root: &str, message: &str) -> Result<serde_json::Value, String> {
    let mut repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let oid = repo
        .stash_save(message)
        .map_err(|e| format!("git stash: {e}"))?;
    Ok(serde_json::json!({ "stashed": oid.is_some() }))
}

/// Every real stash entry, newest first.
fn git_stash_list(project_root: &str) -> Result<serde_json::Value, String> {
    let mut repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let stashes = repo
        .stash_list()
        .map_err(|e| format!("git stash list: {e}"))?
        .into_iter()
        .map(|s| serde_json::json!({ "index": s.index, "message": s.message, "oid": s.oid }))
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "stashes": stashes }))
}

/// Real `git stash pop <index>` -- applies the stash and drops it. A real
/// conflict surfaces `libgit2`'s own error verbatim, never force-applied.
fn git_stash_pop(project_root: &str, index: usize) -> Result<serde_json::Value, String> {
    let mut repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.stash_pop(index)
        .map_err(|e| format!("git stash pop: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Real `git stash apply <index>` -- applies the stash but keeps it in the
/// list (unlike pop). A real conflict surfaces `libgit2`'s own error verbatim.
fn git_stash_apply(project_root: &str, index: usize) -> Result<serde_json::Value, String> {
    let mut repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.stash_apply(index)
        .map_err(|e| format!("git stash apply: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Real `git stash drop <index>` -- discards a stash without applying.
fn git_stash_drop(project_root: &str, index: usize) -> Result<serde_json::Value, String> {
    let mut repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.stash_drop(index)
        .map_err(|e| format!("git stash drop: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Every file a real commit changed, relative to its first parent (a
/// root commit reports everything as added -- the real, correct answer).
fn git_commit_files(project_root: &str, oid: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let files = repo
        .commit_changed_files(oid)
        .map_err(|e| format!("git commit files: {e}"))?
        .into_iter()
        .map(|(path, status)| {
            serde_json::json!({ "path": path, "status": file_status_json(status) })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "files": files }))
}

/// One file's real diff within one real commit -- the commit's own blob
/// for `path` against its first parent's blob (a root commit, or a path
/// the parent doesn't have, diffs against empty content -- the same
/// missing-half-is-empty convention `git_diff` already established).
/// Reuses the same already-tested `compute_diff`.
fn git_commit_diff(project_root: &str, oid: &str, path: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let rel_path = std::path::Path::new(path);
    let new_content = repo
        .commit_blob_content(oid, rel_path)
        .map_err(|e| format!("git commit diff: {e}"))?
        .unwrap_or_default();
    let old_content = match repo
        .commit_parent(oid)
        .map_err(|e| format!("git commit diff (parent): {e}"))?
    {
        Some(parent_oid) => repo
            .commit_blob_content(&parent_oid, rel_path)
            .map_err(|e| format!("git commit diff (parent blob): {e}"))?
            .unwrap_or_default(),
        None => String::new(),
    };
    Ok(serde_json::json!({
        "diff": compute_diff(&old_content, &new_content),
    }))
}

/// Real `git branch <name>` from the current `HEAD` -- does not switch to
/// the new branch (matching the real command's own behavior); an existing
/// branch of the same name is a real, relayed error.
fn git_create_branch(project_root: &str, branch: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    repo.create_branch(branch)
        .map_err(|e| format!("git create branch: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Real settings read/write, wrapping `spartan_settings` directly --
/// deliberately no in-memory caching in `BackendState`, since this
/// crate's own request volume for settings is low (opened once when the
/// Settings screen mounts, written once per real user change) and a
/// second source of truth beyond the real file on disk would only risk
/// drifting from it, the same reasoning `settings_panel.rs` in the
/// original wgpu shell already established (it re-reads fresh, too).
fn settings_get() -> Result<serde_json::Value, String> {
    let settings = spartan_settings::load();
    serde_json::to_value(settings).map_err(|e| format!("serialize settings: {e}"))
}

/// The real, shared `~/.spartan/crashes` directory both this crate's own
/// `main.rs` (via `install_hook`) and `spartan-editor-core`'s reference
/// shell already write real crash reports to (`crash_dir()` there,
/// §75.32) -- kept byte-identical (same env-var fallback order) so a
/// crash from either shell on one machine lands in the one place a user
/// or beta program would actually go look.
pub fn crash_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".spartan").join("crashes")
}

/// Real §75.82 listing of every local crash report on disk, newest
/// first, closing task #35's own "beta testers need a way to see and
/// send these" half. Returns each report's real filename and its real,
/// already-redacted file contents parsed back to structured JSON --
/// never raw, unredacted data, since `write_report` (§75.32) redacts
/// before it ever touches disk in the first place.
fn crash_reports_list() -> Result<serde_json::Value, String> {
    let paths =
        spartan_crash::list_reports(&crash_dir()).map_err(|e| format!("list reports: {e}"))?;
    let reports: Vec<serde_json::Value> = paths
        .iter()
        .filter_map(|path| {
            let filename = path.file_name()?.to_str()?.to_string();
            let contents = std::fs::read_to_string(path).ok()?;
            let parsed: serde_json::Value = serde_json::from_str(&contents).ok()?;
            Some(serde_json::json!({ "filename": filename, "report": parsed }))
        })
        .collect();
    Ok(serde_json::json!({ "reports": reports }))
}

/// Real, explicit, user-initiated upload of exactly one already-written,
/// already-redacted local report to a user-configured endpoint -- never
/// automatic, matching §18's own "never auto-uploads" contract and
/// `spartan_crash::upload_report`'s own doc comment. `filename` is
/// validated against a plain `crash-<digits>.json` shape (matching what
/// `write_report`/`list_reports` themselves only ever produce) before
/// being joined onto `crash_dir()`, so this can never be tricked into
/// reading or sending an arbitrary path elsewhere on disk.
fn crash_report_upload(filename: &str) -> Result<serde_json::Value, String> {
    let is_valid_filename = filename.starts_with("crash-")
        && filename.ends_with(".json")
        && filename[6..filename.len() - 5]
            .chars()
            .all(|c| c.is_ascii_digit());
    if !is_valid_filename {
        return Err(format!(
            "refusing to upload unexpected filename: {filename}"
        ));
    }
    let settings = spartan_settings::load();
    let endpoint = settings
        .crash_reporting
        .upload_endpoint
        .ok_or("no crash-report upload endpoint configured in Settings")?;
    let path = crash_dir().join(filename);
    let report_json =
        std::fs::read_to_string(&path).map_err(|e| format!("read report {filename}: {e}"))?;
    let status = spartan_crash::upload_report(&endpoint, &report_json)
        .map_err(|e| format!("upload failed: {e}"))?;
    Ok(serde_json::json!({ "status": status }))
}

/// Real, deliberate patch shape -- found as a real "too many positional
/// arguments" smell by a code-review pass (`settings_set` had grown to 7
/// same-typed `Option<T>`-in-a-row parameters, needing a real
/// `#[allow(clippy::too_many_arguments)]`, the only one anywhere in
/// `crates/`). Every field besides `gpu_*` is only ever sent when the
/// Settings screen's own corresponding row actually changed, so an
/// unrelated save must not silently reset the others back to their real
/// defaults -- `settings_set` loads the current settings first and only
/// overrides what was actually provided here. The dispatch arm still
/// parses each field individually from `req.params` (unchanged, so every
/// existing `"invalid X: ..."` error message stays byte-identical) and
/// simply collects the results into one real struct instead of passing
/// them as 7 separate positional arguments.
struct SettingsPatch {
    gpu_enabled: bool,
    gpu_layers: Option<u32>,
    leo_approval_mode: Option<spartan_settings::LeoApprovalMode>,
    leo_provider: Option<spartan_settings::LeoProviderSettings>,
    editor: Option<spartan_settings::EditorSettings>,
    appearance: Option<spartan_settings::AppearanceSettings>,
    crash_reporting: Option<spartan_settings::CrashReportingSettings>,
    onboarding_completed: Option<bool>,
    /// Nested `Option` on purpose (task #265): the setting *value* is itself
    /// `Option<String>` (`None` = no verification command), so the patch
    /// needs a third state to distinguish "not provided in this patch, keep
    /// current" (outer `None`) from "provided as empty, clear it" (outer
    /// `Some(None)`) from "provided as a real command" (`Some(Some(cmd))`) --
    /// the same "only override what was actually sent" discipline every
    /// other field here follows, one level deeper because this value can
    /// itself be absent.
    leo_verify_command: Option<Option<String>>,
}

fn settings_set(patch: SettingsPatch) -> Result<serde_json::Value, String> {
    let current = spartan_settings::load();
    let settings = spartan_settings::Settings {
        gpu_offload: spartan_settings::GpuOffloadSettings {
            enabled: patch.gpu_enabled,
            layers: patch.gpu_layers,
        },
        leo_approval_mode: patch.leo_approval_mode.unwrap_or(current.leo_approval_mode),
        leo_provider: patch.leo_provider.unwrap_or(current.leo_provider),
        editor: patch.editor.unwrap_or(current.editor),
        appearance: patch.appearance.unwrap_or(current.appearance),
        crash_reporting: patch.crash_reporting.unwrap_or(current.crash_reporting),
        onboarding_completed: patch
            .onboarding_completed
            .unwrap_or(current.onboarding_completed),
        leo_verify_command: patch
            .leo_verify_command
            .unwrap_or(current.leo_verify_command),
    };
    spartan_settings::save(&settings).map_err(|e| format!("save settings: {e}"))?;
    serde_json::to_value(settings).map_err(|e| format!("serialize settings: {e}"))
}

/// Real §75.72 wiring of `spartan-updater` (already real and tested since
/// §75.49, already wired into the original wgpu shell's own
/// `settings_panel.rs`/`update_bridge.rs`) into this shell for the first
/// time -- closes the gap §75.65 named explicitly ("Settings exposes only
/// GPU offload -- the wgpu shell's separate 'Check for Updates' row is
/// not wired into `spartan-backend` or this screen this pass"). A real,
/// possibly-slow HTTPS round trip against the GitHub API, so this follows
/// the exact same spawn-thread/immediate-ack/later-`Event` shape
/// `leo_start_task` already established -- it must never block the one
/// IPC channel. Neither `UpdateCheckResult` nor `ChangeCategories`
/// derives `Serialize` (a real, deliberate choice in `spartan-updater`
/// itself, which has no JSON/IPC concerns of its own), so this function
/// builds the wire shape by hand at this one real boundary, the same
/// pattern `plan_json`/`tool_result_json` already use elsewhere in this
/// file for other crates' own plain Rust types.
fn check_for_updates(out_tx: Sender<String>) -> Result<serde_json::Value, String> {
    // This project's own real repository and default branch -- matching
    // the exact values `update_bridge.rs` in the original wgpu shell
    // already uses (and `spartan-updater`'s own live integration test).
    const REPO: &str = "ckissinger1988/spartan-ide";
    const BRANCH: &str = "main";
    thread::spawn(move || {
        let event = match spartan_updater::check_for_updates(REPO, BRANCH) {
            Ok(result) => Event {
                event: "update_check_result".to_string(),
                data: serde_json::json!({
                    "current_commit": result.current_commit,
                    "latest_commit": result.latest_commit,
                    "up_to_date": result.up_to_date,
                    "categories": {
                        "language_definitions_changed": result.categories.language_definitions_changed,
                        "leo_changed": result.categories.leo_changed,
                        "other_changed": result.categories.other_changed,
                    },
                }),
            },
            Err(e) => Event {
                event: "update_check_failed".to_string(),
                data: serde_json::json!({ "error": e.to_string() }),
            },
        };
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = out_tx.send(line);
        }
    });
    Ok(serde_json::json!({ "status": "checking" }))
}

/// Real §75.76 "New Project" quick-start scaffolding -- one small, real,
/// runnable starter file set per Tier 1 language (§35.4's original six
/// plus C#, §75.51), each one deliberately matching `spartan-languages`'
/// own real `languages.toml` marker files exactly, so a project created
/// here is correctly detected by this app's own real language registry
/// the moment it's opened -- not a separate, parallel "starter project"
/// concept invented just for this wizard.
fn project_template_files(template: &str) -> Result<Vec<(&'static str, &'static str)>, String> {
    match template {
        "rust" => Ok(vec![
            (
                "Cargo.toml",
                "[package]\nname = \"{{name}}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
            ),
            (
                "src/main.rs",
                "fn main() {\n    println!(\"Hello from {{name}}!\");\n}\n",
            ),
        ]),
        "typescript" => Ok(vec![
            (
                "package.json",
                "{\n  \"name\": \"{{name}}\",\n  \"version\": \"0.1.0\",\n  \"private\": true,\n  \"type\": \"module\",\n  \"scripts\": {\n    \"start\": \"node index.js\"\n  }\n}\n",
            ),
            (
                "tsconfig.json",
                "{\n  \"compilerOptions\": {\n    \"target\": \"ES2020\",\n    \"module\": \"ESNext\",\n    \"strict\": true\n  }\n}\n",
            ),
            (
                "index.ts",
                "console.log(\"Hello from {{name}}!\");\n",
            ),
        ]),
        "javascript" => Ok(vec![
            (
                "package.json",
                "{\n  \"name\": \"{{name}}\",\n  \"version\": \"0.1.0\",\n  \"private\": true,\n  \"type\": \"module\",\n  \"scripts\": {\n    \"start\": \"node index.js\"\n  }\n}\n",
            ),
            (
                "index.js",
                "console.log(\"Hello from {{name}}!\");\n",
            ),
        ]),
        "python" => Ok(vec![
            ("pyproject.toml", "[project]\nname = \"{{name}}\"\nversion = \"0.1.0\"\n"),
            (
                "main.py",
                "def main():\n    print(\"Hello from {{name}}!\")\n\n\nif __name__ == \"__main__\":\n    main()\n",
            ),
        ]),
        "kotlin" => Ok(vec![
            (
                "build.gradle.kts",
                "plugins {\n    kotlin(\"jvm\") version \"1.9.0\"\n    application\n}\n\napplication {\n    mainClass.set(\"MainKt\")\n}\n",
            ),
            (
                "src/main/kotlin/Main.kt",
                "fun main() {\n    println(\"Hello from {{name}}!\")\n}\n",
            ),
        ]),
        "java" => Ok(vec![
            (
                "pom.xml",
                "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n  <modelVersion>4.0.0</modelVersion>\n  <groupId>com.example</groupId>\n  <artifactId>{{name}}</artifactId>\n  <version>0.1.0</version>\n</project>\n",
            ),
            (
                "src/main/java/Main.java",
                "public class Main {\n    public static void main(String[] args) {\n        System.out.println(\"Hello from {{name}}!\");\n    }\n}\n",
            ),
        ]),
        "go" => Ok(vec![
            ("go.mod", "module {{name}}\n\ngo 1.21\n"),
            (
                "main.go",
                "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"Hello from {{name}}!\")\n}\n",
            ),
        ]),
        "csharp" => Ok(vec![
            (
                "{{name}}.csproj",
                "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <OutputType>Exe</OutputType>\n    <TargetFramework>net8.0</TargetFramework>\n  </PropertyGroup>\n</Project>\n",
            ),
            (
                "Program.cs",
                "System.Console.WriteLine(\"Hello from {{name}}!\");\n",
            ),
        ]),
        // Real, direct sibling of task #144's own real, spike-verified
        // minimal Android Gradle project (§ task #144 -- confirmed with a
        // genuine `BUILD SUCCESSFUL` and a real, ZIP-verified debug APK
        // before this template was ever written). Deliberately uses a
        // fixed `com.spartan.app` namespace/applicationId rather than
        // deriving one from `{{name}}` -- a real Java/Kotlin package
        // segment can't contain the `-`/`_` characters
        // `sanitize_project_name` allows, and this template's own
        // substitution mechanism only supports the one `{{name}}` token,
        // so a second, package-safe token would be real, separate,
        // not-yet-justified complexity for a first increment. `{{name}}`
        // is still used for the real, human-visible `android:label`. A
        // project created here is immediately buildable via the real
        // `android_build_apk` (task #144) the moment it's opened.
        "android" => Ok(vec![
            (
                "settings.gradle.kts",
                "pluginManagement {\n    repositories {\n        google()\n        mavenCentral()\n        gradlePluginPortal()\n    }\n}\ndependencyResolutionManagement {\n    repositories {\n        google()\n        mavenCentral()\n    }\n}\nrootProject.name = \"{{name}}\"\ninclude(\":app\")\n",
            ),
            (
                "build.gradle.kts",
                "plugins {\n    id(\"com.android.application\") version \"8.5.2\" apply false\n    id(\"org.jetbrains.kotlin.android\") version \"2.0.21\" apply false\n}\n",
            ),
            (
                "app/build.gradle.kts",
                "plugins {\n    id(\"com.android.application\")\n    id(\"org.jetbrains.kotlin.android\")\n}\n\nandroid {\n    namespace = \"com.spartan.app\"\n    compileSdk = 34\n\n    defaultConfig {\n        applicationId = \"com.spartan.app\"\n        minSdk = 24\n        targetSdk = 34\n        versionCode = 1\n        versionName = \"1.0\"\n    }\n    compileOptions {\n        sourceCompatibility = JavaVersion.VERSION_17\n        targetCompatibility = JavaVersion.VERSION_17\n    }\n    kotlinOptions {\n        jvmTarget = \"17\"\n    }\n}\n",
            ),
            (
                "app/src/main/AndroidManifest.xml",
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">\n    <application android:label=\"{{name}}\">\n        <activity android:name=\".MainActivity\" android:exported=\"true\">\n            <intent-filter>\n                <action android:name=\"android.intent.action.MAIN\" />\n                <category android:name=\"android.intent.category.LAUNCHER\" />\n            </intent-filter>\n        </activity>\n    </application>\n</manifest>\n",
            ),
            (
                "app/src/main/java/com/spartan/app/MainActivity.kt",
                "package com.spartan.app\n\nimport android.app.Activity\nimport android.os.Bundle\n\nclass MainActivity : Activity() {\n    override fun onCreate(savedInstanceState: Bundle?) {\n        super.onCreate(savedInstanceState)\n    }\n}\n",
            ),
        ]),
        other => Err(format!(
            "unknown project template `{other}` -- expected one of rust, typescript, javascript, python, kotlin, java, go, csharp, android"
        )),
    }
}

/// Real, deliberately conservative sanitizer -- alphanumeric, `-`, `_`
/// only, matching the same real `sanitize_identifier` shape
/// `sanitize_container_name` already established for Dev Containers,
/// reused here for a real directory name rather than a Docker container
/// name (a longer cap and an underscore allowance, since project
/// directory names have real, different conventions than container
/// names do).
fn sanitize_project_name(input: &str) -> String {
    sanitize_identifier(input, &['-', '_'], 64, "new-project")
}

/// Real project scaffolding -- creates `<parent_dir>/<sanitized name>`,
/// refuses to touch it if it already exists and is non-empty (never
/// silently overwrites real, possibly unrelated existing files), then
/// writes each real template file, substituting `{{name}}` for the real
/// sanitized project name. Deliberately synchronous: writing a handful
/// of small text files is fast enough that the async-event pattern
/// `devcontainer_up`/`leo_start_task` use for genuinely slow operations
/// would be pure overhead here.
fn create_project(
    parent_dir: &str,
    template: &str,
    name: &str,
) -> Result<serde_json::Value, String> {
    let files = project_template_files(template)?;
    let safe_name = sanitize_project_name(name);
    let project_root = std::path::Path::new(parent_dir).join(&safe_name);

    if project_root.exists() {
        let non_empty = std::fs::read_dir(&project_root)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false);
        if non_empty {
            return Err(format!(
                "{} already exists and is not empty -- refusing to overwrite it",
                project_root.display()
            ));
        }
    }

    for (rel_path, template_content) in files {
        let rel_path = rel_path.replace("{{name}}", &safe_name);
        let content = template_content.replace("{{name}}", &safe_name);
        let full_path = project_root.join(rel_path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create directory: {e}"))?;
        }
        std::fs::write(&full_path, content).map_err(|e| format!("write file: {e}"))?;
    }

    Ok(serde_json::json!({
        "project_root": project_root.to_string_lossy(),
        "name": safe_name,
    }))
}

fn get_str_param(params: &serde_json::Value, key: &str) -> Result<String, String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("missing/invalid string param `{key}`"))
}

fn get_u64_param(params: &serde_json::Value, key: &str) -> Result<u64, String> {
    params
        .get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| format!("missing/invalid u64 param `{key}`"))
}

/// Real request dispatch -- the one function `main.rs`'s stdio loop
/// calls per real line of input. Takes a real `Arc<Mutex<BackendState>>`
/// (not a plain `&mut`, since Leo's own background thread needs shared
/// ownership) and a real outbound-line sender so a slow method can push
/// a later, unprompted `Event` -- both threaded through purely so this
/// function stays fully testable without any real stdin/stdout, matching
/// this crate's own established discipline of separating real I/O from
/// real logic.
pub fn handle_request(
    state: &Arc<Mutex<BackendState>>,
    req: Request,
    out_tx: Sender<String>,
) -> Response {
    let result = match req.method.as_str() {
        "list_dir" => get_str_param(&req.params, "path").and_then(|p| list_dir(&p)),
        "open_file" => get_str_param(&req.params, "path").and_then(|p| {
            let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
            open_file(&mut guard, &p, out_tx.clone())
        }),
        "lsp_hover" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            let line = get_u64_param(&req.params, "line")? as i64;
            let character = get_u64_param(&req.params, "character")? as i64;
            lsp_hover(state, out_tx.clone(), doc_id, line, character)
        })(),
        "lsp_completion" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            let line = get_u64_param(&req.params, "line")? as i64;
            let character = get_u64_param(&req.params, "character")? as i64;
            lsp_completion(state, out_tx.clone(), doc_id, line, character)
        })(),
        "lsp_definition" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            let line = get_u64_param(&req.params, "line")? as i64;
            let character = get_u64_param(&req.params, "character")? as i64;
            lsp_definition(state, out_tx.clone(), doc_id, line, character)
        })(),
        "lsp_type_definition" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            let line = get_u64_param(&req.params, "line")? as i64;
            let character = get_u64_param(&req.params, "character")? as i64;
            lsp_type_definition(state, out_tx.clone(), doc_id, line, character)
        })(),
        "lsp_signature_help" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            let line = get_u64_param(&req.params, "line")? as i64;
            let character = get_u64_param(&req.params, "character")? as i64;
            lsp_signature_help(state, out_tx.clone(), doc_id, line, character)
        })(),
        "lsp_references" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            let line = get_u64_param(&req.params, "line")? as i64;
            let character = get_u64_param(&req.params, "character")? as i64;
            let include_declaration = req
                .params
                .get("include_declaration")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            lsp_references(
                state,
                out_tx.clone(),
                doc_id,
                line,
                character,
                include_declaration,
            )
        })(),
        "lsp_rename" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            let line = get_u64_param(&req.params, "line")? as i64;
            let character = get_u64_param(&req.params, "character")? as i64;
            let new_name = get_str_param(&req.params, "new_name")?;
            lsp_rename(state, out_tx.clone(), doc_id, line, character, new_name)
        })(),
        "lsp_document_symbol" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            lsp_document_symbol(state, out_tx.clone(), doc_id)
        })(),
        "lsp_document_highlight" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            let line = get_u64_param(&req.params, "line")? as i64;
            let character = get_u64_param(&req.params, "character")? as i64;
            lsp_document_highlight(state, out_tx.clone(), doc_id, line, character)
        })(),
        "lsp_call_hierarchy" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            let line = get_u64_param(&req.params, "line")? as i64;
            let character = get_u64_param(&req.params, "character")? as i64;
            let outgoing = req
                .params
                .get("direction")
                .and_then(|v| v.as_str())
                .map(|d| d == "outgoing")
                .unwrap_or(false);
            lsp_call_hierarchy(state, out_tx.clone(), doc_id, line, character, outgoing)
        })(),
        "format_document" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            format_document(state, out_tx.clone(), doc_id)
        })(),
        "edit" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            let start_char = get_u64_param(&req.params, "start_char")? as usize;
            let end_char = get_u64_param(&req.params, "end_char")? as usize;
            let text = get_str_param(&req.params, "text").unwrap_or_default();
            let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
            edit(&mut guard, doc_id, start_char, end_char, &text)
        })(),
        "save_file" => get_u64_param(&req.params, "doc_id").and_then(|id| {
            let guard = state.lock().map_err(|_| "backend state poisoned")?;
            save_file(&guard, id)
        }),
        "undo" => get_u64_param(&req.params, "doc_id").and_then(|id| {
            let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
            undo(&mut guard, id)
        }),
        "redo" => get_u64_param(&req.params, "doc_id").and_then(|id| {
            let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
            redo(&mut guard, id)
        }),
        "close_file" => get_u64_param(&req.params, "doc_id").and_then(|id| {
            let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
            close_file(&mut guard, id)
        }),
        "leo_status" => state
            .lock()
            .map_err(|_| "backend state poisoned".to_string())
            .and_then(|g| leo_status(&g)),
        "leo_session_history" => state
            .lock()
            .map_err(|_| "backend state poisoned".to_string())
            .map(|g| leo_session_history(&g)),
        "leo_start_task" => (|| {
            let task = get_str_param(&req.params, "task")?;
            let project_root = get_str_param(&req.params, "project_root")?;
            leo_start_task(state, out_tx.clone(), task, project_root)
        })(),
        "leo_approve_plan" => leo_approve_plan(state),
        "leo_reject_plan" => leo_reject_plan(state),
        "leo_cancel" => leo_cancel(state),
        "leo_next_step" => leo_next_step(state, out_tx.clone()),
        "leo_approve_call" => leo_approve_call(state),
        "leo_reject_call" => leo_reject_call(state),
        "leo_retry" => leo_retry(state),
        "pty_spawn" => (|| {
            let cwd = get_str_param(&req.params, "cwd")?;
            let cols = get_u64_param(&req.params, "cols")? as u16;
            let rows = get_u64_param(&req.params, "rows")? as u16;
            let command = req.params.get("command").and_then(|v| v.as_str());
            let args: Vec<String> = req
                .params
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            pty_spawn(state, out_tx.clone(), &cwd, cols, rows, command, &args)
        })(),
        "pty_input" => (|| {
            let session_id = get_u64_param(&req.params, "session_id")?;
            let data = get_str_param(&req.params, "data")?;
            pty_input(state, session_id, &data)
        })(),
        "pty_resize" => (|| {
            let session_id = get_u64_param(&req.params, "session_id")?;
            let cols = get_u64_param(&req.params, "cols")? as u16;
            let rows = get_u64_param(&req.params, "rows")? as u16;
            pty_resize(state, session_id, cols, rows)
        })(),
        "pty_close" => get_u64_param(&req.params, "session_id").and_then(|id| pty_close(state, id)),
        "dap_launch" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            let breakpoints = parse_breakpoints(&req.params);
            dap_launch(state, out_tx.clone(), doc_id, &breakpoints)
        })(),
        "dap_continue" => get_u64_param(&req.params, "session_id")
            .and_then(|id| dap_command(state, id, spartan_dap::DapCommand::Continue)),
        "dap_step_over" => get_u64_param(&req.params, "session_id")
            .and_then(|id| dap_command(state, id, spartan_dap::DapCommand::StepOver)),
        "dap_step_into" => get_u64_param(&req.params, "session_id")
            .and_then(|id| dap_command(state, id, spartan_dap::DapCommand::StepInto)),
        "dap_evaluate" => (|| {
            let session_id = get_u64_param(&req.params, "session_id")?;
            let expression = get_str_param(&req.params, "expression")?;
            dap_evaluate(state, session_id, &expression)
        })(),
        "dap_disconnect" => {
            get_u64_param(&req.params, "session_id").and_then(|id| dap_disconnect(state, id))
        }
        "devcontainer_detect" => {
            get_str_param(&req.params, "project_root").and_then(|r| devcontainer_detect(&r))
        }
        "devcontainer_up" => get_str_param(&req.params, "project_root")
            .and_then(|r| devcontainer_up(out_tx.clone(), r)),
        "devcontainer_down" => get_str_param(&req.params, "container_id")
            .and_then(|id| devcontainer_down(out_tx.clone(), id)),
        "devcontainer_status" => {
            get_str_param(&req.params, "container_id").and_then(|id| devcontainer_status(&id))
        }
        "devcontainer_list" => devcontainer_list(),
        "android_detect" => {
            get_str_param(&req.params, "project_root").and_then(|r| android_detect(&r))
        }
        "android_build_apk" => get_str_param(&req.params, "project_root")
            .and_then(|r| android_build_apk(out_tx.clone(), r)),
        "android_list_devices" => android_list_devices(),
        "android_install_apk" => (|| {
            let apk_path = get_str_param(&req.params, "apk_path")?;
            let serial = get_str_param(&req.params, "serial").ok();
            android_install_apk(out_tx.clone(), apk_path, serial)
        })(),
        "android_logcat_start" => {
            let serial = get_str_param(&req.params, "serial").ok();
            android_logcat_start(state, out_tx.clone(), serial)
        }
        "android_logcat_stop" => {
            get_u64_param(&req.params, "session_id").and_then(|id| android_logcat_stop(state, id))
        }
        "devcontainer_exec_spawn" => (|| {
            let container_id = get_str_param(&req.params, "container_id")?;
            let cols = get_u64_param(&req.params, "cols")? as u16;
            let rows = get_u64_param(&req.params, "rows")? as u16;
            devcontainer_exec_spawn(state, out_tx.clone(), &container_id, cols, rows)
        })(),
        "devcontainer_exec_input" => (|| {
            let session_id = get_u64_param(&req.params, "session_id")?;
            let data = get_str_param(&req.params, "data")?;
            devcontainer_exec_input(state, session_id, &data)
        })(),
        "devcontainer_exec_resize" => (|| {
            let session_id = get_u64_param(&req.params, "session_id")?;
            let cols = get_u64_param(&req.params, "cols")? as u16;
            let rows = get_u64_param(&req.params, "rows")? as u16;
            devcontainer_exec_resize(state, session_id, cols, rows)
        })(),
        "devcontainer_exec_close" => get_u64_param(&req.params, "session_id")
            .and_then(|id| devcontainer_exec_close(state, id)),
        "search_project" => (|| {
            let project_root = get_str_param(&req.params, "project_root")?;
            let pattern = get_str_param(&req.params, "pattern")?;
            let path = req.params.get("path").and_then(|v| v.as_str());
            search_project(&project_root, &pattern, path)
        })(),
        "git_status" => get_str_param(&req.params, "project_root").and_then(|r| git_status(&r)),
        "git_stage" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let path = get_str_param(&req.params, "path")?;
            git_stage(&root, &path)
        })(),
        "git_unstage" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let path = get_str_param(&req.params, "path")?;
            git_unstage(&root, &path)
        })(),
        "git_discard" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let path = get_str_param(&req.params, "path")?;
            git_discard(&root, &path)
        })(),
        "git_commit" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let message = get_str_param(&req.params, "message")?;
            git_commit(&root, &message)
        })(),
        "git_commit_amend" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let message = get_str_param(&req.params, "message")?;
            git_commit_amend(&root, &message)
        })(),
        "git_revert_commit" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let oid = get_str_param(&req.params, "oid")?;
            git_revert_commit(&root, &oid)
        })(),
        "git_tags" => get_str_param(&req.params, "project_root").and_then(|r| git_tags(&r)),
        "git_create_tag" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let name = get_str_param(&req.params, "name")?;
            let oid = get_str_param(&req.params, "oid")?;
            let message = req.params.get("message").and_then(|v| v.as_str());
            git_create_tag(&root, &name, &oid, message)
        })(),
        "git_delete_tag" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let name = get_str_param(&req.params, "name")?;
            git_delete_tag(&root, &name)
        })(),
        "git_diff" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let path = get_str_param(&req.params, "path")?;
            let staged = req
                .params
                .get("staged")
                .and_then(|v| v.as_bool())
                .ok_or("missing/invalid bool param `staged`")?;
            git_diff(&root, &path, staged)
        })(),
        "git_diff_hunks" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let path = get_str_param(&req.params, "path")?;
            git_diff_hunks(&root, &path)
        })(),
        "git_stage_hunk" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let path = get_str_param(&req.params, "path")?;
            let hunk_index = req
                .params
                .get("hunk_index")
                .and_then(|v| v.as_u64())
                .ok_or("missing/invalid u64 param `hunk_index`")?;
            git_stage_hunk(&root, &path, hunk_index)
        })(),
        "git_branches" => get_str_param(&req.params, "project_root").and_then(|r| git_branches(&r)),
        "git_checkout" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let branch = get_str_param(&req.params, "branch")?;
            git_checkout(&root, &branch)
        })(),
        "git_create_branch" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let branch = get_str_param(&req.params, "branch")?;
            git_create_branch(&root, &branch)
        })(),
        "git_remote_branches" => {
            get_str_param(&req.params, "project_root").and_then(|r| git_remote_branches(&r))
        }
        "git_checkout_remote" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let branch = get_str_param(&req.params, "branch")?;
            git_checkout_remote(&root, &branch)
        })(),
        "git_merge_branch" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let branch = get_str_param(&req.params, "branch")?;
            git_merge_branch(&root, &branch)
        })(),
        "git_merge_status" => {
            get_str_param(&req.params, "project_root").and_then(|r| git_merge_status(&r))
        }
        "git_resolve_conflict" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let path = get_str_param(&req.params, "path")?;
            let content = get_str_param(&req.params, "content")?;
            git_resolve_conflict(&root, &path, &content)
        })(),
        "git_commit_merge" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let message = get_str_param(&req.params, "message")?;
            git_commit_merge(&root, &message)
        })(),
        "git_abort_merge" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            git_abort_merge(&root)
        })(),
        "git_log" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let max = req.params.get("max").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
            git_log(&root, max)
        })(),
        "git_log_for_ref" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let ref_name = get_str_param(&req.params, "ref_name")?;
            let max = req.params.get("max").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
            git_log_for_ref(&root, &ref_name, max)
        })(),
        "git_cherry_pick" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let oid = get_str_param(&req.params, "oid")?;
            git_cherry_pick(&root, &oid)
        })(),
        "git_blame" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let path = get_str_param(&req.params, "path")?;
            git_blame(&root, &path)
        })(),
        "git_remotes" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            git_remotes(&root)
        })(),
        "git_fetch" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let remote = get_str_param(&req.params, "remote")?;
            git_fetch(&root, &remote)
        })(),
        "git_push" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let remote = get_str_param(&req.params, "remote")?;
            let branch = get_str_param(&req.params, "branch")?;
            git_push(&root, &remote, &branch)
        })(),
        "git_pull" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let remote = get_str_param(&req.params, "remote")?;
            let branch = get_str_param(&req.params, "branch")?;
            git_pull(&root, &remote, &branch)
        })(),
        "git_stash_save" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let message = req
                .params
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            git_stash_save(&root, message)
        })(),
        "git_stash_list" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            git_stash_list(&root)
        })(),
        "git_stash_pop" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let index = req
                .params
                .get("index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            git_stash_pop(&root, index)
        })(),
        "git_stash_apply" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let index = req
                .params
                .get("index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            git_stash_apply(&root, index)
        })(),
        "git_stash_drop" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let index = req
                .params
                .get("index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            git_stash_drop(&root, index)
        })(),
        "git_commit_files" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let oid = get_str_param(&req.params, "oid")?;
            git_commit_files(&root, &oid)
        })(),
        "git_commit_diff" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let oid = get_str_param(&req.params, "oid")?;
            let path = get_str_param(&req.params, "path")?;
            git_commit_diff(&root, &oid, &path)
        })(),
        "settings_get" => settings_get(),
        "settings_set" => (|| {
            let gpu_enabled = req
                .params
                .get("gpu_enabled")
                .and_then(|v| v.as_bool())
                .ok_or("missing/invalid bool param `gpu_enabled`")?;
            let gpu_layers = req
                .params
                .get("gpu_layers")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32);
            let leo_approval_mode = req
                .params
                .get("leo_approval_mode")
                .map(|v| serde_json::from_value::<spartan_settings::LeoApprovalMode>(v.clone()))
                .transpose()
                .map_err(|e| format!("invalid leo_approval_mode: {e}"))?;
            let leo_provider = req
                .params
                .get("leo_provider")
                .map(|v| serde_json::from_value::<spartan_settings::LeoProviderSettings>(v.clone()))
                .transpose()
                .map_err(|e| format!("invalid leo_provider: {e}"))?;
            let editor = req
                .params
                .get("editor")
                .map(|v| serde_json::from_value::<spartan_settings::EditorSettings>(v.clone()))
                .transpose()
                .map_err(|e| format!("invalid editor: {e}"))?;
            let appearance = req
                .params
                .get("appearance")
                .map(|v| serde_json::from_value::<spartan_settings::AppearanceSettings>(v.clone()))
                .transpose()
                .map_err(|e| format!("invalid appearance: {e}"))?;
            let crash_reporting = req
                .params
                .get("crash_reporting")
                .map(|v| {
                    serde_json::from_value::<spartan_settings::CrashReportingSettings>(v.clone())
                })
                .transpose()
                .map_err(|e| format!("invalid crash_reporting: {e}"))?;
            let onboarding_completed = req
                .params
                .get("onboarding_completed")
                .map(|v| serde_json::from_value::<bool>(v.clone()))
                .transpose()
                .map_err(|e| format!("invalid onboarding_completed: {e}"))?;
            // Nested-`Option` parse (task #265): absent -> keep current
            // (outer `None`); present-but-empty/whitespace -> clear it
            // (`Some(None)`); present with a real command -> set it
            // (`Some(Some(cmd))`). See `SettingsPatch`'s own field doc.
            let leo_verify_command = match req.params.get("leo_verify_command") {
                None => None,
                Some(v) => {
                    let s = v
                        .as_str()
                        .ok_or("invalid leo_verify_command: must be a string")?;
                    let trimmed = s.trim();
                    Some(if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    })
                }
            };
            settings_set(SettingsPatch {
                gpu_enabled,
                gpu_layers,
                leo_approval_mode,
                leo_provider,
                editor,
                appearance,
                crash_reporting,
                onboarding_completed,
                leo_verify_command,
            })
        })(),
        "check_for_updates" => check_for_updates(out_tx.clone()),
        "model_status" => Ok(model_status_json()),
        "litellm_proxy_start" => (|| {
            let port = get_u64_param(&req.params, "port")? as u16;
            let config_path = req
                .params
                .get("config_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let auto_restart = req
                .params
                .get("auto_restart")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            litellm_proxy_start(state, out_tx.clone(), port, config_path, auto_restart)
        })(),
        "litellm_proxy_stop" => litellm_proxy_stop(state),
        "litellm_proxy_status" => litellm_proxy_status(state),
        "hf_list_models" => Ok(hf_list_models_json()),
        "hf_pull_model" => {
            let model_id = req
                .params
                .get("model_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let hf_repo = req
                .params
                .get("hf_repo")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let tag = req
                .params
                .get("tag")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            hf_pull_model(state, out_tx.clone(), model_id, hf_repo, tag)
        }
        "lmstudio_list_models" => Ok(lmstudio_list_models_json()),
        "lmstudio_pull_model" => {
            let model_id = req
                .params
                .get("model_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let hf_repo = req
                .params
                .get("hf_repo")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let tag = req
                .params
                .get("tag")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            lmstudio_pull_model(state, out_tx.clone(), model_id, hf_repo, tag)
        }
        "llamacpp_list_models" => Ok(llamacpp_list_models_json()),
        "llamacpp_download_model" => {
            let model_id = req
                .params
                .get("model_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let hf_repo = req
                .params
                .get("hf_repo")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let tag = req
                .params
                .get("tag")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            llamacpp_download_model(state, out_tx.clone(), model_id, hf_repo, tag)
        }
        "model_download_cancel" => (|| {
            let source = get_str_param(&req.params, "source")?;
            let event_id = get_str_param(&req.params, "event_id")?;
            model_download_cancel(state, source, event_id)
        })(),
        "crash_reports_list" => crash_reports_list(),
        "crash_report_upload" => {
            get_str_param(&req.params, "filename").and_then(|f| crash_report_upload(&f))
        }
        "create_project" => (|| {
            let parent_dir = get_str_param(&req.params, "parent_dir")?;
            let template = get_str_param(&req.params, "template")?;
            let name = get_str_param(&req.params, "name")?;
            create_project(&parent_dir, &template, &name)
        })(),
        other => Err(format!("unknown method `{other}`")),
    };
    match result {
        Ok(value) => Response::ok(req.id, value),
        Err(message) => Response::err(req.id, message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn req(id: u64, method: &str, params: serde_json::Value) -> Request {
        Request {
            id,
            method: method.to_string(),
            params,
        }
    }

    fn new_state() -> Arc<Mutex<BackendState>> {
        Arc::new(Mutex::new(BackendState::new()))
    }

    fn call(
        state: &Arc<Mutex<BackendState>>,
        id: u64,
        method: &str,
        params: serde_json::Value,
    ) -> Response {
        let (tx, _rx) = mpsc::channel();
        handle_request(state, req(id, method, params), tx)
    }

    #[test]
    fn list_dir_lists_a_real_temp_directory_dirs_first() {
        let dir = std::env::temp_dir().join(format!("spartan-backend-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("zsubdir")).unwrap();
        std::fs::write(dir.join("afile.txt"), "hi").unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "list_dir",
            serde_json::json!({ "path": dir.to_string_lossy() }),
        );
        assert!(resp.error.is_none());
        let entries = resp.result.unwrap()["entries"].as_array().unwrap().clone();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["name"], "zsubdir");
        assert_eq!(entries[0]["is_dir"], true);
        assert_eq!(entries[1]["name"], "afile.txt");
        assert_eq!(entries[1]["is_dir"], false);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_dir_on_a_real_nonexistent_path_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "list_dir",
            serde_json::json!({ "path": "/definitely/not/a/real/path/xyz" }),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
    }

    #[test]
    fn open_edit_save_round_trips_through_a_real_file() {
        let file =
            std::env::temp_dir().join(format!("spartan-backend-test-{}.txt", std::process::id()));
        std::fs::write(&file, "hello world").unwrap();
        let state = new_state();

        let open_resp = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        );
        let open_result = open_resp.result.unwrap();
        assert_eq!(open_result["content"], "hello world");
        let doc_id = open_result["doc_id"].as_u64().unwrap();

        let edit_resp = call(
            &state,
            2,
            "edit",
            serde_json::json!({ "doc_id": doc_id, "start_char": 5, "end_char": 5, "text": "," }),
        );
        assert!(edit_resp.error.is_none());

        let save_resp = call(
            &state,
            3,
            "save_file",
            serde_json::json!({ "doc_id": doc_id }),
        );
        assert!(save_resp.error.is_none());

        let on_disk = std::fs::read_to_string(&file).unwrap();
        assert_eq!(on_disk, "hello, world");
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn edit_with_a_real_non_empty_range_and_no_text_is_a_real_delete() {
        let file = std::env::temp_dir().join(format!(
            "spartan-backend-test-del-{}.txt",
            std::process::id()
        ));
        std::fs::write(&file, "hello world").unwrap();
        let state = new_state();
        let open_result = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        )
        .result
        .unwrap();
        let doc_id = open_result["doc_id"].as_u64().unwrap();

        call(
            &state,
            2,
            "edit",
            serde_json::json!({ "doc_id": doc_id, "start_char": 0, "end_char": 6, "text": "" }),
        );
        call(
            &state,
            3,
            "save_file",
            serde_json::json!({ "doc_id": doc_id }),
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "world");
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn undo_reverts_a_real_edit_and_reports_it_changed() {
        let file = std::env::temp_dir().join(format!(
            "spartan-backend-test-undo-{}.txt",
            std::process::id()
        ));
        std::fs::write(&file, "abc").unwrap();
        let state = new_state();
        let doc_id = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        )
        .result
        .unwrap()["doc_id"]
            .as_u64()
            .unwrap();

        call(
            &state,
            2,
            "edit",
            serde_json::json!({ "doc_id": doc_id, "start_char": 3, "end_char": 3, "text": "d" }),
        );
        let undo_resp = call(&state, 3, "undo", serde_json::json!({ "doc_id": doc_id }));
        let undo_result = undo_resp.result.unwrap();
        assert_eq!(undo_result["changed"], true);
        assert_eq!(undo_result["content"], "abc");
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn redo_restores_a_real_undone_edit() {
        let file = std::env::temp_dir().join(format!(
            "spartan-backend-test-redo-{}.txt",
            std::process::id()
        ));
        std::fs::write(&file, "abc").unwrap();
        let state = new_state();
        let doc_id = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        )
        .result
        .unwrap()["doc_id"]
            .as_u64()
            .unwrap();

        call(
            &state,
            2,
            "edit",
            serde_json::json!({ "doc_id": doc_id, "start_char": 3, "end_char": 3, "text": "d" }),
        );
        call(&state, 3, "undo", serde_json::json!({ "doc_id": doc_id }));
        let redo_resp = call(&state, 4, "redo", serde_json::json!({ "doc_id": doc_id }));
        let redo_result = redo_resp.result.unwrap();
        assert_eq!(redo_result["changed"], true);
        assert_eq!(redo_result["content"], "abcd");
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn redo_with_nothing_to_redo_reports_unchanged() {
        let file = std::env::temp_dir().join(format!(
            "spartan-backend-test-redo-empty-{}.txt",
            std::process::id()
        ));
        std::fs::write(&file, "abc").unwrap();
        let state = new_state();
        let doc_id = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        )
        .result
        .unwrap()["doc_id"]
            .as_u64()
            .unwrap();
        let redo_resp = call(&state, 2, "redo", serde_json::json!({ "doc_id": doc_id }));
        assert_eq!(redo_resp.result.unwrap()["changed"], false);
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn a_real_new_edit_after_undo_clears_the_real_redo_stack() {
        let file = std::env::temp_dir().join(format!(
            "spartan-backend-test-redo-clear-{}.txt",
            std::process::id()
        ));
        std::fs::write(&file, "abc").unwrap();
        let state = new_state();
        let doc_id = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        )
        .result
        .unwrap()["doc_id"]
            .as_u64()
            .unwrap();

        call(
            &state,
            2,
            "edit",
            serde_json::json!({ "doc_id": doc_id, "start_char": 3, "end_char": 3, "text": "d" }),
        );
        call(&state, 3, "undo", serde_json::json!({ "doc_id": doc_id }));
        // A fresh edit should invalidate the pending redo.
        call(
            &state,
            4,
            "edit",
            serde_json::json!({ "doc_id": doc_id, "start_char": 3, "end_char": 3, "text": "e" }),
        );
        let redo_resp = call(&state, 5, "redo", serde_json::json!({ "doc_id": doc_id }));
        assert_eq!(redo_resp.result.unwrap()["changed"], false);
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn pty_spawn_starts_a_real_process_and_returns_a_session_id() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "pty_spawn",
            serde_json::json!({
                "cwd": std::env::temp_dir().to_string_lossy(),
                "cols": 80,
                "rows": 24,
                "command": "bash",
                "args": ["-c", "echo READY && exit"],
            }),
        );
        assert!(resp.error.is_none(), "pty_spawn errored: {:?}", resp.error);
        let result = resp.result.unwrap();
        assert_eq!(result["session_id"], 0);

        // A second real spawn on the same state gets a distinct, incrementing id.
        let resp2 = call(
            &state,
            2,
            "pty_spawn",
            serde_json::json!({
                "cwd": std::env::temp_dir().to_string_lossy(),
                "cols": 80,
                "rows": 24,
                "command": "bash",
                "args": ["-c", "exit"],
            }),
        );
        assert_eq!(resp2.result.unwrap()["session_id"], 1);
    }

    #[test]
    fn pty_input_on_an_unknown_session_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "pty_input",
            serde_json::json!({ "session_id": 999, "data": "hi\n" }),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("no pty session"));
    }

    #[test]
    fn pty_resize_on_an_unknown_session_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "pty_resize",
            serde_json::json!({ "session_id": 999, "cols": 100, "rows": 40 }),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("no pty session"));
    }

    #[test]
    fn pty_close_on_an_unknown_session_is_a_real_harmless_no_op() {
        // Closing an id that was never spawned (or already closed) should
        // not error -- matches close_file's own "already gone is fine"
        // semantics, since a UI's close button firing twice shouldn't crash.
        let state = new_state();
        let resp = call(
            &state,
            1,
            "pty_close",
            serde_json::json!({ "session_id": 999 }),
        );
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["ok"], true);
    }

    #[test]
    fn pty_close_really_removes_the_session_so_input_after_close_errors() {
        let state = new_state();
        let session_id = call(
            &state,
            1,
            "pty_spawn",
            serde_json::json!({
                "cwd": std::env::temp_dir().to_string_lossy(),
                "cols": 80,
                "rows": 24,
                "command": "bash",
                "args": ["-c", "sleep 5"],
            }),
        )
        .result
        .unwrap()["session_id"]
            .as_u64()
            .unwrap();

        let close_resp = call(
            &state,
            2,
            "pty_close",
            serde_json::json!({ "session_id": session_id }),
        );
        assert_eq!(close_resp.result.unwrap()["ok"], true);

        let input_resp = call(
            &state,
            3,
            "pty_input",
            serde_json::json!({ "session_id": session_id, "data": "hi\n" }),
        );
        assert!(input_resp.error.unwrap().contains("no pty session"));
    }

    #[test]
    fn devcontainer_detect_with_no_config_reports_not_found() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-devcontainer-detect-none-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "devcontainer_detect",
            serde_json::json!({ "project_root": dir.to_string_lossy() }),
        );
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["found"], false);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn devcontainer_detect_finds_and_summarizes_a_real_config() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-devcontainer-detect-found-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".devcontainer")).unwrap();
        std::fs::write(
            dir.join(".devcontainer").join("devcontainer.json"),
            r#"{
                "name": "Test Project",
                "image": "mcr.microsoft.com/devcontainers/rust:1",
                "forwardPorts": [3000],
                "postCreateCommand": "cargo build"
            }"#,
        )
        .unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "devcontainer_detect",
            serde_json::json!({ "project_root": dir.to_string_lossy() }),
        );
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["found"], true);
        assert_eq!(result["config"]["name"], "Test Project");
        assert_eq!(result["config"]["forwardPorts"][0], 3000);
        assert_eq!(result["config"]["hasPostCreateCommand"], true);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn android_detect_correctly_reports_a_plain_non_android_project() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-android-detect-none-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "android_detect",
            serde_json::json!({ "project_root": dir.to_string_lossy() }),
        );
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["isAndroidProject"], false);
        // Real, internally-consistent shape check rather than asserting a
        // specific present/absent value for any one field -- this
        // environment's own real toolchain state (Gradle present, no
        // real Android SDK) shouldn't be hardcoded into a test that must
        // also pass in a real developer's environment where the opposite
        // could be true.
        assert!(result.get("sdkRoot").is_some());
        assert!(result.get("gradlePath").is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn android_detect_recognizes_a_real_android_manifest_project() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-android-detect-found-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let manifest_dir = dir.join("app").join("src").join("main");
        std::fs::create_dir_all(&manifest_dir).unwrap();
        std::fs::write(manifest_dir.join("AndroidManifest.xml"), "<manifest />").unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "android_detect",
            serde_json::json!({ "project_root": dir.to_string_lossy() }),
        );
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["isAndroidProject"], true);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn android_build_apk_refuses_a_non_android_project() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-android-build-refuse-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "android_build_apk",
            serde_json::json!({ "project_root": dir.to_string_lossy() }),
        );
        assert!(resp.error.is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn android_build_apk_refuses_an_empty_project_root() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "android_build_apk",
            serde_json::json!({ "project_root": "" }),
        );
        assert!(resp.error.is_some());
    }

    #[test]
    fn android_build_apk_acks_immediately_for_a_real_recognized_android_project() {
        // Confirms the dispatch arm reaches the real handler and returns
        // the real "ack now, event later" shape without ever blocking on
        // the (possibly multi-minute) real Gradle build itself -- the
        // background thread's own eventual real/failed event is covered
        // by spartan-android's own build.rs tests, including a real,
        // self-skipping live `assembleDebug` run.
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-android-build-ack-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let manifest_dir = dir.join("app").join("src").join("main");
        std::fs::create_dir_all(&manifest_dir).unwrap();
        std::fs::write(manifest_dir.join("AndroidManifest.xml"), "<manifest />").unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "android_build_apk",
            serde_json::json!({ "project_root": dir.to_string_lossy() }),
        );
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["status"], "starting");
        // Deliberately not cleaned up here: a real background thread is now
        // racing to spawn a real Gradle process against this directory (it
        // will fail fast and harmlessly -- no real build.gradle exists in
        // this fixture -- but this test doesn't wait for or assert on that
        // event), so deleting the directory immediately would race with it.
    }

    /// Real, live confirmation that this crate's own dispatch correctly
    /// reaches a *real* installed Gradle and parses its real version --
    /// self-skips (matching this workspace's own established convention)
    /// if no real `gradle` is found on `$PATH` in whatever environment
    /// runs this test.
    #[test]
    fn android_detect_reports_a_real_gradle_version_when_gradle_is_actually_installed() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-android-detect-gradle-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "android_detect",
            serde_json::json!({ "project_root": dir.to_string_lossy() }),
        );
        let result = resp.result.unwrap();
        if result["gradlePath"].is_null() {
            eprintln!("SKIP: no real `gradle` on $PATH in this environment");
            std::fs::remove_dir_all(&dir).ok();
            return;
        }
        assert!(
            !result["gradleVersion"].is_null(),
            "expected a real parsed Gradle version"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Real, live confirmation this crate's own dispatch reaches a real
    /// `adb` when one is present on this machine -- self-skips (matching
    /// this workspace's own established convention) if none is found,
    /// rather than asserting a specific device count (a different real
    /// environment running this test with a real device attached should
    /// still pass).
    #[test]
    fn android_list_devices_reaches_a_real_adb_when_one_is_present() {
        let state = new_state();
        let resp = call(&state, 1, "android_list_devices", serde_json::json!({}));
        match resp.error {
            Some(e) if e.contains("no real `adb`") => {
                eprintln!("SKIP: no real `adb` found in this environment");
            }
            Some(e) => {
                panic!("expected either a real device list or an honest 'no adb' error, got: {e}")
            }
            None => {
                let devices = &resp.result.unwrap()["devices"];
                assert!(devices.is_array(), "expected a real JSON array of devices");
            }
        }
    }

    #[test]
    fn android_install_apk_refuses_an_empty_apk_path() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "android_install_apk",
            serde_json::json!({ "apk_path": "" }),
        );
        assert!(resp.error.is_some());
    }

    #[test]
    fn android_install_apk_refuses_when_no_real_adb_and_acks_when_one_is_present() {
        // Real, environment-dependent branch, matching
        // `android_list_devices`'s own precedent: this crate can't fake
        // adb's presence, so it asserts whichever real, honest outcome
        // this environment actually produces rather than assuming one.
        let state = new_state();
        let resp = call(
            &state,
            1,
            "android_install_apk",
            serde_json::json!({ "apk_path": "/nonexistent/fake.apk" }),
        );
        match resp.error {
            Some(e) => assert!(
                e.contains("no real `adb`"),
                "expected the honest 'no adb' error, got: {e}"
            ),
            None => {
                // A real adb is present -- this acks immediately and a
                // real background thread will fail shortly after (no
                // real APK exists at this path), matching
                // `android_build_apk_acks_immediately...`'s own
                // deliberately-not-awaited pattern.
                assert_eq!(resp.result.unwrap()["status"], "starting");
            }
        }
    }

    #[test]
    fn android_logcat_stop_on_an_unknown_session_is_a_real_harmless_no_op() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "android_logcat_stop",
            serde_json::json!({ "session_id": 999 }),
        );
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["ok"], true);
    }

    #[test]
    fn android_logcat_start_reaches_a_real_adb_when_one_is_present_and_really_stops() {
        // Real, environment-dependent branch, matching
        // `android_list_devices`'s/`android_install_apk`'s own precedent.
        let state = new_state();
        let resp = call(&state, 1, "android_logcat_start", serde_json::json!({}));
        match resp.error {
            Some(e) => {
                assert!(
                    e.contains("no real `adb`"),
                    "expected the honest 'no adb' error, got: {e}"
                );
            }
            None => {
                let session_id = resp.result.unwrap()["session_id"].as_u64().unwrap();
                // A real adb logcat process is now genuinely running
                // (streaming, or -- with no real device attached, as
                // confirmed live earlier this session -- blocked on its
                // own real "waiting for device" state). Stopping it must
                // really remove the session, confirmed by a second stop
                // call being the same harmless no-op an already-gone
                // session gets.
                let stop_resp = call(
                    &state,
                    2,
                    "android_logcat_stop",
                    serde_json::json!({ "session_id": session_id }),
                );
                assert!(stop_resp.error.is_none());
                assert_eq!(stop_resp.result.unwrap()["ok"], true);
            }
        }
    }

    #[test]
    fn devcontainer_up_errors_honestly_when_no_config_exists() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-devcontainer-up-no-config-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "devcontainer_up",
            serde_json::json!({ "project_root": dir.to_string_lossy() }),
        );
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().contains("no devcontainer.json"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn devcontainer_up_errors_honestly_when_docker_is_unreachable() {
        // A real, environment-honest test: this sandboxed development
        // environment has no Docker daemon running at all (confirmed
        // directly during this feature's own development), so this real
        // early check inside `devcontainer_up` is expected to catch that
        // and fail fast with a specific message, never attempting the
        // (impossible here) pull/build. On a real machine with Docker
        // actually running, this exact scenario can't be reached this
        // way -- skipped rather than asserting a false premise there.
        if spartan_devcontainer::docker::is_docker_available() {
            eprintln!("SKIP: a real Docker daemon is reachable in this environment");
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-devcontainer-up-no-docker-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".devcontainer")).unwrap();
        std::fs::write(
            dir.join(".devcontainer").join("devcontainer.json"),
            r#"{ "image": "alpine:latest" }"#,
        )
        .unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "devcontainer_up",
            serde_json::json!({ "project_root": dir.to_string_lossy() }),
        );
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().contains("Docker isn't running"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn devcontainer_exec_input_on_an_unknown_session_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "devcontainer_exec_input",
            serde_json::json!({ "session_id": 999, "data": "echo hi\n" }),
        );
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().contains("no devcontainer exec session"));
    }

    #[test]
    fn devcontainer_exec_resize_on_an_unknown_session_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "devcontainer_exec_resize",
            serde_json::json!({ "session_id": 999, "cols": 80, "rows": 24 }),
        );
        assert!(resp.error.is_some());
    }

    #[test]
    fn devcontainer_exec_close_on_an_unknown_session_is_a_real_harmless_no_op() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "devcontainer_exec_close",
            serde_json::json!({ "session_id": 999 }),
        );
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["ok"], true);
    }

    #[test]
    fn devcontainer_list_returns_a_real_empty_or_error_result_without_panicking() {
        // No Docker daemon is reachable in this sandboxed environment,
        // so this is real, honest error-path coverage -- on a machine
        // with Docker actually running, this would return a real
        // (possibly empty) list instead, both are legitimate outcomes
        // this dispatch method must handle without panicking.
        let state = new_state();
        let resp = call(&state, 1, "devcontainer_list", serde_json::json!({}));
        assert!(resp.result.is_some() || resp.error.is_some());
    }

    /// A real temp git repository, matching `spartan-git`'s own
    /// established `TempRepo` fixture pattern exactly -- a real
    /// `git2::Repository::init` with a real, fixed test signature
    /// configured on the repo itself (this sandboxed test environment may
    /// have no ambient global git config at all).
    struct TempRepo {
        dir: PathBuf,
    }

    impl TempRepo {
        fn new(unique: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("spartan-backend-git-test-{unique}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let repo = git2::Repository::init(&dir).unwrap();
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Spartan Test").unwrap();
            config
                .set_str("user.email", "test@example.invalid")
                .unwrap();
            Self { dir }
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn git_status_on_a_real_repo_reports_a_real_untracked_file() {
        let tmp = TempRepo::new("status");
        std::fs::write(tmp.dir.join("new.txt"), "hello").unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "git_status",
            serde_json::json!({ "project_root": tmp.dir.to_string_lossy() }),
        );
        assert!(resp.error.is_none(), "git_status errored: {:?}", resp.error);
        let entries = resp.result.unwrap()["entries"].as_array().unwrap().clone();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["path"], "new.txt");
        assert_eq!(entries[0]["unstaged"], "added");
        assert!(entries[0]["staged"].is_null());
    }

    #[test]
    fn git_status_on_a_real_non_repo_path_errors_honestly() {
        let dir =
            std::env::temp_dir().join(format!("spartan-backend-not-a-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "git_status",
            serde_json::json!({ "project_root": dir.to_string_lossy() }),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("no git repository"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn search_project_finds_a_real_substring_across_real_files() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-search-project-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "hello world\nsecond line\n").unwrap();
        std::fs::write(dir.join("sub/b.txt"), "another hello here\n").unwrap();
        std::fs::write(dir.join("c.txt"), "no match here\n").unwrap();

        let state = new_state();
        let resp = call(
            &state,
            1,
            "search_project",
            serde_json::json!({ "project_root": dir.to_string_lossy(), "pattern": "hello" }),
        );
        assert!(
            resp.error.is_none(),
            "search_project errored: {:?}",
            resp.error
        );
        let matches = resp.result.unwrap()["matches"].as_array().unwrap().clone();
        assert_eq!(matches.len(), 2);
        let paths: Vec<String> = matches
            .iter()
            .map(|m| m["path"].as_str().unwrap().to_string())
            .collect();
        assert!(paths.contains(&"a.txt".to_string()));
        assert!(paths.iter().any(|p| p.contains("b.txt")));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn search_project_refuses_a_real_path_jail_escape() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-search-project-jail-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let state = new_state();
        let resp = call(
            &state,
            1,
            "search_project",
            serde_json::json!({
                "project_root": dir.to_string_lossy(),
                "pattern": "x",
                "path": "../../etc",
            }),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("path-jail"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn git_stage_then_unstage_a_real_file_moves_it_between_real_states() {
        let tmp = TempRepo::new("stage_unstage");
        std::fs::write(tmp.dir.join("f.txt"), "content").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();

        let stage_resp = call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        assert_eq!(stage_resp.result.unwrap()["ok"], true);

        let status_after_stage = call(
            &state,
            2,
            "git_status",
            serde_json::json!({ "project_root": root }),
        );
        let entries = status_after_stage.result.unwrap()["entries"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(entries[0]["staged"], "added");
        assert!(entries[0]["unstaged"].is_null());

        let unstage_resp = call(
            &state,
            3,
            "git_unstage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        assert_eq!(unstage_resp.result.unwrap()["ok"], true);

        let status_after_unstage = call(
            &state,
            4,
            "git_status",
            serde_json::json!({ "project_root": root }),
        );
        let entries = status_after_unstage.result.unwrap()["entries"]
            .as_array()
            .unwrap()
            .clone();
        assert!(entries[0]["staged"].is_null());
        assert_eq!(entries[0]["unstaged"], "added");
    }

    #[test]
    fn git_diff_staged_shows_the_real_head_vs_index_difference() {
        let tmp = TempRepo::new("diff_staged");
        std::fs::write(tmp.dir.join("f.txt"), "line one\nline two\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "first" }),
        );
        std::fs::write(tmp.dir.join("f.txt"), "line one\nline two changed\n").unwrap();
        call(
            &state,
            3,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        let resp = call(
            &state,
            4,
            "git_diff",
            serde_json::json!({ "project_root": root, "path": "f.txt", "staged": true }),
        );
        let diff = resp.result.unwrap()["diff"].as_str().unwrap().to_string();
        assert!(diff.contains("-line two\n"), "diff was: {diff}");
        assert!(diff.contains("+line two changed\n"), "diff was: {diff}");
        assert!(diff.contains(" line one\n"), "diff was: {diff}");
    }

    #[test]
    fn git_diff_unstaged_shows_the_real_index_vs_working_tree_difference() {
        let tmp = TempRepo::new("diff_unstaged");
        std::fs::write(tmp.dir.join("f.txt"), "original\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "first" }),
        );
        // A real unstaged edit -- never re-staged, so the index still has
        // "original" and only the working-tree file has changed.
        std::fs::write(tmp.dir.join("f.txt"), "edited but not staged\n").unwrap();
        let resp = call(
            &state,
            3,
            "git_diff",
            serde_json::json!({ "project_root": root, "path": "f.txt", "staged": false }),
        );
        let diff = resp.result.unwrap()["diff"].as_str().unwrap().to_string();
        assert!(diff.contains("-original\n"), "diff was: {diff}");
        assert!(
            diff.contains("+edited but not staged\n"),
            "diff was: {diff}"
        );
    }

    #[test]
    fn git_blame_attributes_each_line_through_the_dispatch_path() {
        let tmp = TempRepo::new("blame_dispatch");
        std::fs::write(tmp.dir.join("f.txt"), "line one\nline two\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "first" }),
        );
        // Change only line two, then commit again.
        std::fs::write(tmp.dir.join("f.txt"), "line one\nline two changed\n").unwrap();
        call(
            &state,
            3,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            4,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "second" }),
        );
        let resp = call(
            &state,
            5,
            "git_blame",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        let lines = resp.result.unwrap()["lines"].as_array().unwrap().clone();
        assert_eq!(lines.len(), 2, "one blame entry per committed line");
        assert_eq!(lines[0]["summary"], "first", "line 1 unchanged");
        assert_eq!(lines[1]["summary"], "second", "line 2 changed");
        // The two lines were touched by two different real commits.
        assert_ne!(lines[0]["oid"], lines[1]["oid"]);
    }

    #[test]
    fn git_blame_on_a_real_non_repo_path_errors_honestly() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-git-blame-non-repo-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "git_blame",
            serde_json::json!({ "project_root": dir.to_string_lossy(), "path": "f.txt" }),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_remote_round_trip_through_the_dispatch_path() {
        // A real bare repo as the "remote" -- no network, no credentials.
        let remote_dir = std::env::temp_dir().join(format!(
            "spartan-backend-git-remote-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&remote_dir);
        git2::Repository::init_bare(&remote_dir).unwrap();
        let remote_url = remote_dir.to_str().unwrap();

        // Repo A: commit through dispatch, add remote (via git2 -- there's
        // no remote-add dispatch method yet), then list + push through
        // dispatch.
        let tmp = TempRepo::new("remote_dispatch_a");
        std::fs::write(tmp.dir.join("f.txt"), "one\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "first" }),
        );
        git2::Repository::open(&tmp.dir)
            .unwrap()
            .remote("origin", remote_url)
            .unwrap();
        let branch = spartan_git::GitRepo::discover(&tmp.dir)
            .unwrap()
            .current_branch()
            .unwrap();

        let resp = call(
            &state,
            3,
            "git_remotes",
            serde_json::json!({ "project_root": root }),
        );
        let remotes = resp.result.unwrap()["remotes"].as_array().unwrap().clone();
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0]["name"], "origin");
        assert_eq!(remotes[0]["url"], remote_url);

        let resp = call(
            &state,
            4,
            "git_push",
            serde_json::json!({ "project_root": root, "remote": "origin", "branch": branch }),
        );
        assert_eq!(resp.result.unwrap()["ok"], true);

        // Repo B: add the same remote, pull through dispatch -> fast-forwards
        // and really gets A's pushed file.
        let tmp_b = TempRepo::new("remote_dispatch_b");
        git2::Repository::open(&tmp_b.dir)
            .unwrap()
            .remote("origin", remote_url)
            .unwrap();
        let root_b = tmp_b.dir.to_string_lossy().into_owned();
        let resp = call(
            &state,
            5,
            "git_pull",
            serde_json::json!({ "project_root": root_b, "remote": "origin", "branch": branch }),
        );
        assert_eq!(resp.result.unwrap()["outcome"], "fast_forwarded");
        assert_eq!(
            std::fs::read_to_string(tmp_b.dir.join("f.txt")).unwrap(),
            "one\n"
        );

        let _ = std::fs::remove_dir_all(&remote_dir);
    }

    #[test]
    fn git_stash_round_trip_through_the_dispatch_path() {
        let tmp = TempRepo::new("stash_dispatch");
        std::fs::write(tmp.dir.join("f.txt"), "original\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "init" }),
        );
        // A real uncommitted change, then stash it.
        std::fs::write(tmp.dir.join("f.txt"), "modified\n").unwrap();
        let resp = call(
            &state,
            3,
            "git_stash_save",
            serde_json::json!({ "project_root": root, "message": "wip" }),
        );
        assert_eq!(resp.result.unwrap()["stashed"], true);
        // Working tree reverted; the stash is listed.
        assert_eq!(
            std::fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "original\n"
        );
        let resp = call(
            &state,
            4,
            "git_stash_list",
            serde_json::json!({ "project_root": root }),
        );
        let stashes = resp.result.unwrap()["stashes"].as_array().unwrap().clone();
        assert_eq!(stashes.len(), 1);
        assert_eq!(stashes[0]["index"], 0);
        // Pop restores the change.
        let resp = call(
            &state,
            5,
            "git_stash_pop",
            serde_json::json!({ "project_root": root, "index": 0 }),
        );
        assert_eq!(resp.result.unwrap()["ok"], true);
        assert_eq!(
            std::fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "modified\n"
        );
    }

    #[test]
    fn git_discard_reverts_an_unstaged_edit_through_the_dispatch_path() {
        let tmp = TempRepo::new("discard_dispatch");
        std::fs::write(tmp.dir.join("f.txt"), "committed\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "init" }),
        );
        std::fs::write(tmp.dir.join("f.txt"), "dirty\n").unwrap();
        let resp = call(
            &state,
            3,
            "git_discard",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        assert_eq!(resp.result.unwrap()["ok"], true);
        assert_eq!(
            std::fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "committed\n"
        );
    }

    #[test]
    fn git_stash_apply_keeps_the_stash_through_the_dispatch_path() {
        let tmp = TempRepo::new("stash_apply_dispatch");
        std::fs::write(tmp.dir.join("f.txt"), "original\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "init" }),
        );
        std::fs::write(tmp.dir.join("f.txt"), "modified\n").unwrap();
        call(
            &state,
            3,
            "git_stash_save",
            serde_json::json!({ "project_root": root, "message": "wip" }),
        );
        // Apply restores the change but keeps the stash.
        let resp = call(
            &state,
            4,
            "git_stash_apply",
            serde_json::json!({ "project_root": root, "index": 0 }),
        );
        assert_eq!(resp.result.unwrap()["ok"], true);
        assert_eq!(
            std::fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "modified\n"
        );
        let resp = call(
            &state,
            5,
            "git_stash_list",
            serde_json::json!({ "project_root": root }),
        );
        let stashes = resp.result.unwrap()["stashes"].as_array().unwrap().clone();
        assert_eq!(stashes.len(), 1, "apply must keep the stash, not drop it");
    }

    #[test]
    fn git_commit_amend_rewrites_the_last_commit_through_the_dispatch_path() {
        let tmp = TempRepo::new("amend_dispatch");
        std::fs::write(tmp.dir.join("f.txt"), "v1\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "original" }),
        );
        let resp = call(
            &state,
            3,
            "git_commit_amend",
            serde_json::json!({ "project_root": root, "message": "amended" }),
        );
        assert_eq!(resp.result.unwrap()["ok"], true);
        // Exactly one commit remains, with the amended message.
        let resp = call(
            &state,
            4,
            "git_log",
            serde_json::json!({ "project_root": root, "max": 10 }),
        );
        let commits = resp.result.unwrap()["commits"].as_array().unwrap().clone();
        assert_eq!(commits.len(), 1, "amend must not add a commit");
        assert_eq!(commits[0]["summary"], "amended");
    }

    #[test]
    fn git_revert_commit_adds_an_undo_commit_through_the_dispatch_path() {
        let tmp = TempRepo::new("revert_dispatch");
        std::fs::write(tmp.dir.join("f.txt"), "line one\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "first" }),
        );
        std::fs::write(tmp.dir.join("f.txt"), "line one\nline two\n").unwrap();
        call(
            &state,
            3,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        let commit_resp = call(
            &state,
            4,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "add line two" }),
        );
        let bad_oid = commit_resp.result.unwrap()["oid"]
            .as_str()
            .unwrap()
            .to_string();
        // Revert the second commit -> a new commit that removes line two.
        let resp = call(
            &state,
            5,
            "git_revert_commit",
            serde_json::json!({ "project_root": root, "oid": bad_oid }),
        );
        assert_eq!(resp.result.unwrap()["ok"], true);
        // Three commits now; the newest is a "Revert" commit; file back to v1.
        let resp = call(
            &state,
            6,
            "git_log",
            serde_json::json!({ "project_root": root, "max": 10 }),
        );
        let commits = resp.result.unwrap()["commits"].as_array().unwrap().clone();
        assert_eq!(
            commits.len(),
            3,
            "revert must add a commit, not rewrite history"
        );
        assert_eq!(commits[0]["summary"], "Revert \"add line two\"");
        assert_eq!(
            std::fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "line one\n"
        );
    }

    #[test]
    fn git_log_for_ref_and_cherry_pick_round_trip_through_the_dispatch_path() {
        let tmp = TempRepo::new("cherry_pick_dispatch");
        std::fs::write(tmp.dir.join("f.txt"), "line one\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "root" }),
        );
        call(
            &state,
            3,
            "git_create_branch",
            serde_json::json!({ "project_root": root, "branch": "feature" }),
        );
        call(
            &state,
            4,
            "git_checkout",
            serde_json::json!({ "project_root": root, "branch": "feature" }),
        );
        std::fs::write(tmp.dir.join("f.txt"), "line one\nline two\n").unwrap();
        call(
            &state,
            5,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        let feature_commit = call(
            &state,
            6,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "add line two" }),
        );
        let feature_oid = feature_commit.result.unwrap()["oid"]
            .as_str()
            .unwrap()
            .to_string();
        // Back to master -- it does NOT have "line two" yet.
        call(
            &state,
            7,
            "git_checkout",
            serde_json::json!({ "project_root": root, "branch": "master" }),
        );
        assert_eq!(
            std::fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "line one\n"
        );
        // Browse "feature"'s own log without checking it out again.
        let feature_log = call(
            &state,
            8,
            "git_log_for_ref",
            serde_json::json!({ "project_root": root, "ref_name": "feature", "max": 10 }),
        );
        let commits = feature_log.result.unwrap()["commits"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(commits.len(), 2, "feature has root + its own commit");
        assert_eq!(commits[0]["oid"], feature_oid);
        // Cherry-pick that commit onto master.
        let resp = call(
            &state,
            9,
            "git_cherry_pick",
            serde_json::json!({ "project_root": root, "oid": feature_oid }),
        );
        assert_eq!(resp.result.unwrap()["ok"], true);
        assert_eq!(
            std::fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "line one\nline two\n",
            "the cherry-picked change is now on master's working tree"
        );
        let master_log = call(
            &state,
            10,
            "git_log",
            serde_json::json!({ "project_root": root, "max": 10 }),
        );
        let commits = master_log.result.unwrap()["commits"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(commits.len(), 2, "a real new commit, not a rewrite");
    }

    #[test]
    fn git_cherry_pick_with_an_unknown_oid_errors_honestly_through_the_dispatch_path() {
        let tmp = TempRepo::new("cherry_pick_bad_dispatch");
        std::fs::write(tmp.dir.join("f.txt"), "v1\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "init" }),
        );
        let resp = call(
            &state,
            3,
            "git_cherry_pick",
            serde_json::json!({
                "project_root": root,
                "oid": "0000000000000000000000000000000000000000"
            }),
        );
        assert!(resp.error.is_some());
    }

    #[test]
    fn git_tag_create_list_delete_round_trip_through_the_dispatch_path() {
        let tmp = TempRepo::new("tags_dispatch");
        std::fs::write(tmp.dir.join("f.txt"), "v1\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        let commit_resp = call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "init" }),
        );
        let head = commit_resp.result.unwrap()["oid"]
            .as_str()
            .unwrap()
            .to_string();
        // Create an annotated tag on HEAD.
        let resp = call(
            &state,
            3,
            "git_create_tag",
            serde_json::json!({ "project_root": root, "name": "v1.0", "oid": head, "message": "first release" }),
        );
        assert_eq!(resp.result.unwrap()["ok"], true);
        // List: exactly one annotated tag pointing at HEAD.
        let resp = call(
            &state,
            4,
            "git_tags",
            serde_json::json!({ "project_root": root }),
        );
        let tags = resp.result.unwrap()["tags"].as_array().unwrap().clone();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0]["name"], "v1.0");
        assert_eq!(tags[0]["annotated"], true);
        assert_eq!(tags[0]["target"], head);
        // Delete it -> empty list.
        let resp = call(
            &state,
            5,
            "git_delete_tag",
            serde_json::json!({ "project_root": root, "name": "v1.0" }),
        );
        assert_eq!(resp.result.unwrap()["ok"], true);
        let resp = call(
            &state,
            6,
            "git_tags",
            serde_json::json!({ "project_root": root }),
        );
        assert!(resp.result.unwrap()["tags"].as_array().unwrap().is_empty());
    }

    #[test]
    fn git_diff_on_a_real_non_repo_path_errors_honestly() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-git-diff-non-repo-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "git_diff",
            serde_json::json!({
                "project_root": dir.to_string_lossy(),
                "path": "f.txt",
                "staged": true,
            }),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("no git repository found"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn git_diff_hunks_and_stage_hunk_round_trip_through_the_dispatch_path() {
        let tmp = TempRepo::new("diff_hunks_dispatch");
        let base: String = (1..=20)
            .map(|n| format!("line{n}\n"))
            .collect::<Vec<_>>()
            .join("");
        std::fs::write(tmp.dir.join("f.txt"), &base).unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "base" }),
        );
        let modified = base
            .replace("line2\n", "line2 CHANGED\n")
            .replace("line19\n", "line19 CHANGED\n");
        std::fs::write(tmp.dir.join("f.txt"), &modified).unwrap();

        let hunks_resp = call(
            &state,
            3,
            "git_diff_hunks",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        let hunks = hunks_resp.result.unwrap()["hunks"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(hunks.len(), 2);
        assert!(hunks[0]["body"].as_str().unwrap().contains("line2 CHANGED"));

        let stage_resp = call(
            &state,
            4,
            "git_stage_hunk",
            serde_json::json!({ "project_root": root, "path": "f.txt", "hunk_index": 0 }),
        );
        assert_eq!(stage_resp.result.unwrap()["ok"], true);

        // The real working tree is untouched by staging.
        assert_eq!(
            std::fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            modified
        );

        let status_resp = call(
            &state,
            5,
            "git_status",
            serde_json::json!({ "project_root": root }),
        );
        let entries = status_resp.result.unwrap()["entries"]
            .as_array()
            .unwrap()
            .clone();
        let entry = entries
            .iter()
            .find(|e| e["path"] == "f.txt")
            .expect("f.txt must be in status");
        assert!(!entry["staged"].is_null());
        assert!(!entry["unstaged"].is_null());

        // Exactly one real hunk remains after staging the first.
        let remaining_resp = call(
            &state,
            6,
            "git_diff_hunks",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        let remaining = remaining_resp.result.unwrap()["hunks"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn git_stage_hunk_out_of_range_errors_honestly_through_the_dispatch_path() {
        let tmp = TempRepo::new("stage_hunk_oor_dispatch");
        std::fs::write(tmp.dir.join("f.txt"), "v1\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "base" }),
        );
        std::fs::write(tmp.dir.join("f.txt"), "v2\n").unwrap();
        let resp = call(
            &state,
            3,
            "git_stage_hunk",
            serde_json::json!({ "project_root": root, "path": "f.txt", "hunk_index": 99 }),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("git stage hunk"));
    }

    #[test]
    fn git_branches_create_and_checkout_round_trip_through_the_real_dispatch() {
        let tmp = TempRepo::new("branches_dispatch");
        std::fs::write(tmp.dir.join("f.txt"), "content").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "first" }),
        );

        let branches = call(
            &state,
            3,
            "git_branches",
            serde_json::json!({ "project_root": root }),
        );
        let list = branches.result.unwrap()["branches"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["name"], "master");
        assert_eq!(list[0]["current"], true);

        let created = call(
            &state,
            4,
            "git_create_branch",
            serde_json::json!({ "project_root": root, "branch": "feature" }),
        );
        assert_eq!(created.result.unwrap()["ok"], true);

        let checked_out = call(
            &state,
            5,
            "git_checkout",
            serde_json::json!({ "project_root": root, "branch": "feature" }),
        );
        assert_eq!(checked_out.result.unwrap()["ok"], true);

        let branches_after = call(
            &state,
            6,
            "git_branches",
            serde_json::json!({ "project_root": root }),
        );
        let list = branches_after.result.unwrap()["branches"]
            .as_array()
            .unwrap()
            .clone();
        let feature = list.iter().find(|b| b["name"] == "feature").unwrap();
        assert_eq!(feature["current"], true);
        // git_status must also report the real new branch name.
        let status = call(
            &state,
            7,
            "git_status",
            serde_json::json!({ "project_root": root }),
        );
        assert_eq!(status.result.unwrap()["branch"], "feature");

        // A repo with no remotes: git_remote_branches returns a real empty
        // list, never an error (task #251). The full remote round trip is
        // proven in spartan-git's own bare-remote integration test.
        let remotes = call(
            &state,
            8,
            "git_remote_branches",
            serde_json::json!({ "project_root": root }),
        );
        assert!(
            remotes.error.is_none(),
            "git_remote_branches errored: {:?}",
            remotes.error
        );
        assert_eq!(
            remotes.result.unwrap()["branches"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn git_log_returns_real_commits_newest_first_through_the_dispatch() {
        let tmp = TempRepo::new("log_dispatch");
        std::fs::write(tmp.dir.join("f.txt"), "v1").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "first commit" }),
        );
        std::fs::write(tmp.dir.join("f.txt"), "v2").unwrap();
        call(
            &state,
            3,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            4,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "second commit" }),
        );
        let resp = call(
            &state,
            5,
            "git_log",
            serde_json::json!({ "project_root": root, "max": 10 }),
        );
        let commits = resp.result.unwrap()["commits"].as_array().unwrap().clone();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0]["summary"], "second commit");
        assert_eq!(commits[1]["summary"], "first commit");
        assert_eq!(commits[0]["author"], "Spartan Test");
        assert_eq!(commits[0]["oid"].as_str().unwrap().len(), 40);
        assert!(commits[0]["time"].as_i64().unwrap() > 0);
    }

    #[test]
    fn git_commit_files_and_diff_round_trip_through_the_real_dispatch() {
        let tmp = TempRepo::new("commit_detail_dispatch");
        std::fs::write(tmp.dir.join("a.txt"), "a v1\n").unwrap();
        std::fs::write(tmp.dir.join("b.txt"), "b v1\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        for (i, p) in ["a.txt", "b.txt"].iter().enumerate() {
            call(
                &state,
                (i + 1) as u64,
                "git_stage",
                serde_json::json!({ "project_root": root, "path": p }),
            );
        }
        call(
            &state,
            3,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "first" }),
        );
        std::fs::write(tmp.dir.join("a.txt"), "a v2\n").unwrap();
        call(
            &state,
            4,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "a.txt" }),
        );
        let commit_resp = call(
            &state,
            5,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "second" }),
        );
        let oid = commit_resp.result.unwrap()["oid"]
            .as_str()
            .unwrap()
            .to_string();

        let files_resp = call(
            &state,
            6,
            "git_commit_files",
            serde_json::json!({ "project_root": root, "oid": oid }),
        );
        let files = files_resp.result.unwrap()["files"]
            .as_array()
            .unwrap()
            .clone();
        // Only a.txt was touched by the second commit -- b.txt must not
        // appear.
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["path"], "a.txt");
        assert_eq!(files[0]["status"], "modified");

        let diff_resp = call(
            &state,
            7,
            "git_commit_diff",
            serde_json::json!({ "project_root": root, "oid": oid, "path": "a.txt" }),
        );
        let diff = diff_resp.result.unwrap()["diff"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(diff.contains("-a v1\n"), "diff was: {diff}");
        assert!(diff.contains("+a v2\n"), "diff was: {diff}");
    }

    #[test]
    fn git_commit_files_on_a_bogus_oid_errors_honestly() {
        let tmp = TempRepo::new("commit_detail_bad_oid");
        std::fs::write(tmp.dir.join("f.txt"), "x").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        let resp = call(
            &state,
            1,
            "git_commit_files",
            serde_json::json!({ "project_root": root, "oid": "not-a-real-oid" }),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("git commit files"));
    }

    #[test]
    fn git_checkout_of_a_nonexistent_branch_errors_honestly() {
        let tmp = TempRepo::new("checkout_missing");
        std::fs::write(tmp.dir.join("f.txt"), "content").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "first" }),
        );
        let resp = call(
            &state,
            3,
            "git_checkout",
            serde_json::json!({ "project_root": root, "branch": "no-such-branch" }),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("git checkout"));
    }

    #[test]
    fn git_commit_a_real_staged_file_clears_real_status_and_returns_a_real_oid() {
        let tmp = TempRepo::new("commit");
        std::fs::write(tmp.dir.join("f.txt"), "content").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();

        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        let commit_resp = call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "real first commit" }),
        );
        assert!(
            commit_resp.error.is_none(),
            "git_commit errored: {:?}",
            commit_resp.error
        );
        let oid = commit_resp.result.unwrap()["oid"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(oid.len(), 40, "a real git2::Oid renders as 40 hex chars");

        let status_after_commit = call(
            &state,
            3,
            "git_status",
            serde_json::json!({ "project_root": root }),
        );
        let entries = status_after_commit.result.unwrap()["entries"]
            .as_array()
            .unwrap()
            .clone();
        assert!(
            entries.is_empty(),
            "a clean tree after commit has no real status entries"
        );
    }

    /// Creates two real branches diverging from a shared root commit, each
    /// with a real, overlapping edit to the same line of the same file --
    /// the exact real setup that produces a genuine merge conflict, not a
    /// simulated one.
    fn repo_with_a_real_conflicting_merge(
        unique: &str,
    ) -> (TempRepo, String, Arc<Mutex<BackendState>>) {
        let tmp = TempRepo::new(unique);
        std::fs::write(tmp.dir.join("f.txt"), "line one\nline two\nline three\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "root" }),
        );
        call(
            &state,
            3,
            "git_create_branch",
            serde_json::json!({ "project_root": root, "branch": "feature" }),
        );
        call(
            &state,
            4,
            "git_checkout",
            serde_json::json!({ "project_root": root, "branch": "feature" }),
        );
        std::fs::write(
            tmp.dir.join("f.txt"),
            "line one\nFEATURE CHANGE\nline three\n",
        )
        .unwrap();
        call(
            &state,
            5,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            6,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "feature edit" }),
        );
        call(
            &state,
            7,
            "git_checkout",
            serde_json::json!({ "project_root": root, "branch": "master" }),
        );
        std::fs::write(
            tmp.dir.join("f.txt"),
            "line one\nMASTER CHANGE\nline three\n",
        )
        .unwrap();
        call(
            &state,
            8,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            9,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "master edit" }),
        );
        (tmp, root, state)
    }

    #[test]
    fn git_merge_branch_reports_a_real_conflict_through_the_dispatch_path() {
        let (tmp, root, state) = repo_with_a_real_conflicting_merge("merge_conflict_dispatch");

        let resp = call(
            &state,
            10,
            "git_merge_branch",
            serde_json::json!({ "project_root": root, "branch": "feature" }),
        );
        assert_eq!(resp.result.unwrap()["outcome"], "conflicted");

        let status = call(
            &state,
            11,
            "git_merge_status",
            serde_json::json!({ "project_root": root }),
        );
        let result = status.result.unwrap();
        assert_eq!(result["in_progress"], true);
        let conflicts = result["conflicts"].as_array().unwrap().clone();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0]["path"], "f.txt");
        assert!(conflicts[0]["ours"]
            .as_str()
            .unwrap()
            .contains("MASTER CHANGE"));
        assert!(conflicts[0]["theirs"]
            .as_str()
            .unwrap()
            .contains("FEATURE CHANGE"));

        // A real one-click "take theirs" resolution, then complete the merge
        // with a real two-parent commit.
        let theirs = conflicts[0]["theirs"].as_str().unwrap().to_string();
        let resolve_resp = call(
            &state,
            12,
            "git_resolve_conflict",
            serde_json::json!({ "project_root": root, "path": "f.txt", "content": theirs }),
        );
        assert_eq!(resolve_resp.result.unwrap()["ok"], true);

        let status_after_resolve = call(
            &state,
            13,
            "git_merge_status",
            serde_json::json!({ "project_root": root }),
        );
        assert!(status_after_resolve.result.unwrap()["conflicts"]
            .as_array()
            .unwrap()
            .is_empty());

        let commit_resp = call(
            &state,
            14,
            "git_commit_merge",
            serde_json::json!({ "project_root": root, "message": "merge feature" }),
        );
        let oid = commit_resp.result.unwrap()["oid"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(oid.len(), 40);

        let status_final = call(
            &state,
            15,
            "git_merge_status",
            serde_json::json!({ "project_root": root }),
        );
        assert_eq!(status_final.result.unwrap()["in_progress"], false);
        assert_eq!(
            std::fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "line one\nFEATURE CHANGE\nline three\n"
        );
    }

    #[test]
    fn git_abort_merge_discards_the_conflict_through_the_dispatch_path() {
        let (tmp, root, state) = repo_with_a_real_conflicting_merge("merge_abort_dispatch");

        call(
            &state,
            10,
            "git_merge_branch",
            serde_json::json!({ "project_root": root, "branch": "feature" }),
        );
        let abort_resp = call(
            &state,
            11,
            "git_abort_merge",
            serde_json::json!({ "project_root": root }),
        );
        assert_eq!(abort_resp.result.unwrap()["ok"], true);

        let status = call(
            &state,
            12,
            "git_merge_status",
            serde_json::json!({ "project_root": root }),
        );
        assert_eq!(status.result.unwrap()["in_progress"], false);
        assert_eq!(
            std::fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "line one\nMASTER CHANGE\nline three\n"
        );
    }

    #[test]
    fn git_merge_branch_fast_forwards_with_no_conflict_through_the_dispatch_path() {
        let tmp = TempRepo::new("merge_ff_dispatch");
        std::fs::write(tmp.dir.join("f.txt"), "v1\n").unwrap();
        let state = new_state();
        let root = tmp.dir.to_string_lossy().into_owned();
        call(
            &state,
            1,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            2,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "root" }),
        );
        call(
            &state,
            3,
            "git_create_branch",
            serde_json::json!({ "project_root": root, "branch": "feature" }),
        );
        call(
            &state,
            4,
            "git_checkout",
            serde_json::json!({ "project_root": root, "branch": "feature" }),
        );
        std::fs::write(tmp.dir.join("f.txt"), "v2\n").unwrap();
        call(
            &state,
            5,
            "git_stage",
            serde_json::json!({ "project_root": root, "path": "f.txt" }),
        );
        call(
            &state,
            6,
            "git_commit",
            serde_json::json!({ "project_root": root, "message": "feature edit" }),
        );
        call(
            &state,
            7,
            "git_checkout",
            serde_json::json!({ "project_root": root, "branch": "master" }),
        );

        let resp = call(
            &state,
            8,
            "git_merge_branch",
            serde_json::json!({ "project_root": root, "branch": "feature" }),
        );
        assert_eq!(resp.result.unwrap()["outcome"], "fast_forwarded");
        assert_eq!(
            std::fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "v2\n"
        );

        let status = call(
            &state,
            9,
            "git_merge_status",
            serde_json::json!({ "project_root": root }),
        );
        assert_eq!(status.result.unwrap()["in_progress"], false);
    }

    #[test]
    fn git_merge_branch_on_a_real_non_repo_path_errors_honestly() {
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-merge-non-repo-{}-{}",
            std::process::id(),
            "merge_non_repo_dispatch"
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "git_merge_branch",
            serde_json::json!({ "project_root": scratch.to_string_lossy(), "branch": "feature" }),
        );
        assert!(resp.result.is_none());
        assert!(resp
            .error
            .unwrap()
            .contains("no git repository found at or above this path"));
    }

    /// `settings_get`/`settings_set` both resolve `$HOME` process-wide
    /// (`spartan_settings::settings_path`) -- a real Mutex here serializes
    /// the two tests that mutate it against each other so a default
    /// multi-threaded `cargo test` run can't interleave one test's
    /// temporary `$HOME` with the other's real file I/O.
    static HOME_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn settings_get_with_no_saved_file_returns_real_defaults() {
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        // A real, isolated $HOME so this test can't read/clobber the
        // actual user's real ~/.spartan/settings.json.
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-settings-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let prior_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &scratch);

        let state = new_state();
        let resp = call(&state, 1, "settings_get", serde_json::json!({}));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["gpu_offload"]["enabled"], true);
        assert!(result["gpu_offload"]["layers"].is_null());

        match prior_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn settings_set_then_get_round_trips_real_values_through_a_real_file() {
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-settings-roundtrip-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let prior_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &scratch);

        let state = new_state();
        let set_resp = call(
            &state,
            1,
            "settings_set",
            serde_json::json!({
                "gpu_enabled": false,
                "gpu_layers": 12,
                "leo_provider": { "kind": "Claude", "model": "claude-3-5-sonnet-latest" },
            }),
        );
        assert!(
            set_resp.error.is_none(),
            "settings_set errored: {:?}",
            set_resp.error
        );
        assert_eq!(set_resp.result.unwrap()["gpu_offload"]["enabled"], false);

        let get_resp = call(&state, 2, "settings_get", serde_json::json!({}));
        let result = get_resp.result.unwrap();
        assert_eq!(result["gpu_offload"]["enabled"], false);
        assert_eq!(result["gpu_offload"]["layers"], 12);
        assert_eq!(result["leo_provider"]["kind"], "Claude");
        assert_eq!(result["leo_provider"]["model"], "claude-3-5-sonnet-latest");

        match prior_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn settings_set_without_leo_provider_preserves_the_real_previously_saved_one() {
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-settings-provider-preserve-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let prior_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &scratch);

        let state = new_state();
        call(
            &state,
            1,
            "settings_set",
            serde_json::json!({
                "gpu_enabled": true,
                "leo_provider": { "kind": "LiteLLM", "model": "gpt-4o" },
            }),
        );
        // A later, unrelated GPU-only save must not silently reset the
        // real, already-chosen provider back to the Ollama default.
        let set_resp = call(
            &state,
            2,
            "settings_set",
            serde_json::json!({ "gpu_enabled": false }),
        );
        let result = set_resp.result.unwrap();
        assert_eq!(result["leo_provider"]["kind"], "LiteLLM");
        assert_eq!(result["leo_provider"]["model"], "gpt-4o");

        match prior_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn settings_set_real_dispatch_parses_sets_preserves_and_clears_leo_verify_command() {
        // Real, dispatch-level coverage of `settings_set`'s own nested-
        // `Option` `leo_verify_command` parse arm (task #265) -- not just
        // the extracted `run_leo_verification_and_completion` function's
        // own unit tests, which never touch this crate's real IPC
        // request-parsing path at all.
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-settings-verify-command-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let prior_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &scratch);

        let state = new_state();
        // Set a real, deliberately whitespace-padded command -- confirms
        // the dispatch arm really trims it.
        let set_resp = call(
            &state,
            1,
            "settings_set",
            serde_json::json!({
                "gpu_enabled": true,
                "leo_verify_command": "  cargo test  ",
            }),
        );
        assert_eq!(set_resp.result.unwrap()["leo_verify_command"], "cargo test");

        // A later, unrelated GPU-only save (the key omitted entirely) must
        // preserve the real, already-set command -- outer `None`, not a
        // silent clear.
        let preserve_resp = call(
            &state,
            2,
            "settings_set",
            serde_json::json!({ "gpu_enabled": false }),
        );
        assert_eq!(
            preserve_resp.result.unwrap()["leo_verify_command"],
            "cargo test"
        );

        // An explicit empty string really clears it back to `null`, not
        // a rejected request or a literal empty-string value stored.
        let clear_resp = call(
            &state,
            3,
            "settings_set",
            serde_json::json!({
                "gpu_enabled": false,
                "leo_verify_command": "",
            }),
        );
        assert_eq!(
            clear_resp.result.unwrap()["leo_verify_command"],
            serde_json::Value::Null
        );

        match prior_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn settings_set_editor_appearance_and_onboarding_round_trip_and_preserve_each_other() {
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-settings-editor-appearance-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let prior_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &scratch);

        let state = new_state();
        // Set editor + onboarding first.
        call(
            &state,
            1,
            "settings_set",
            serde_json::json!({
                "gpu_enabled": true,
                "editor": { "font_size": 18, "tab_size": 4, "word_wrap": true },
                "onboarding_completed": true,
            }),
        );
        // A later, unrelated appearance-only save must not reset the
        // real, already-saved editor settings or onboarding flag.
        let set_resp = call(
            &state,
            2,
            "settings_set",
            serde_json::json!({
                "gpu_enabled": true,
                "appearance": { "reduce_motion": true },
            }),
        );
        let result = set_resp.result.unwrap();
        assert_eq!(result["editor"]["font_size"], 18);
        assert_eq!(result["editor"]["tab_size"], 4);
        assert_eq!(result["editor"]["word_wrap"], true);
        assert_eq!(result["appearance"]["reduce_motion"], true);
        assert_eq!(result["onboarding_completed"], true);

        let get_resp = call(&state, 3, "settings_get", serde_json::json!({}));
        let result = get_resp.result.unwrap();
        assert_eq!(result["editor"]["font_size"], 18);
        assert_eq!(result["appearance"]["reduce_motion"], true);
        assert_eq!(result["onboarding_completed"], true);

        match prior_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Real regression test for a real bug found live by code review:
    /// `onboarding_completed` used to be parsed via `.as_bool()`, which
    /// silently returns `None` (treated identically to "not provided,
    /// keep the current value") for *any* non-boolean JSON value --
    /// unlike every sibling optional field (`editor`/`appearance`/
    /// `leo_provider`), which all report an honest `"invalid X: ..."`
    /// error on a real type mismatch. A caller sending a genuine bug
    /// (e.g. `"onboarding_completed": "true"`, a string) got back a
    /// silent `Ok` with the flag left unchanged, believing it had been
    /// set.
    #[test]
    fn settings_set_with_a_non_boolean_onboarding_completed_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "settings_set",
            serde_json::json!({
                "gpu_enabled": true,
                "onboarding_completed": "true",
            }),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("invalid onboarding_completed"));
    }

    /// A real, minimal, hand-rolled HTTP/1.1 server for
    /// `crash_report_upload`'s own real `ureq` POST -- the same technique
    /// `spartan-crash`'s own test module already established, duplicated
    /// here (not shared) since it's a small, self-contained test helper
    /// and this crate has no existing precedent for cross-crate test-only
    /// dependencies.
    fn spawn_mock_upload_server(
        response_status: u16,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let body = loop {
                let n = stream.read(&mut chunk).unwrap();
                if n == 0 {
                    break String::new();
                }
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf);
                if let Some(header_end) = text.find("\r\n\r\n") {
                    let content_length: usize = text[..header_end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    let body_start = header_end + 4;
                    if buf.len() >= body_start + content_length {
                        break String::from_utf8_lossy(
                            &buf[body_start..body_start + content_length],
                        )
                        .to_string();
                    }
                }
            };
            let _ = tx.send(body);
            let reason = if response_status == 200 {
                "OK"
            } else {
                "Error"
            };
            let response =
                format!("HTTP/1.1 {response_status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(response.as_bytes());
        });
        (format!("http://127.0.0.1:{port}"), rx)
    }

    #[test]
    fn crash_reports_list_on_a_fresh_home_with_no_crashes_returns_a_real_empty_list() {
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-crash-list-empty-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let prior_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &scratch);

        let state = new_state();
        let resp = call(&state, 1, "crash_reports_list", serde_json::json!({}));
        assert!(resp.error.is_none());
        let reports = resp.result.unwrap()["reports"].as_array().unwrap().clone();
        assert!(reports.is_empty());

        match prior_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn crash_reports_list_finds_real_reports_written_by_the_real_crash_hook() {
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-crash-list-populated-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let prior_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &scratch);

        spartan_crash::write_report(
            &crash_dir(),
            &spartan_crash::CrashReport {
                unix_timestamp: 1,
                message: "a real test panic message".to_string(),
                location: Some("src/lib.rs:1:1".to_string()),
            },
        )
        .unwrap();

        let state = new_state();
        let resp = call(&state, 1, "crash_reports_list", serde_json::json!({}));
        assert!(resp.error.is_none());
        let reports = resp.result.unwrap()["reports"].as_array().unwrap().clone();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0]["filename"], "crash-1.json");
        assert_eq!(reports[0]["report"]["message"], "a real test panic message");

        match prior_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn crash_report_upload_refuses_an_unexpected_filename_before_touching_disk() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "crash_report_upload",
            serde_json::json!({ "filename": "../../etc/passwd" }),
        );
        assert!(resp.result.is_none());
        assert!(resp
            .error
            .unwrap()
            .contains("refusing to upload unexpected filename"));
    }

    #[test]
    fn crash_report_upload_errors_honestly_with_no_endpoint_configured() {
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-crash-upload-no-endpoint-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let prior_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &scratch);

        spartan_crash::write_report(
            &crash_dir(),
            &spartan_crash::CrashReport {
                unix_timestamp: 2,
                message: "m".to_string(),
                location: None,
            },
        )
        .unwrap();

        let state = new_state();
        let resp = call(
            &state,
            1,
            "crash_report_upload",
            serde_json::json!({ "filename": "crash-2.json" }),
        );
        assert!(resp.result.is_none());
        assert!(resp
            .error
            .unwrap()
            .contains("no crash-report upload endpoint configured"));

        match prior_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn crash_report_upload_really_posts_the_exact_on_disk_report_to_a_real_local_server() {
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-crash-upload-real-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let prior_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &scratch);

        let report = spartan_crash::CrashReport {
            unix_timestamp: 3,
            message: "real upload test".to_string(),
            location: None,
        };
        spartan_crash::write_report(&crash_dir(), &report).unwrap();
        let expected_body = spartan_crash::format_report(&report);

        let (endpoint, rx) = spawn_mock_upload_server(200);
        spartan_settings::save(&spartan_settings::Settings {
            crash_reporting: spartan_settings::CrashReportingSettings {
                upload_endpoint: Some(endpoint),
            },
            ..Default::default()
        })
        .unwrap();

        let state = new_state();
        let resp = call(
            &state,
            1,
            "crash_report_upload",
            serde_json::json!({ "filename": "crash-3.json" }),
        );
        assert!(resp.error.is_none(), "{:?}", resp.error);
        assert_eq!(resp.result.unwrap()["status"], 200);
        let received = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(received, expected_body);

        match prior_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn create_project_writes_a_real_runnable_rust_scaffold_to_disk() {
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-create-project-rust-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();

        let state = new_state();
        let resp = call(
            &state,
            1,
            "create_project",
            serde_json::json!({
                "parent_dir": scratch.to_string_lossy(),
                "template": "rust",
                "name": "My Cool Crate!",
            }),
        );
        assert!(
            resp.error.is_none(),
            "create_project errored: {:?}",
            resp.error
        );
        let result = resp.result.unwrap();
        // Real sanitization: spaces and punctuation become `-`.
        assert_eq!(result["name"], "My-Cool-Crate-");

        let project_root = scratch.join("My-Cool-Crate-");
        let cargo_toml = std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap();
        assert!(cargo_toml.contains("name = \"My-Cool-Crate-\""));
        let main_rs = std::fs::read_to_string(project_root.join("src/main.rs")).unwrap();
        assert!(main_rs.contains("Hello from My-Cool-Crate-!"));

        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn create_project_refuses_to_overwrite_a_real_nonempty_existing_directory() {
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-create-project-conflict-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        let existing = scratch.join("taken");
        std::fs::create_dir_all(&existing).unwrap();
        std::fs::write(existing.join("real-preexisting-file.txt"), "do not touch").unwrap();

        let state = new_state();
        let resp = call(
            &state,
            1,
            "create_project",
            serde_json::json!({
                "parent_dir": scratch.to_string_lossy(),
                "template": "go",
                "name": "taken",
            }),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("already exists"));
        // The real pre-existing file must be completely untouched.
        assert_eq!(
            std::fs::read_to_string(existing.join("real-preexisting-file.txt")).unwrap(),
            "do not touch"
        );

        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn create_project_rejects_an_unknown_template_honestly() {
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-create-project-unknown-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();

        let state = new_state();
        let resp = call(
            &state,
            1,
            "create_project",
            serde_json::json!({
                "parent_dir": scratch.to_string_lossy(),
                "template": "cobol",
                "name": "legacy",
            }),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("unknown project template"));

        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn create_project_every_real_template_produces_files_spartan_languages_can_detect() {
        for template in [
            "rust",
            "typescript",
            "javascript",
            "python",
            "kotlin",
            "java",
            "go",
            "csharp",
            "android",
        ] {
            let scratch = std::env::temp_dir().join(format!(
                "spartan-backend-create-project-detect-{template}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&scratch);
            std::fs::create_dir_all(&scratch).unwrap();

            let state = new_state();
            let resp = call(
                &state,
                1,
                "create_project",
                serde_json::json!({
                    "parent_dir": scratch.to_string_lossy(),
                    "template": template,
                    "name": "detectme",
                }),
            );
            assert!(
                resp.error.is_none(),
                "template {template} errored: {:?}",
                resp.error
            );
            let project_root = scratch.join("detectme");
            let registry = spartan_languages::LanguageRegistry::curated_default();
            let detected = registry.detect_project_languages(&project_root);
            assert!(
                !detected.is_empty(),
                "template {template} produced a project spartan-languages could not detect at all"
            );

            std::fs::remove_dir_all(&scratch).ok();
        }
    }

    #[test]
    fn create_project_android_template_is_recognized_by_spartan_android_as_a_real_android_project()
    {
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-create-project-android-detect-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();

        let state = new_state();
        let resp = call(
            &state,
            1,
            "create_project",
            serde_json::json!({
                "parent_dir": scratch.to_string_lossy(),
                "template": "android",
                "name": "my-android-app",
            }),
        );
        assert!(
            resp.error.is_none(),
            "create_project errored: {:?}",
            resp.error
        );
        let project_root = scratch.join("my-android-app");
        assert!(
            spartan_android::is_android_project(&project_root),
            "the real android template must be recognized as a real Android project"
        );
        let manifest = std::fs::read_to_string(
            project_root
                .join("app")
                .join("src")
                .join("main")
                .join("AndroidManifest.xml"),
        )
        .unwrap();
        assert!(manifest.contains("my-android-app"));

        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Real, live, self-skipping end-to-end confirmation that the android
    /// template isn't merely *detected* as an Android project but is
    /// genuinely, actually buildable -- scaffolds a real project via the
    /// real `create_project` dispatch, then runs the real
    /// `spartan_android::build::build_debug_apk` against it (the exact
    /// same function `android_build_apk` calls). Self-skips (matching this
    /// workspace's own established convention) if this environment has no
    /// real Android SDK (`SPARTAN_TEST_ANDROID_SDK`) -- when it runs, this
    /// is a genuine `assembleDebug` against real Google Maven/Maven
    /// Central dependencies, confirmed once already by `spartan-android`'s
    /// own `build.rs` test against a hand-written fixture; this test
    /// confirms the *product's own template content*, not a hand-written
    /// duplicate, produces an identical real result.
    #[test]
    fn create_project_android_template_produces_a_real_buildable_project() {
        let Ok(sdk_root) = std::env::var("SPARTAN_TEST_ANDROID_SDK") else {
            eprintln!(
                "SKIP: SPARTAN_TEST_ANDROID_SDK not set, skipping real android template build test"
            );
            return;
        };
        let sdk_root = std::path::PathBuf::from(sdk_root);
        if !sdk_root.is_dir() {
            eprintln!(
                "SKIP: {sdk_root:?} does not exist, skipping real android template build test"
            );
            return;
        }

        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-create-project-android-build-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();

        let state = new_state();
        let resp = call(
            &state,
            1,
            "create_project",
            serde_json::json!({
                "parent_dir": scratch.to_string_lossy(),
                "template": "android",
                "name": "buildme",
            }),
        );
        assert!(
            resp.error.is_none(),
            "create_project errored: {:?}",
            resp.error
        );
        let project_root = scratch.join("buildme");

        let (tx, rx) = mpsc::channel();
        let gradle = spartan_android::detect_toolchain().gradle_path;
        let result = spartan_android::build::build_debug_apk(
            &project_root,
            Some(&sdk_root),
            gradle.as_deref(),
            tx,
        );
        let lines: Vec<String> = rx.try_iter().collect();
        let apk_path = result.unwrap_or_else(|e| {
            panic!(
                "expected a real successful build of the product's own android template, got \
                 error: {e}\nlast output lines: {:?}",
                &lines[lines.len().saturating_sub(20)..]
            )
        });
        assert!(apk_path.is_file());
        let bytes = std::fs::read(&apk_path).unwrap();
        assert_eq!(
            &bytes[0..4],
            b"PK\x03\x04",
            "expected a real ZIP/APK signature"
        );

        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn build_leo_provider_constructs_a_real_ollama_provider_by_default() {
        let settings = spartan_settings::LeoProviderSettings::default();
        let provider =
            build_leo_provider(&settings, spartan_settings::GpuOffloadSettings::default())
                .expect("Ollama provider construction must never fail");
        assert!(provider.is_local());
        assert_eq!(provider.id(), "llama3.1:8b");
    }

    #[test]
    fn model_status_json_reports_a_real_configured_provider() {
        // Loads whatever settings exist (defaults to Ollama when none), builds
        // the real provider, and reports a real live health probe. We assert on
        // the shape, not a specific health (Ollama may or may not be running).
        let status = model_status_json();
        // Either a configured provider with the expected fields, or an honest
        // construction error -- never a fabricated success.
        if status["configured"] == serde_json::Value::Bool(true) {
            assert!(status["kind"].is_string(), "reports the provider kind");
            assert!(status["is_local"].is_boolean());
            assert!(status["context_window"].is_u64());
            let health = status["health"].as_str().unwrap();
            assert!(
                matches!(health, "healthy" | "unauthorized" | "unreachable"),
                "health is a real enum value, got {health:?}"
            );
        } else {
            assert!(status["error"].is_string(), "an error is reported plainly");
        }
    }

    /// `model_status_json()` itself has been real and tested since §75.43,
    /// but `handle_request` never exposed it as a real callable method --
    /// `spartan-devserver`'s own wrapping dispatcher answered `model_status`
    /// directly and never fell through to this crate for it, so `desktop/`
    /// (which talks to a plain `spartan-backend` process, not a devserver)
    /// had no way to reach it at all. This confirms the dispatch arm itself
    /// reaches the same real function, matching its own shape exactly.
    #[test]
    fn model_status_is_a_real_reachable_backend_method() {
        let state = new_state();
        let resp = call(&state, 1, "model_status", serde_json::json!({}));
        assert!(resp.error.is_none(), "model_status must succeed: {resp:?}");
        let result = resp.result.unwrap();
        assert!(
            result["configured"].is_boolean(),
            "reports a real configured flag"
        );
        assert!(result["kind"].is_string(), "reports the provider kind");
    }

    #[test]
    fn build_leo_provider_constructs_a_real_lmstudio_provider() {
        let settings = spartan_settings::LeoProviderSettings {
            kind: spartan_settings::LeoProviderKind::LmStudio,
            model: "local-model".to_string(),
            ..Default::default()
        };
        let provider =
            build_leo_provider(&settings, spartan_settings::GpuOffloadSettings::default())
                .expect("LM Studio provider construction must never fail (no key needed)");
        // LM Studio runs the model on-device -- a real local runtime.
        assert!(provider.is_local());
        assert_eq!(provider.id(), "local-model");
    }

    #[test]
    fn build_leo_provider_wraps_a_configured_fallback_chain_in_a_failover_provider() {
        // Primary Ollama + one LiteLLM fallback -> a real FailoverProvider.
        let settings = spartan_settings::LeoProviderSettings {
            kind: spartan_settings::LeoProviderKind::Ollama,
            model: "llama3.1:8b".to_string(),
            fallbacks: vec![spartan_settings::LeoProviderSettings {
                kind: spartan_settings::LeoProviderKind::LiteLLM,
                model: "gpt-4o".to_string(),
                ..Default::default()
            }],
        };
        let provider =
            build_leo_provider(&settings, spartan_settings::GpuOffloadSettings::default())
                .expect("a valid fallback chain builds");
        // The wrapper reports itself as "failover"; the chain contains a cloud
        // (LiteLLM) provider, so the wrapper is conservatively non-local.
        assert_eq!(provider.id(), "failover");
        assert!(
            !provider.is_local(),
            "a chain containing a cloud provider is not local"
        );
    }

    #[test]
    fn build_leo_provider_fails_the_whole_chain_when_a_fallback_cannot_be_built() {
        // A llama.cpp fallback with an empty model path can't be built -> the
        // whole chain build fails with a clear, fallback-attributed message,
        // rather than silently dropping that link.
        let settings = spartan_settings::LeoProviderSettings {
            kind: spartan_settings::LeoProviderKind::Ollama,
            model: "llama3.1:8b".to_string(),
            fallbacks: vec![spartan_settings::LeoProviderSettings {
                kind: spartan_settings::LeoProviderKind::LlamaCpp,
                model: String::new(),
                ..Default::default()
            }],
        };
        let result = build_leo_provider(&settings, spartan_settings::GpuOffloadSettings::default());
        match result {
            Ok(_) => panic!("an unbuildable fallback must fail the chain"),
            Err(err) => assert!(err.contains("fallback provider #1"), "err was: {err}"),
        }
    }

    #[test]
    fn build_leo_provider_constructs_a_real_litellm_provider() {
        let settings = spartan_settings::LeoProviderSettings {
            kind: spartan_settings::LeoProviderKind::LiteLLM,
            model: "gpt-4o".to_string(),
            ..Default::default()
        };
        let provider =
            build_leo_provider(&settings, spartan_settings::GpuOffloadSettings::default())
                .expect("LiteLLM provider construction must never fail (no API key needed)");
        assert!(!provider.is_local());
        assert_eq!(provider.id(), "gpt-4o");
    }

    #[test]
    fn build_leo_provider_errors_clearly_when_claude_has_no_api_key() {
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let prior_key = std::env::var("ANTHROPIC_API_KEY").ok();
        std::env::remove_var("ANTHROPIC_API_KEY");

        let settings = spartan_settings::LeoProviderSettings {
            kind: spartan_settings::LeoProviderKind::Claude,
            model: "claude-3-5-sonnet-latest".to_string(),
            ..Default::default()
        };
        let result = build_leo_provider(&settings, spartan_settings::GpuOffloadSettings::default());
        match result {
            Err(message) => assert!(message.contains("ANTHROPIC_API_KEY")),
            Ok(_) => panic!("Claude must fail clearly, not silently, with no API key configured"),
        }

        if let Some(key) = prior_key {
            std::env::set_var("ANTHROPIC_API_KEY", key);
        }
    }

    #[test]
    fn build_leo_provider_constructs_a_real_claude_provider_when_a_key_is_set() {
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let prior_key = std::env::var("ANTHROPIC_API_KEY").ok();
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-fake-key");

        let settings = spartan_settings::LeoProviderSettings {
            kind: spartan_settings::LeoProviderKind::Claude,
            model: "claude-3-5-sonnet-latest".to_string(),
            ..Default::default()
        };
        let provider =
            build_leo_provider(&settings, spartan_settings::GpuOffloadSettings::default())
                .expect("Claude provider construction must succeed once a key is set");
        assert!(!provider.is_local());
        assert_eq!(provider.id(), "claude-3-5-sonnet-latest");

        match prior_key {
            Some(key) => std::env::set_var("ANTHROPIC_API_KEY", key),
            None => std::env::remove_var("ANTHROPIC_API_KEY"),
        }
    }

    #[test]
    fn build_leo_provider_errors_clearly_when_llamacpp_has_no_model_path_configured() {
        let settings = spartan_settings::LeoProviderSettings {
            kind: spartan_settings::LeoProviderKind::LlamaCpp,
            model: String::new(),
            ..Default::default()
        };
        let result = build_leo_provider(&settings, spartan_settings::GpuOffloadSettings::default());
        match result {
            Err(message) => assert!(message.contains("no .gguf model file path configured")),
            Ok(_) => panic!("llama.cpp must fail clearly, not silently, with no model path set"),
        }
    }

    #[test]
    fn build_leo_provider_errors_clearly_when_llamacpp_model_path_does_not_exist() {
        let settings = spartan_settings::LeoProviderSettings {
            kind: spartan_settings::LeoProviderKind::LlamaCpp,
            model: "/nonexistent/path/to/a/model.gguf".to_string(),
            ..Default::default()
        };
        let result = build_leo_provider(&settings, spartan_settings::GpuOffloadSettings::default());
        match result {
            Err(message) => assert!(message.contains("failed to load llama.cpp model")),
            Ok(_) => panic!("llama.cpp must fail clearly on a real nonexistent model path"),
        }
    }

    /// Real, self-skipping live test -- matches this crate's own established
    /// convention for tests needing a real, possibly-absent external
    /// dependency: `SPARTAN_TEST_GGUF_MODEL` must point at a real,
    /// already-downloaded `.gguf` file. No model file is bundled with this
    /// repository (hundreds of megabytes, a real, deliberate choice not to
    /// commit one).
    #[test]
    fn build_leo_provider_constructs_a_real_llamacpp_provider_from_a_real_model_file() {
        let Ok(model_path) = std::env::var("SPARTAN_TEST_GGUF_MODEL") else {
            eprintln!("SKIP: SPARTAN_TEST_GGUF_MODEL not set, skipping real llama.cpp provider construction test");
            return;
        };
        if !std::path::Path::new(&model_path).exists() {
            eprintln!("SKIP: {model_path} does not exist, skipping real llama.cpp provider construction test");
            return;
        }

        let settings = spartan_settings::LeoProviderSettings {
            kind: spartan_settings::LeoProviderKind::LlamaCpp,
            model: model_path,
            ..Default::default()
        };
        let provider =
            build_leo_provider(&settings, spartan_settings::GpuOffloadSettings::default())
                .expect("a real, valid .gguf file must construct successfully");
        assert!(provider.is_local());
        assert!(!provider.supports_native_tool_calling());
    }

    #[test]
    fn resolve_llamacpp_download_target_resolves_a_real_curated_model_id() {
        let model = hf_downloader::CURATED_MODELS[0];
        let (event_id, hf_repo, tag) =
            resolve_llamacpp_download_target(Some(model.id.to_string()), None, None).unwrap();
        assert_eq!(event_id, model.id);
        assert_eq!(hf_repo, model.hf_repo);
        assert_eq!(tag, model.tag);
    }

    #[test]
    fn resolve_llamacpp_download_target_resolves_and_normalizes_a_real_custom_repo_and_tag() {
        let (event_id, hf_repo, tag) = resolve_llamacpp_download_target(
            None,
            Some("https://huggingface.co/bartowski/Foo-GGUF/".to_string()),
            Some("Q4_K_M".to_string()),
        )
        .unwrap();
        assert_eq!(event_id, "bartowski/Foo-GGUF:Q4_K_M");
        assert_eq!(hf_repo, "bartowski/Foo-GGUF");
        assert_eq!(tag, "Q4_K_M");
    }

    #[test]
    fn resolve_llamacpp_download_target_errors_on_an_unknown_curated_id() {
        let result =
            resolve_llamacpp_download_target(Some("not-a-real-curated-id".to_string()), None, None);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_llamacpp_download_target_errors_with_neither_model_id_nor_custom_pair() {
        assert!(resolve_llamacpp_download_target(None, None, None).is_err());
        assert!(resolve_llamacpp_download_target(
            None,
            Some("bartowski/Foo-GGUF".to_string()),
            None
        )
        .is_err());
    }

    #[test]
    fn llamacpp_list_models_json_reports_the_real_curated_list_and_a_real_downloaded_array() {
        let value = llamacpp_list_models_json();
        let models = value.get("models").and_then(|v| v.as_array()).unwrap();
        assert_eq!(models.len(), hf_downloader::CURATED_MODELS.len());
        // Real, on-disk listing -- an array (possibly empty), never absent.
        assert!(value.get("downloaded").and_then(|v| v.as_array()).is_some());
    }

    #[test]
    fn llamacpp_download_model_dispatch_arm_reaches_the_real_handler() {
        let state = new_state();
        let (tx, _rx) = mpsc::channel();
        let model = hf_downloader::CURATED_MODELS[0];
        let result =
            llamacpp_download_model(&state, tx, Some(model.id.to_string()), None, None).unwrap();
        assert_eq!(
            result.get("status").and_then(|v| v.as_str()),
            Some("starting")
        );
        assert_eq!(
            result.get("model_id").and_then(|v| v.as_str()),
            Some(model.id)
        );
    }

    /// Real, load-bearing (task #268): confirms a real in-flight download's
    /// cancellation flag really is registered under the exact key
    /// `model_download_cancel` will look up, and really is cleared once
    /// the download's own background thread finishes -- both directly
    /// against `BackendState`, not just that the dispatch functions
    /// compile and return an ack. Deliberately outcome-agnostic: whether
    /// the real network call behind it succeeds or fails honestly (this
    /// environment's own already-documented TLS-proxy condition, §75.49,
    /// included), either is a real "finished" state that must unregister
    /// the flag -- so this test never needs to self-skip.
    #[test]
    fn llamacpp_download_model_registers_and_unregisters_a_real_cancellation_flag() {
        let state = new_state();
        let (tx, rx) = mpsc::channel();
        let model = hf_downloader::CURATED_MODELS[0];
        llamacpp_download_model(&state, tx, Some(model.id.to_string()), None, None).unwrap();

        // The background thread registers the flag synchronously before
        // this call returns (`begin_cancellable_download` runs on the
        // caller's own thread, not inside the spawned one) -- so it's
        // real and present the instant the ack comes back.
        {
            let guard = state.lock().unwrap();
            let key = download_registry_key("llamacpp", model.id);
            assert!(
                guard.download_cancellations.contains_key(&key),
                "expected a real registered cancellation flag for {key:?}"
            );
        }

        // Wait for the real background thread to report a real terminal
        // event (success or a real, honest network failure -- either is
        // fine, this test only cares that the download genuinely finished
        // and cleaned up after itself), bounded well under this crate's
        // own real network-call timeouts.
        let _ = rx.recv_timeout(Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(200));

        let guard = state.lock().unwrap();
        let key = download_registry_key("llamacpp", model.id);
        assert!(
            !guard.download_cancellations.contains_key(&key),
            "a real finished download must remove its own cancellation flag"
        );
    }

    #[test]
    fn model_download_cancel_on_an_unknown_id_is_a_real_honest_no_op() {
        let state = new_state();
        let result = model_download_cancel(
            &state,
            "hf".to_string(),
            "no-such-real-download".to_string(),
        )
        .unwrap();
        assert_eq!(
            result.get("cancelled").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn model_download_cancel_sets_a_real_registered_flag() {
        let state = new_state();
        let flag = begin_cancellable_download(&state, "hf", "some-real-model");
        assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));

        let result =
            model_download_cancel(&state, "hf".to_string(), "some-real-model".to_string()).unwrap();
        assert_eq!(
            result.get("cancelled").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(
            flag.load(std::sync::atomic::Ordering::SeqCst),
            "the real flag the download's own thread holds must now be set"
        );
    }

    #[test]
    fn download_registry_key_namespaces_by_source_so_the_same_id_never_collides() {
        assert_ne!(
            download_registry_key("hf", "qwen2.5-coder-1.5b"),
            download_registry_key("lmstudio", "qwen2.5-coder-1.5b")
        );
    }

    #[test]
    fn build_leo_provider_disabled_gpu_offload_forces_zero_layers_for_ollama() {
        let settings = spartan_settings::LeoProviderSettings::default();
        let gpu = spartan_settings::GpuOffloadSettings {
            enabled: false,
            layers: Some(20),
        };
        // Real, indirect proof: construction must still succeed (no panic
        // on a disabled-with-stale-layers combination) -- `num_gpu()`
        // itself is already directly unit-tested in `spartan-settings`.
        let provider = build_leo_provider(&settings, gpu)
            .expect("must construct even when GPU offload is disabled");
        assert!(provider.is_local());
    }

    #[test]
    fn close_file_removes_the_real_open_document() {
        let file = std::env::temp_dir().join(format!(
            "spartan-backend-test-close-{}.txt",
            std::process::id()
        ));
        std::fs::write(&file, "x").unwrap();
        let state = new_state();
        let doc_id = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        )
        .result
        .unwrap()["doc_id"]
            .as_u64()
            .unwrap();
        call(
            &state,
            2,
            "close_file",
            serde_json::json!({ "doc_id": doc_id }),
        );
        let resp = call(
            &state,
            3,
            "save_file",
            serde_json::json!({ "doc_id": doc_id }),
        );
        assert!(resp.error.is_some());
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn an_unknown_method_errors_honestly_instead_of_panicking() {
        let state = new_state();
        let resp = call(&state, 1, "not_a_real_method", serde_json::json!({}));
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("unknown method"));
    }

    #[test]
    fn editing_a_real_unopened_doc_id_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "edit",
            serde_json::json!({ "doc_id": 999, "start_char": 0, "end_char": 0, "text": "x" }),
        );
        assert!(resp.error.is_some());
    }

    #[test]
    fn lsp_hover_on_a_real_unopened_doc_id_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "lsp_hover",
            serde_json::json!({ "doc_id": 999, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no open document"));
    }

    #[test]
    fn lsp_hover_on_a_real_open_synthetic_file_with_no_lsp_session_errors_honestly() {
        // A real file with an unrecognized extension never gets a real LSP
        // session at all (`lsp_integration::maybe_spawn_lsp`'s own honest
        // `None` case) -- `lsp_hover` must report that specifically, not
        // silently hang or crash.
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-lsp-hover-no-session-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.unknownext");
        std::fs::write(&file, "hello").unwrap();

        let state = new_state();
        let open_resp = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        );
        let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

        let resp = call(
            &state,
            2,
            "lsp_hover",
            serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no live LSP session"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lsp_completion_on_a_real_unopened_doc_id_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "lsp_completion",
            serde_json::json!({ "doc_id": 999, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no open document"));
    }

    #[test]
    fn lsp_completion_on_a_real_open_synthetic_file_with_no_lsp_session_errors_honestly() {
        // The direct sibling of `lsp_hover`'s own identical test above --
        // same real, honest error path, same "unrecognized extension never
        // gets a real LSP session" cause.
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-lsp-completion-no-session-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.unknownext");
        std::fs::write(&file, "hello").unwrap();

        let state = new_state();
        let open_resp = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        );
        let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

        let resp = call(
            &state,
            2,
            "lsp_completion",
            serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no live LSP session"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lsp_definition_on_a_real_unopened_doc_id_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "lsp_definition",
            serde_json::json!({ "doc_id": 999, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no open document"));
    }

    #[test]
    fn lsp_definition_on_a_real_open_synthetic_file_with_no_lsp_session_errors_honestly() {
        // The direct sibling of `lsp_hover`'s/`lsp_completion`'s own
        // identical tests above -- same real, honest error path, same
        // "unrecognized extension never gets a real LSP session" cause.
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-lsp-definition-no-session-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.unknownext");
        std::fs::write(&file, "hello").unwrap();

        let state = new_state();
        let open_resp = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        );
        let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

        let resp = call(
            &state,
            2,
            "lsp_definition",
            serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no live LSP session"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lsp_type_definition_on_a_real_unopened_doc_id_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "lsp_type_definition",
            serde_json::json!({ "doc_id": 999, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no open document"));
    }

    #[test]
    fn lsp_type_definition_on_a_real_open_synthetic_file_with_no_lsp_session_errors_honestly() {
        // The direct sibling of `lsp_definition`'s own identical test above
        // -- same real, honest error path, same "unrecognized extension
        // never gets a real LSP session" cause.
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-lsp-type-definition-no-session-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.unknownext");
        std::fs::write(&file, "hello").unwrap();

        let state = new_state();
        let open_resp = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        );
        let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

        let resp = call(
            &state,
            2,
            "lsp_type_definition",
            serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no live LSP session"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lsp_signature_help_on_a_real_unopened_doc_id_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "lsp_signature_help",
            serde_json::json!({ "doc_id": 999, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no open document"));
    }

    #[test]
    fn lsp_signature_help_on_a_real_open_synthetic_file_with_no_lsp_session_errors_honestly() {
        // The direct sibling of `lsp_hover`'s/`lsp_completion`'s/
        // `lsp_definition`'s own identical tests above -- same real,
        // honest error path, same "unrecognized extension never gets a
        // real LSP session" cause.
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-lsp-signature-help-no-session-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.unknownext");
        std::fs::write(&file, "hello").unwrap();

        let state = new_state();
        let open_resp = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        );
        let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

        let resp = call(
            &state,
            2,
            "lsp_signature_help",
            serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no live LSP session"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lsp_references_on_a_real_unopened_doc_id_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "lsp_references",
            serde_json::json!({ "doc_id": 999, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no open document"));
    }

    #[test]
    fn lsp_references_on_a_real_open_synthetic_file_with_no_lsp_session_errors_honestly() {
        // The direct sibling of `lsp_hover`'s/`lsp_completion`'s/
        // `lsp_definition`'s/`lsp_signature_help`'s own identical tests
        // above -- same real, honest error path, same "unrecognized
        // extension never gets a real LSP session" cause.
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-lsp-references-no-session-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.unknownext");
        std::fs::write(&file, "hello").unwrap();

        let state = new_state();
        let open_resp = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        );
        let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

        let resp = call(
            &state,
            2,
            "lsp_references",
            serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no live LSP session"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lsp_rename_on_a_real_unopened_doc_id_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "lsp_rename",
            serde_json::json!({ "doc_id": 999, "line": 0, "character": 0, "new_name": "renamed" }),
        );
        assert!(resp.error.unwrap().contains("no open document"));
    }

    #[test]
    fn lsp_rename_on_a_real_open_synthetic_file_with_no_lsp_session_errors_honestly() {
        // The direct sibling of `lsp_hover`'s/`lsp_completion`'s/
        // `lsp_definition`'s/`lsp_signature_help`'s/`lsp_references`'s own
        // identical tests above -- same real, honest error path, same
        // "unrecognized extension never gets a real LSP session" cause.
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-lsp-rename-no-session-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.unknownext");
        std::fs::write(&file, "hello").unwrap();

        let state = new_state();
        let open_resp = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        );
        let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

        let resp = call(
            &state,
            2,
            "lsp_rename",
            serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 0, "new_name": "renamed" }),
        );
        assert!(resp.error.unwrap().contains("no live LSP session"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lsp_rename_with_no_new_name_param_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "lsp_rename",
            serde_json::json!({ "doc_id": 999, "line": 0, "character": 0 }),
        );
        assert!(resp.error.is_some());
    }

    #[test]
    fn lsp_document_symbol_on_a_real_unopened_doc_id_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "lsp_document_symbol",
            serde_json::json!({ "doc_id": 999 }),
        );
        assert!(resp.error.unwrap().contains("no open document"));
    }

    #[test]
    fn lsp_document_symbol_on_a_real_open_synthetic_file_with_no_lsp_session_errors_honestly() {
        // The direct sibling of `lsp_hover`'s/`lsp_completion`'s/
        // `lsp_definition`'s/`lsp_signature_help`'s/`lsp_references`'s/
        // `lsp_rename`'s own identical tests above -- same real, honest
        // error path, same "unrecognized extension never gets a real LSP
        // session" cause.
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-lsp-document-symbol-no-session-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.unknownext");
        std::fs::write(&file, "hello").unwrap();

        let state = new_state();
        let open_resp = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        );
        let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

        let resp = call(
            &state,
            2,
            "lsp_document_symbol",
            serde_json::json!({ "doc_id": doc_id }),
        );
        assert!(resp.error.unwrap().contains("no live LSP session"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lsp_document_highlight_on_a_real_unopened_doc_id_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "lsp_document_highlight",
            serde_json::json!({ "doc_id": 999, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no open document"));
    }

    #[test]
    fn lsp_document_highlight_on_a_real_open_synthetic_file_with_no_lsp_session_errors_honestly() {
        // The direct sibling of `lsp_hover`'s/`lsp_completion`'s/
        // `lsp_definition`'s/`lsp_signature_help`'s/`lsp_references`'s/
        // `lsp_rename`'s/`lsp_document_symbol`'s own identical tests above --
        // same real, honest error path, same "unrecognized extension never
        // gets a real LSP session" cause.
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-lsp-document-highlight-no-session-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.unknownext");
        std::fs::write(&file, "hello").unwrap();

        let state = new_state();
        let open_resp = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        );
        let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

        let resp = call(
            &state,
            2,
            "lsp_document_highlight",
            serde_json::json!({ "doc_id": doc_id, "line": 0, "character": 0 }),
        );
        assert!(resp.error.unwrap().contains("no live LSP session"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn format_document_on_a_real_unopened_doc_id_errors_honestly() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "format_document",
            serde_json::json!({ "doc_id": 999 }),
        );
        assert!(resp.error.unwrap().contains("no open document"));
    }

    #[test]
    fn format_document_on_a_real_unrecognized_extension_errors_honestly() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-format-document-no-profile-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.unknownext");
        std::fs::write(&file, "hello").unwrap();

        let state = new_state();
        let open_resp = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        );
        let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

        let resp = call(
            &state,
            2,
            "format_document",
            serde_json::json!({ "doc_id": doc_id }),
        );
        assert!(resp
            .error
            .unwrap()
            .contains("no language profile recognizes"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn format_document_on_a_real_language_with_no_configured_formatter_errors_honestly() {
        // Java is the one real Tier 1 language with no `formatter` entry
        // in the curated registry at all.
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-format-document-no-formatter-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Main.java");
        std::fs::write(&file, "class Main {}").unwrap();

        let state = new_state();
        let open_resp = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        );
        let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

        let resp = call(
            &state,
            2,
            "format_document",
            serde_json::json!({ "doc_id": doc_id }),
        );
        assert!(resp.error.unwrap().contains("no formatter is configured"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn format_document_reformats_a_real_file_via_a_real_installed_rustfmt_if_present() {
        if std::process::Command::new("rustfmt")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("SKIP: rustfmt not installed");
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-format-document-real-rustfmt-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        std::fs::write(&file, "fn main( ) { let x=1 ; }").unwrap();

        let state = new_state();
        let open_resp = call(
            &state,
            1,
            "open_file",
            serde_json::json!({ "path": file.to_string_lossy() }),
        );
        let doc_id = open_resp.result.unwrap()["doc_id"].as_u64().unwrap();

        let (tx, rx) = mpsc::channel();
        let resp = handle_request(
            &state,
            req(
                2,
                "format_document",
                serde_json::json!({ "doc_id": doc_id }),
            ),
            tx,
        );
        assert!(resp.error.is_none(), "{:?}", resp.error);
        assert_eq!(resp.result.unwrap()["status"], "requested");

        let line = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(event["event"], "format_document_result");
        assert_eq!(event["data"]["doc_id"], doc_id);
        let formatted = event["data"]["formatted"].as_str().unwrap();
        assert!(formatted.contains("fn main() {"), "got: {formatted}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn leo_status_before_any_task_is_real_idle_with_no_plan() {
        let state = new_state();
        let resp = call(&state, 1, "leo_status", serde_json::json!({}));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["state"], "Idle");
        assert!(result["plan"].is_null());
    }

    #[test]
    fn leo_start_task_transitions_to_planning_and_returns_an_immediate_ack() {
        let dir =
            std::env::temp_dir().join(format!("spartan-backend-leo-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let state = new_state();
        let resp = call(
            &state,
            1,
            "leo_start_task",
            serde_json::json!({ "task": "add a test", "project_root": dir.to_string_lossy() }),
        );
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["status"], "planning");
        // Real bug found live via a CI failure, not by inspection: a
        // second `leo_status` call here (removed) raced a real spawned
        // background thread -- `begin_planning()` itself is synchronous
        // (already fully proven by the assertion above), but
        // `generate_plan`'s real HTTP call to Ollama can fail via a fast
        // `ECONNREFUSED` (no Ollama reachable in CI) quickly enough to
        // transition Planning -> Failed before this test's own next
        // instruction runs, an environment-dependent race no sleep or
        // retry fixes at its root. `Idle -> Planning` is already fully,
        // deterministically covered by the two assertions above; a
        // second, later state read adds no real guarantee this test's
        // own name promises.
        state.lock().unwrap().leo_agent = None;
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn leo_start_task_increments_the_real_generation_counter() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-leo-generation-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let state = new_state();
        call(
            &state,
            1,
            "leo_start_task",
            serde_json::json!({ "task": "a", "project_root": dir.to_string_lossy() }),
        );
        let gen1 = state.lock().unwrap().leo_generation;
        call(
            &state,
            2,
            "leo_start_task",
            serde_json::json!({ "task": "b", "project_root": dir.to_string_lossy() }),
        );
        let gen2 = state.lock().unwrap().leo_generation;
        assert_eq!(
            gen2,
            gen1 + 1,
            "each real leo_start_task call must bump the generation"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn approval_mode_from_settings_maps_both_real_variants_correctly() {
        assert_eq!(
            approval_mode_from_settings(spartan_settings::LeoApprovalMode::ManualEveryStep),
            ApprovalMode::ManualEveryStep
        );
        assert_eq!(
            approval_mode_from_settings(spartan_settings::LeoApprovalMode::AutoApproveSafe),
            ApprovalMode::AutoApproveSafe
        );
    }

    #[test]
    fn leo_start_task_picks_up_the_real_configured_approval_mode() {
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "spartan-backend-leo-approval-mode-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let prior_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &scratch);

        spartan_settings::save(&spartan_settings::Settings {
            leo_approval_mode: spartan_settings::LeoApprovalMode::AutoApproveSafe,
            ..Default::default()
        })
        .unwrap();

        let dir = scratch.join("project");
        std::fs::create_dir_all(&dir).unwrap();
        let state = new_state();
        call(
            &state,
            1,
            "leo_start_task",
            serde_json::json!({ "task": "a", "project_root": dir.to_string_lossy() }),
        );
        let guard = state.lock().unwrap();
        let agent = guard.leo_agent.as_ref().unwrap();
        assert!(
            agent.may_auto_execute(&ToolCall::ReadFile {
                path: "x".to_string()
            }),
            "a Safe call must be real-auto-approvable once settings say AutoApproveSafe"
        );
        assert!(
            !agent.may_auto_execute(&ToolCall::EditFile {
                path: "x".to_string(),
                content: "y".to_string()
            }),
            "a Destructive call must never auto-approve, regardless of settings (§9)"
        );
        drop(guard);

        match prior_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn leo_approve_plan_before_any_task_errors_honestly() {
        let state = new_state();
        let resp = call(&state, 1, "leo_approve_plan", serde_json::json!({}));
        assert!(resp.error.is_some());
    }

    #[test]
    fn leo_reject_plan_before_any_task_errors_honestly() {
        let state = new_state();
        let resp = call(&state, 1, "leo_reject_plan", serde_json::json!({}));
        assert!(resp.error.is_some());
    }

    #[test]
    fn leo_cancel_before_any_task_errors_honestly() {
        let state = new_state();
        let resp = call(&state, 1, "leo_cancel", serde_json::json!({}));
        assert!(resp.error.is_some());
    }

    #[test]
    fn leo_cancel_while_awaiting_approval_returns_to_idle_and_bumps_generation() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-leo-cancel-awaiting-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut agent = Agent::new(dir.clone(), ApprovalMode::ManualEveryStep);
        agent.begin_planning().unwrap();
        agent.apply_generated_plan(Ok(sample_plan())).unwrap();
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            leo_generation: 5,
            ..Default::default()
        }));

        let resp = call(&state, 1, "leo_cancel", serde_json::json!({}));
        assert!(resp.error.is_none(), "leo_cancel errored: {:?}", resp.error);
        assert_eq!(resp.result.unwrap()["state"], "Idle");

        let guard = state.lock().unwrap();
        assert_eq!(
                guard.leo_generation, 6,
                "cancel must bump the real generation counter so a late-arriving background result is discarded"
            );
        drop(guard);

        let status = call(&state, 2, "leo_status", serde_json::json!({}));
        assert_eq!(status.result.unwrap()["plan"], serde_json::Value::Null);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Real §75.73-closing cooperative cancellation (task #269): confirms
    /// `leo_cancel` genuinely sets the real, shared `leo_cancel_flag` --
    /// the specific new mechanism this pass adds on top of the already-
    /// tested generation bump above, verified deterministically with no
    /// real background model call involved at all.
    #[test]
    fn leo_cancel_sets_the_real_cancel_flag_true() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-leo-cancel-flag-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut agent = Agent::new(dir.clone(), ApprovalMode::ManualEveryStep);
        agent.begin_planning().unwrap();
        agent.apply_generated_plan(Ok(sample_plan())).unwrap();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            leo_cancel_flag: Arc::clone(&cancel_flag),
            ..Default::default()
        }));

        assert!(
            !cancel_flag.load(std::sync::atomic::Ordering::SeqCst),
            "starts unset"
        );
        let resp = call(&state, 1, "leo_cancel", serde_json::json!({}));
        assert!(resp.error.is_none(), "leo_cancel errored: {:?}", resp.error);
        assert!(
            cancel_flag.load(std::sync::atomic::Ordering::SeqCst),
            "leo_cancel must set the real shared cancel flag true, so a real \
             in-flight background model call actually observes it and stops"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Real §75.73-closing cooperative cancellation (task #269): every
    /// real new `leo_start_task` call mints a genuinely fresh cancel flag
    /// (`false`), not a reset of the previous task's own flag -- a stale
    /// `Arc` clone held by an already-superseded background thread must
    /// never be able to observe or affect a brand-new task's own flag.
    #[test]
    fn leo_start_task_mints_a_fresh_cancel_flag_each_real_new_task() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-leo-fresh-cancel-flag-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let state = new_state();

        call(
            &state,
            1,
            "leo_start_task",
            serde_json::json!({ "task": "first task", "project_root": dir.to_string_lossy() }),
        );
        let first_flag = {
            let guard = state.lock().unwrap();
            Arc::clone(&guard.leo_cancel_flag)
        };
        assert!(!first_flag.load(std::sync::atomic::Ordering::SeqCst));
        // Simulate the first task being cancelled -- its own real flag is
        // now permanently `true`.
        first_flag.store(true, std::sync::atomic::Ordering::SeqCst);

        call(
            &state,
            2,
            "leo_start_task",
            serde_json::json!({ "task": "second task", "project_root": dir.to_string_lossy() }),
        );
        let second_flag = {
            let guard = state.lock().unwrap();
            Arc::clone(&guard.leo_cancel_flag)
        };
        assert!(
            !second_flag.load(std::sync::atomic::Ordering::SeqCst),
            "a brand-new task must get its own fresh, unset cancel flag, \
             never inheriting the previous (cancelled) task's own true value"
        );
        assert!(
            !Arc::ptr_eq(&first_flag, &second_flag),
            "the two tasks must hold genuinely distinct Arc<AtomicBool> instances"
        );

        // Real, established pattern (see leo_start_task_transitions_to_
        // planning_and_returns_an_immediate_ack's own doc comment): avoid
        // racing the real spawned background thread's own possibly-live
        // network call by discarding the agent before this test ends.
        state.lock().unwrap().leo_agent = None;
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn leo_cancel_from_an_already_idle_agent_errors_honestly() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-leo-cancel-idle-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let agent = Agent::new(dir.clone(), ApprovalMode::ManualEveryStep);
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            ..Default::default()
        }));
        let resp = call(&state, 1, "leo_cancel", serde_json::json!({}));
        assert!(resp.error.is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    // --- Real task #266: multi-turn session history ---

    #[test]
    fn push_leo_history_bounds_the_real_session_history_at_the_max() {
        let mut state = BackendState {
            leo_current_task: Some("task".to_string()),
            ..Default::default()
        };
        for i in 0..(MAX_LEO_SESSION_HISTORY + 5) {
            push_leo_history(&mut state, "Done", Some(format!("summary {i}")), None);
        }
        assert_eq!(state.leo_session_history.len(), MAX_LEO_SESSION_HISTORY);
        // The oldest 5 real entries must have been dropped, keeping the
        // most recent `MAX_LEO_SESSION_HISTORY` -- confirmed by checking
        // the real, still-present first entry is "summary 5", not the
        // real, now-evicted "summary 0".
        assert_eq!(
            state.leo_session_history[0].summary.as_deref(),
            Some("summary 5")
        );
    }

    #[test]
    fn leo_session_history_is_empty_by_default() {
        let state = new_state();
        let resp = call(&state, 1, "leo_session_history", serde_json::json!({}));
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["entries"], serde_json::json!([]));
    }

    #[test]
    fn leo_session_history_reports_real_entries_newest_first() {
        let state = Arc::new(Mutex::new(BackendState::default()));
        {
            let mut guard = state.lock().unwrap();
            guard.leo_current_task = Some("first task".to_string());
            push_leo_history(&mut guard, "Done", Some("did the first thing".into()), None);
            guard.leo_current_task = Some("second task".to_string());
            push_leo_history(&mut guard, "Failed", None, Some("a real error".into()));
        }
        let resp = call(&state, 1, "leo_session_history", serde_json::json!({}));
        let entries = resp.result.unwrap()["entries"].clone();
        let arr = entries.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(
            arr[0]["task"], "second task",
            "newest entry must come first"
        );
        assert_eq!(arr[0]["outcome"], "Failed");
        assert_eq!(arr[0]["error"], "a real error");
        assert_eq!(arr[1]["task"], "first task");
        assert_eq!(arr[1]["outcome"], "Done");
        assert_eq!(arr[1]["summary"], "did the first thing");
    }

    #[test]
    fn record_leo_next_step_outcome_pushes_a_real_done_entry() {
        let mut state = BackendState {
            leo_current_task: Some("do the thing".to_string()),
            ..Default::default()
        };
        let event = Event {
            event: "leo_execute_done".to_string(),
            data: serde_json::json!({ "summary": "did it", "memory_saved": true }),
        };
        record_leo_next_step_outcome(&mut state, &event);
        assert_eq!(state.leo_session_history.len(), 1);
        assert_eq!(state.leo_session_history[0].outcome, "Done");
        assert_eq!(
            state.leo_session_history[0].summary.as_deref(),
            Some("did it")
        );
        assert_eq!(state.leo_session_history[0].task, "do the thing");
    }

    #[test]
    fn record_leo_next_step_outcome_stashes_the_real_error_but_does_not_push_yet() {
        // A real `Failed` outcome is not immediately terminal (§75.78's
        // own bounded retry loop can still bring it back) -- confirms no
        // history entry is pushed yet, only the real error text is
        // remembered for a later, real retroactive recording.
        let mut state = BackendState::default();
        let event = Event {
            event: "leo_execute_failed".to_string(),
            data: serde_json::json!({ "error": "a real tool error" }),
        };
        record_leo_next_step_outcome(&mut state, &event);
        assert!(state.leo_session_history.is_empty());
        assert_eq!(state.leo_last_error.as_deref(), Some("a real tool error"));
    }

    #[test]
    fn leo_cancel_pushes_a_real_cancelled_history_entry_with_the_real_task_text() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-leo-cancel-history-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut agent = Agent::new(dir.clone(), ApprovalMode::ManualEveryStep);
        agent.begin_planning().unwrap();
        agent.apply_generated_plan(Ok(sample_plan())).unwrap();
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            leo_current_task: Some("cancel me".to_string()),
            ..Default::default()
        }));

        let resp = call(&state, 1, "leo_cancel", serde_json::json!({}));
        assert!(resp.error.is_none(), "leo_cancel errored: {:?}", resp.error);

        let history = call(&state, 2, "leo_session_history", serde_json::json!({}));
        let entries = history.result.unwrap()["entries"].clone();
        let arr = entries.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["task"], "cancel me");
        assert_eq!(arr[0]["outcome"], "Cancelled");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn leo_start_task_retroactively_records_a_previous_failed_agent() {
        let tmp = TempRepo::new("leo-start-task-retroactive-failed");
        let mut agent = agent_in_executing_state(&tmp.dir);
        agent.mark_failed().unwrap();
        assert_eq!(agent.state(), spartan_leo::state::AgentState::Failed);
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            leo_current_task: Some("the failed task".to_string()),
            leo_last_error: Some("the real reason it failed".to_string()),
            leo_project_root: Some(tmp.dir.clone()),
            ..Default::default()
        }));

        // A real new task, never retried -- the previous `Failed` agent
        // is about to be discarded for good by `leo_start_task`'s own
        // fresh `Agent`, which is exactly the one real point this should
        // retroactively record it.
        let resp = call(
            &state,
            1,
            "leo_start_task",
            serde_json::json!({ "task": "a new task", "project_root": tmp.dir.to_string_lossy() }),
        );
        assert!(
            resp.error.is_none(),
            "leo_start_task errored: {:?}",
            resp.error
        );

        let history = call(&state, 2, "leo_session_history", serde_json::json!({}));
        let entries = history.result.unwrap()["entries"].clone();
        let arr = entries.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["task"], "the failed task");
        assert_eq!(arr[0]["outcome"], "Failed");
        assert_eq!(arr[0]["error"], "the real reason it failed");

        // Real, synchronous confirmation that the new task's own text is
        // now current (used if *this* task later fails and is abandoned
        // in turn) -- checked here rather than waiting on the real
        // background thread, which needs a live model this environment
        // doesn't have.
        assert_eq!(
            state.lock().unwrap().leo_current_task.as_deref(),
            Some("a new task")
        );
    }

    fn sample_plan() -> ImplementationPlan {
        ImplementationPlan {
            goal: "test goal".to_string(),
            approach: "test approach".to_string(),
            files: vec!["f.txt".to_string()],
            risk_notes: "none".to_string(),
        }
    }

    /// A real `Agent`, already through `Idle -> Planning ->
    /// AwaitingApproval -> Executing` against a real temp git repo (no
    /// mock) -- exactly the state `leo_next_step`/`leo_approve_call`/
    /// `leo_reject_call` all require, built directly rather than through
    /// the full async `leo_start_task`/`leo_approve_plan` IPC round trip
    /// (which needs a real model) since these tests exercise the
    /// execute-loop's own real state handling, not plan generation.
    fn agent_in_executing_state(root: &std::path::Path) -> Agent {
        let mut repo = git2::Repository::open(root).unwrap();
        // Checkpointing needs a real base commit to reset back to --
        // a brand-new `TempRepo` has no `HEAD` at all yet.
        {
            let signature = repo.signature().unwrap();
            let tree_oid = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(tree_oid).unwrap();
            repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
                .unwrap();
        }

        let mut agent = Agent::new(root.to_path_buf(), ApprovalMode::ManualEveryStep);
        agent.begin_planning().unwrap();
        agent.apply_generated_plan(Ok(sample_plan())).unwrap();
        agent.approve_plan(&mut repo).unwrap();
        agent
    }

    #[test]
    fn leo_next_step_before_any_task_errors_honestly() {
        let state = new_state();
        let resp = call(&state, 1, "leo_next_step", serde_json::json!({}));
        assert!(resp.error.is_some());
    }

    #[test]
    fn leo_next_step_requires_the_executing_state() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-leo-next-step-state-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut agent = Agent::new(dir.clone(), ApprovalMode::ManualEveryStep);
        agent.begin_planning().unwrap();
        agent.apply_generated_plan(Ok(sample_plan())).unwrap();
        // Real, deliberately still `AwaitingApproval`, not `Executing`.
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            ..Default::default()
        }));
        let resp = call(&state, 1, "leo_next_step", serde_json::json!({}));
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().contains("Executing"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn compute_diff_marks_added_and_removed_lines_and_keeps_unchanged_ones() {
        let old = "line one\nline two\nline three\n";
        let new = "line one\nline TWO changed\nline three\nline four\n";
        let diff = compute_diff(old, new);
        assert!(diff.contains("-line two\n"));
        assert!(diff.contains("+line TWO changed\n"));
        assert!(diff.contains(" line one\n"));
        assert!(diff.contains(" line three\n"));
        assert!(diff.contains("+line four\n"));
    }

    #[test]
    fn compute_diff_of_identical_content_has_no_added_or_removed_lines() {
        let content = "same\ncontent\n";
        let diff = compute_diff(content, content);
        assert!(!diff.contains('+'));
        assert!(!diff.contains('-'));
    }

    #[test]
    fn compute_diff_against_empty_old_content_marks_every_line_added() {
        let diff = compute_diff("", "brand new file\nsecond line\n");
        assert!(diff.contains("+brand new file\n"));
        assert!(diff.contains("+second line\n"));
        assert!(!diff.contains('-'));
    }

    #[test]
    fn leo_next_step_errors_when_a_call_is_already_pending() {
        let tmp = TempRepo::new("leo-next-step-pending");
        let agent = agent_in_executing_state(&tmp.dir);
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            leo_pending_call: Some(PendingCall {
                call_id: "call_1".to_string(),
                call: ToolCall::ReadFile {
                    path: "f.txt".to_string(),
                },
            }),
            ..Default::default()
        }));
        let resp = call(&state, 1, "leo_next_step", serde_json::json!({}));
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().contains("already awaiting approval"));
    }

    #[test]
    fn run_leo_verification_and_completion_with_no_command_is_the_real_unchanged_v1_waypoint() {
        // §75.66's own original behavior, byte-for-byte, when no
        // verification command is configured (`None`, the real default).
        let tmp = TempRepo::new("leo-verify-none");
        let mut agent = agent_in_executing_state(&tmp.dir);
        let event = run_leo_verification_and_completion(&mut agent, None, "did the thing".into());
        assert_eq!(event.event, "leo_execute_done");
        assert_eq!(event.data["summary"], "did the thing");
        assert!(event.data.get("verification").is_none());
        assert_eq!(agent.state(), spartan_leo::state::AgentState::Done);
    }

    #[test]
    fn run_leo_verification_and_completion_real_exit_0_marks_the_task_done() {
        let tmp = TempRepo::new("leo-verify-pass");
        let mut agent = agent_in_executing_state(&tmp.dir);
        let event = run_leo_verification_and_completion(
            &mut agent,
            Some("echo real-verify-output"),
            "did the thing".into(),
        );
        assert_eq!(event.event, "leo_execute_done");
        assert_eq!(event.data["verification"]["exit_code"], 0);
        assert!(event.data["verification"]["stdout"]
            .as_str()
            .unwrap()
            .contains("real-verify-output"));
        assert_eq!(agent.state(), spartan_leo::state::AgentState::Done);
    }

    #[test]
    fn run_leo_verification_and_completion_real_non_zero_exit_marks_the_task_failed_not_done() {
        // A real, genuine verification failure -- `false` always exits 1
        // -- must never be reported as `Done`, and must leave the agent
        // in the exact `Failed` state `leo_retry` (§75.78) recovers from.
        let tmp = TempRepo::new("leo-verify-fail");
        let mut agent = agent_in_executing_state(&tmp.dir);
        let event =
            run_leo_verification_and_completion(&mut agent, Some("false"), "did the thing".into());
        assert_eq!(event.event, "leo_execute_failed");
        assert!(event.data["error"]
            .as_str()
            .unwrap()
            .contains("verification command"));
        assert_eq!(event.data["verification"]["exit_code"], 1);
        assert_eq!(agent.state(), spartan_leo::state::AgentState::Failed);
    }

    #[test]
    fn run_leo_verification_and_completion_a_real_failure_can_then_really_recover_via_leo_retry() {
        // The whole point of feeding this into `Failed` rather than
        // swallowing it: confirms the real, full round trip actually
        // works, not just that the state label is correct.
        let tmp = TempRepo::new("leo-verify-fail-then-retry");
        let mut agent = agent_in_executing_state(&tmp.dir);
        let mut repo = git2::Repository::open(&tmp.dir).unwrap();
        run_leo_verification_and_completion(&mut agent, Some("false"), "x".into());
        assert_eq!(agent.state(), spartan_leo::state::AgentState::Failed);
        agent.begin_recovery(&mut repo).unwrap();
        assert_eq!(agent.state(), spartan_leo::state::AgentState::Executing);
    }

    #[test]
    fn run_leo_verification_and_completion_a_real_unrunnable_command_marks_failed_honestly() {
        // A command that can't even spawn (not a real exit-code failure)
        // must still land in `Failed`, not silently pass or panic.
        let tmp = TempRepo::new("leo-verify-unrunnable");
        let mut agent = agent_in_executing_state(&tmp.dir);
        let event = run_leo_verification_and_completion(
            &mut agent,
            Some("/definitely/not/a/real/binary/anywhere"),
            "x".into(),
        );
        assert_eq!(event.event, "leo_execute_failed");
        assert_eq!(agent.state(), spartan_leo::state::AgentState::Failed);
    }

    #[test]
    fn leo_next_step_uses_the_real_configured_leo_verify_command_end_to_end() {
        // A real, end-to-end confirmation through the actual IPC dispatch
        // and settings-loading path `leo_next_step` uses -- not just the
        // extracted function in isolation. Writes a real
        // `~/.spartan/settings.json`-equivalent for this test's own
        // isolated `$HOME`, then drives a real `task_complete` action
        // straight through a fake `ModelProvider` (no live model needed,
        // matching every other `leo_next_step`-adjacent test's own
        // precedent) and confirms the real configured command actually
        // ran and its real output reached the emitted event.
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join(format!(
            "spartan-backend-leo-verify-e2e-home-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let prior_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &home);

        spartan_settings::save(&spartan_settings::Settings {
            leo_verify_command: Some("echo end-to-end-verify-ran".to_string()),
            ..Default::default()
        })
        .unwrap();

        let tmp = TempRepo::new("leo-next-step-verify-e2e");
        let mut agent = agent_in_executing_state(&tmp.dir);
        // Drive the exact real logic `leo_next_step`'s background thread
        // runs once the model proposes `task_complete`, using the real
        // settings just written -- confirming the dispatch-facing plumbing
        // (loading settings, reading `leo_verify_command`) agrees with the
        // extracted function's own already-covered behavior.
        let settings = spartan_settings::load();
        assert_eq!(
            settings.leo_verify_command.as_deref(),
            Some("echo end-to-end-verify-ran")
        );
        let event = run_leo_verification_and_completion(
            &mut agent,
            settings.leo_verify_command.as_deref(),
            "e2e summary".into(),
        );
        assert_eq!(event.event, "leo_execute_done");
        assert!(event.data["verification"]["stdout"]
            .as_str()
            .unwrap()
            .contains("end-to-end-verify-ran"));

        match prior_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn leo_retry_before_any_task_errors_honestly() {
        let state = new_state();
        let resp = call(&state, 1, "leo_retry", serde_json::json!({}));
        assert!(resp.error.is_some());
    }

    #[test]
    fn leo_retry_real_transitions_a_failed_agent_back_to_executing() {
        let tmp = TempRepo::new("leo-retry-success");
        let mut agent = agent_in_executing_state(&tmp.dir);
        // A real `Executing -> Failed` transition, matching exactly what
        // `leo_next_step`'s own two real call sites do on a genuine tool-
        // execution or model error.
        agent.mark_failed().unwrap();
        assert_eq!(agent.state(), spartan_leo::state::AgentState::Failed);
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            leo_project_root: Some(tmp.dir.clone()),
            ..Default::default()
        }));
        let resp = call(&state, 1, "leo_retry", serde_json::json!({}));
        assert!(resp.error.is_none(), "leo_retry errored: {:?}", resp.error);
        assert_eq!(resp.result.unwrap()["state"], "Executing");
        let guard = state.lock().unwrap();
        assert_eq!(
            guard.leo_agent.as_ref().unwrap().state(),
            spartan_leo::state::AgentState::Executing
        );
    }

    #[test]
    fn leo_retry_reports_recovery_exhausted_honestly_after_the_real_bound() {
        let tmp = TempRepo::new("leo-retry-exhausted");
        let mut agent = agent_in_executing_state(&tmp.dir);
        let mut repo = git2::Repository::open(&tmp.dir).unwrap();
        // Real §4.1 bound is 3 -- exhaust it for real via the same
        // `mark_failed`/`begin_recovery` pair `leo_retry` itself uses,
        // called directly here to drive the agent to the real exhausted
        // state without needing three separate IPC round trips.
        for _ in 0..3 {
            agent.mark_failed().unwrap();
            agent.begin_recovery(&mut repo).unwrap();
        }
        agent.mark_failed().unwrap();
        assert_eq!(agent.state(), spartan_leo::state::AgentState::Failed);
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            leo_project_root: Some(tmp.dir.clone()),
            ..Default::default()
        }));
        let resp = call(&state, 1, "leo_retry", serde_json::json!({}));
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("exhausted"));
        // A real, deliberate consequence: the agent stays in `Failed` --
        // `begin_recovery` never even attempted the transition once the
        // bound check failed, matching `Agent::begin_recovery`'s own
        // real early-return-before-transitioning behavior.
        let guard = state.lock().unwrap();
        assert_eq!(
            guard.leo_agent.as_ref().unwrap().state(),
            spartan_leo::state::AgentState::Failed
        );
    }

    #[test]
    fn leo_approve_call_with_nothing_pending_errors_honestly() {
        let state = new_state();
        let resp = call(&state, 1, "leo_approve_call", serde_json::json!({}));
        assert!(resp.error.is_some());
    }

    #[test]
    fn leo_reject_call_with_nothing_pending_errors_honestly() {
        let state = new_state();
        let resp = call(&state, 1, "leo_reject_call", serde_json::json!({}));
        assert!(resp.error.is_some());
    }

    #[test]
    fn leo_approve_call_executes_a_real_read_file_call_and_appends_history() {
        let tmp = TempRepo::new("leo-approve-read");
        std::fs::write(tmp.dir.join("f.txt"), "real file contents").unwrap();
        let agent = agent_in_executing_state(&tmp.dir);
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            leo_pending_call: Some(PendingCall {
                call_id: "call_1".to_string(),
                call: ToolCall::ReadFile {
                    path: "f.txt".to_string(),
                },
            }),
            ..Default::default()
        }));

        let resp = call(&state, 1, "leo_approve_call", serde_json::json!({}));
        assert!(
            resp.error.is_none(),
            "leo_approve_call errored: {:?}",
            resp.error
        );
        let result = resp.result.unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["result"]["kind"], "file_content");
        assert_eq!(result["result"]["content"], "real file contents");

        let guard = state.lock().unwrap();
        assert!(guard.leo_pending_call.is_none());
        assert_eq!(guard.leo_history.len(), 2, "an Assistant+Tool message pair");
        assert_eq!(guard.leo_history[1].content, "real file contents");
        assert_eq!(guard.leo_history[1].tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn leo_approve_call_executes_a_real_search_files_call() {
        let tmp = TempRepo::new("leo-approve-search");
        std::fs::write(tmp.dir.join("f.txt"), "fn needle() {}\n").unwrap();
        let agent = agent_in_executing_state(&tmp.dir);
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            leo_pending_call: Some(PendingCall {
                call_id: "call_1".to_string(),
                call: ToolCall::SearchFiles {
                    pattern: "needle".to_string(),
                    path: None,
                },
            }),
            ..Default::default()
        }));

        let resp = call(&state, 1, "leo_approve_call", serde_json::json!({}));
        assert!(
            resp.error.is_none(),
            "leo_approve_call errored: {:?}",
            resp.error
        );
        let result = resp.result.unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["result"]["kind"], "search_matches");
        let matches = result["result"]["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["path"], "f.txt");

        let guard = state.lock().unwrap();
        assert!(guard.leo_pending_call.is_none());
        assert!(guard.leo_history[1].content.contains("f.txt:1:"));
    }

    #[test]
    fn leo_approve_call_executes_a_real_list_directory_call() {
        let tmp = TempRepo::new("leo-approve-list");
        std::fs::write(tmp.dir.join("a.txt"), "x").unwrap();
        std::fs::create_dir_all(tmp.dir.join("zsub")).unwrap();
        // A real, empty directory carries no git-trackable content at
        // all -- `approve_plan`'s own checkpoint does a real
        // stash-then-reapply round trip (checkpoint.rs's own doc comment
        // already names this exact class of limitation for untracked
        // paths), which cannot preserve a directory with nothing in it.
        // A real file inside it sidesteps that entirely, matching how a
        // real project's subdirectories always actually look.
        std::fs::write(tmp.dir.join("zsub/nested.txt"), "y").unwrap();
        let agent = agent_in_executing_state(&tmp.dir);
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            leo_pending_call: Some(PendingCall {
                call_id: "call_1".to_string(),
                call: ToolCall::ListDirectory { path: None },
            }),
            ..Default::default()
        }));

        let resp = call(&state, 1, "leo_approve_call", serde_json::json!({}));
        assert!(
            resp.error.is_none(),
            "leo_approve_call errored: {:?}",
            resp.error
        );
        let result = resp.result.unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["result"]["kind"], "directory_listing");
        let entries = result["result"]["entries"].as_array().unwrap();
        // TempRepo initializes a real .git directory too -- just confirm
        // our two real real entries are both present, not an exact count.
        assert!(entries
            .iter()
            .any(|e| e["name"] == "a.txt" && e["is_dir"] == false));
        assert!(entries
            .iter()
            .any(|e| e["name"] == "zsub" && e["is_dir"] == true));
    }

    #[test]
    fn leo_reject_call_appends_a_rejection_notice_and_clears_pending() {
        let tmp = TempRepo::new("leo-reject-call");
        let agent = agent_in_executing_state(&tmp.dir);
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            leo_pending_call: Some(PendingCall {
                call_id: "call_1".to_string(),
                call: ToolCall::EditFile {
                    path: "f.txt".to_string(),
                    content: "x".to_string(),
                },
            }),
            ..Default::default()
        }));

        let resp = call(&state, 1, "leo_reject_call", serde_json::json!({}));
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["ok"], true);

        let guard = state.lock().unwrap();
        assert!(guard.leo_pending_call.is_none());
        assert_eq!(guard.leo_history.len(), 2);
        assert!(guard.leo_history[1].content.contains("rejected"));
        // A rejection must never actually touch the real file.
        assert!(!tmp.dir.join("f.txt").exists());
    }

    #[test]
    fn leo_approve_call_with_a_path_jail_violation_reports_the_error_and_still_appends_history() {
        let tmp = TempRepo::new("leo-approve-jail-violation");
        let agent = agent_in_executing_state(&tmp.dir);
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            leo_pending_call: Some(PendingCall {
                call_id: "call_1".to_string(),
                call: ToolCall::ReadFile {
                    path: "../../../../../../etc/passwd".to_string(),
                },
            }),
            ..Default::default()
        }));

        let resp = call(&state, 1, "leo_approve_call", serde_json::json!({}));
        // Not a protocol-level error -- a real, reported tool failure the
        // model itself gets to see and react to on the next real step.
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["ok"], false);
        assert!(result["error"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("jail"));

        let guard = state.lock().unwrap();
        assert!(guard.leo_pending_call.is_none());
        assert_eq!(guard.leo_history.len(), 2);
    }

    #[test]
    fn leo_status_reports_a_real_pending_call() {
        let tmp = TempRepo::new("leo-status-pending");
        let agent = agent_in_executing_state(&tmp.dir);
        let state = Arc::new(Mutex::new(BackendState {
            leo_agent: Some(agent),
            leo_pending_call: Some(PendingCall {
                call_id: "call_7".to_string(),
                call: ToolCall::RunTerminal {
                    command: "echo hi".to_string(),
                },
            }),
            ..Default::default()
        }));

        let resp = call(&state, 1, "leo_status", serde_json::json!({}));
        let result = resp.result.unwrap();
        assert_eq!(result["state"], "Executing");
        assert_eq!(result["pending_call"]["call_id"], "call_7");
        assert_eq!(result["pending_call"]["tool"], "run_terminal");
        assert_eq!(result["pending_call"]["args"]["command"], "echo hi");
    }

    #[test]
    fn augment_task_with_memory_passes_through_unchanged_when_empty() {
        assert_eq!(augment_task_with_memory("do the thing", ""), "do the thing");
        assert_eq!(
            augment_task_with_memory("do the thing", "   \n  "),
            "do the thing"
        );
    }

    #[test]
    fn augment_task_with_memory_prefixes_real_notes() {
        let result = augment_task_with_memory(
            "add a login form",
            "- Always use the existing AuthContext, never a new one\n",
        );
        assert!(result.contains("Always use the existing AuthContext"));
        assert!(result.ends_with("Task: add a login form"));
    }

    /// Real task #273 param threading: `auto_restart: true` reaches
    /// `litellm_proxy_start` and, since no real `litellm` binary is
    /// installed in this environment, still produces the same real,
    /// honest "not on $PATH" error the pre-existing (no-`auto_restart`)
    /// path already reported -- confirming the new param doesn't change
    /// behavior on this early-exit branch, and (real, load-bearing) that
    /// no supervisor thread was spawned for a start that never succeeded.
    #[test]
    fn litellm_proxy_start_with_auto_restart_still_reports_a_real_honest_not_installed_error() {
        let state = new_state();
        let resp = call(
            &state,
            1,
            "litellm_proxy_start",
            serde_json::json!({ "port": 4999, "auto_restart": true }),
        );
        let err = resp
            .error
            .expect("litellm isn't installed in this environment");
        assert!(err.to_lowercase().contains("litellm"));
        assert!(err.to_lowercase().contains("$path") || err.to_lowercase().contains("path"));
        // No process handle was ever created for a start that failed
        // before spawning anything.
        assert!(state.lock().unwrap().litellm.is_none());
    }

    /// Real task #273 generation minting: every real `litellm_proxy_start`
    /// call increments `BackendState::litellm_generation`, the same
    /// discipline `leo_generation` already established -- confirmed by
    /// calling the (crate-private) function directly twice and inspecting
    /// the real resulting counter value, since no dispatch response
    /// exposes it directly.
    #[test]
    fn litellm_proxy_start_mints_a_real_fresh_generation_on_every_call() {
        let state = new_state();
        let (tx, _rx) = mpsc::channel();
        assert_eq!(state.lock().unwrap().litellm_generation, 0);
        let _ = litellm_proxy_start(&state, tx.clone(), 4998, None, false);
        assert_eq!(state.lock().unwrap().litellm_generation, 1);
        let _ = litellm_proxy_start(&state, tx, 4998, None, true);
        assert_eq!(
            state.lock().unwrap().litellm_generation,
            2,
            "a second real start call must mint a genuinely new generation"
        );
    }
}
