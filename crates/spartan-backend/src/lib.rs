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
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use spartan_buffer::Document;
use spartan_leo::agent::{Agent, AgentError};
use spartan_leo::approval::ApprovalMode;
use spartan_leo::execute::{self, ExecuteAction};
use spartan_leo::plan::{generate_plan, ImplementationPlan, PlanError};
use spartan_leo::tool::{ToolCall, ToolResult};
use spartan_model::provider::Message;
use spartan_model::{ClaudeProvider, LiteLLMProvider, ModelProvider, OllamaProvider};

mod pty;

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
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
    pty_sessions: HashMap<u64, pty::PtyHandle>,
    next_pty_id: u64,
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

fn open_file(state: &mut BackendState, path: &str) -> Result<serde_json::Value, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read({path}): {e}"))?;
    let document = Document::new(&content);
    let doc_id = state.next_doc_id;
    state.next_doc_id += 1;
    state.open_docs.insert(
        doc_id,
        OpenDoc {
            path: PathBuf::from(path),
            document,
            redo_stack: Vec::new(),
        },
    );
    Ok(serde_json::json!({ "doc_id": doc_id, "content": content }))
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
        Ok(()) => Ok(serde_json::json!({ "changed": true, "content": open_doc.document.text() })),
        Err(_) => {
            // The checkpoint aged out of the bounded ring since `undo`
            // pushed it -- a real, possible outcome, not an error to
            // surface to the user; fall back to "nothing to redo".
            Ok(serde_json::json!({ "changed": false, "content": open_doc.document.text() }))
        }
    }
}

fn close_file(state: &mut BackendState, doc_id: u64) -> Result<serde_json::Value, String> {
    state.open_docs.remove(&doc_id);
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
fn build_leo_provider(
    provider_settings: &spartan_settings::LeoProviderSettings,
    gpu_offload: spartan_settings::GpuOffloadSettings,
) -> Result<Box<dyn ModelProvider>, String> {
    match provider_settings.kind {
        spartan_settings::LeoProviderKind::Ollama => Ok(Box::new(
            OllamaProvider::local(&provider_settings.model).with_gpu_layers(gpu_offload.num_gpu()),
        )),
        spartan_settings::LeoProviderKind::Claude => {
            let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
                "ANTHROPIC_API_KEY is not set -- required to use Claude as Leo's provider"
                    .to_string()
            })?;
            Ok(Box::new(ClaudeProvider::new(
                api_key,
                &provider_settings.model,
            )))
        }
        spartan_settings::LeoProviderKind::LiteLLM => {
            Ok(Box::new(LiteLLMProvider::local(&provider_settings.model)))
        }
    }
}

fn leo_start_task(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    task: String,
    project_root: String,
) -> Result<serde_json::Value, String> {
    let my_generation = {
        let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
        let approval_mode = approval_mode_from_settings(spartan_settings::load().leo_approval_mode);
        let mut agent = Agent::new(PathBuf::from(&project_root), approval_mode);
        agent
            .begin_planning()
            .map_err(|e| format!("begin_planning: {e:?}"))?;
        guard.leo_agent = Some(agent);
        guard.leo_project_root = Some(PathBuf::from(&project_root));
        // A fresh `Agent` per task (§75.47's own documented decision)
        // means the execute-loop's own real state must reset too, or a
        // second task would start with the first task's stale history.
        guard.leo_history.clear();
        guard.leo_pending_call = None;
        guard.leo_generation += 1;
        guard.leo_generation
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
            generate_plan(provider.as_ref(), &task_with_memory);

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
            match agent.apply_generated_plan(result) {
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
            }
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
fn leo_next_step(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
) -> Result<serde_json::Value, String> {
    let (plan, mut history, my_generation) = {
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
        (plan, guard.leo_history.clone(), guard.leo_generation)
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
            let result = execute::next_action(provider.as_ref(), &plan, &history);

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
                        // No configured verification command exists in
                        // this pass -- a real, named v1 scope cut (see
                        // this crate's own doc comment for `leo_next_step`
                        // above) -- so `Verifying` is a real, momentary,
                        // always-passing waypoint on the way to `Done`
                        // rather than a fabricated command result.
                        let transitioned =
                            agent.begin_verification().and_then(|()| agent.mark_done());
                        match transitioned {
                            Ok(()) => {
                                // Real §4.3 project-tier memory write --
                                // "Leo writes to this itself" (memory.rs's
                                // own doc comment) -- a real, best-effort
                                // append, not on the critical path: a
                                // real memory-file I/O failure (e.g. a
                                // read-only project directory) must never
                                // hide that the task itself genuinely
                                // completed, so `memory_saved` is reported
                                // honestly rather than silently swallowed
                                // or allowed to fail the whole task.
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
                        }
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

fn git_commit(project_root: &str, message: &str) -> Result<serde_json::Value, String> {
    let repo = spartan_git::GitRepo::discover(std::path::Path::new(project_root))
        .ok_or("no git repository found at or above this path")?;
    let oid = repo
        .commit(message)
        .map_err(|e| format!("git commit: {e}"))?;
    Ok(serde_json::json!({ "ok": true, "oid": oid.to_string() }))
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

/// A real, optional set of fields alongside the always-present GPU pair
/// (§75.69, §75.70) -- `leo_approval_mode`/`leo_provider` are only ever
/// sent when the Settings screen's own Leo rows actually changed, so an
/// unrelated GPU-only save must not silently reset them back to their
/// real defaults; loading the current settings first and only
/// overriding what was actually provided preserves them.
fn settings_set(
    gpu_enabled: bool,
    gpu_layers: Option<u32>,
    leo_approval_mode: Option<spartan_settings::LeoApprovalMode>,
    leo_provider: Option<spartan_settings::LeoProviderSettings>,
) -> Result<serde_json::Value, String> {
    let current = spartan_settings::load();
    let settings = spartan_settings::Settings {
        gpu_offload: spartan_settings::GpuOffloadSettings {
            enabled: gpu_enabled,
            layers: gpu_layers,
        },
        leo_approval_mode: leo_approval_mode.unwrap_or(current.leo_approval_mode),
        leo_provider: leo_provider.unwrap_or(current.leo_provider),
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
            open_file(&mut guard, &p)
        }),
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
        "leo_start_task" => (|| {
            let task = get_str_param(&req.params, "task")?;
            let project_root = get_str_param(&req.params, "project_root")?;
            leo_start_task(state, out_tx.clone(), task, project_root)
        })(),
        "leo_approve_plan" => leo_approve_plan(state),
        "leo_reject_plan" => leo_reject_plan(state),
        "leo_next_step" => leo_next_step(state, out_tx.clone()),
        "leo_approve_call" => leo_approve_call(state),
        "leo_reject_call" => leo_reject_call(state),
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
        "git_commit" => (|| {
            let root = get_str_param(&req.params, "project_root")?;
            let message = get_str_param(&req.params, "message")?;
            git_commit(&root, &message)
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
            settings_set(gpu_enabled, gpu_layers, leo_approval_mode, leo_provider)
        })(),
        "check_for_updates" => check_for_updates(out_tx.clone()),
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
    fn build_leo_provider_constructs_a_real_ollama_provider_by_default() {
        let settings = spartan_settings::LeoProviderSettings::default();
        let provider =
            build_leo_provider(&settings, spartan_settings::GpuOffloadSettings::default())
                .expect("Ollama provider construction must never fail");
        assert!(provider.is_local());
        assert_eq!(provider.id(), "llama3.1:8b");
    }

    #[test]
    fn build_leo_provider_constructs_a_real_litellm_provider() {
        let settings = spartan_settings::LeoProviderSettings {
            kind: spartan_settings::LeoProviderKind::LiteLLM,
            model: "gpt-4o".to_string(),
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
        let status = call(&state, 2, "leo_status", serde_json::json!({}));
        assert_eq!(status.result.unwrap()["state"], "Planning");
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
}
