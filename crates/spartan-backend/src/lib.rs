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
use spartan_leo::plan::{generate_plan, ImplementationPlan, PlanError};
use spartan_model::OllamaProvider;

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
    let changed = open_doc.document.undo();
    Ok(serde_json::json!({ "changed": changed, "content": open_doc.document.text() }))
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

/// Real Leo status snapshot -- the renderer calls this on mount to
/// rehydrate a persistent chat panel's state, since (unlike a single
/// full-screen mode) it may be mounted before or after a task is
/// already in flight.
fn leo_status(state: &BackendState) -> Result<serde_json::Value, String> {
    match &state.leo_agent {
        Some(agent) => Ok(serde_json::json!({
            "state": agent_state_name(agent),
            "plan": agent.plan().map(plan_json),
        })),
        None => Ok(serde_json::json!({ "state": "Idle", "plan": null })),
    }
}

/// Real `Idle -> Planning` transition plus a real, spawned background
/// thread that makes the actual blocking model call -- mirroring
/// `spartan-editor-core::leo_bridge::spawn_plan_request` exactly, moved
/// to this crate's own `Arc<Mutex<BackendState>>` + `Event`-over-stdout
/// shape instead of an in-process `mpsc` receiver a render loop polls.
fn leo_start_task(
    state: &Arc<Mutex<BackendState>>,
    out_tx: Sender<String>,
    task: String,
    project_root: String,
) -> Result<serde_json::Value, String> {
    {
        let mut guard = state.lock().map_err(|_| "backend state poisoned")?;
        let mut agent = Agent::new(PathBuf::from(&project_root), ApprovalMode::ManualEveryStep);
        agent
            .begin_planning()
            .map_err(|e| format!("begin_planning: {e:?}"))?;
        guard.leo_agent = Some(agent);
        guard.leo_project_root = Some(PathBuf::from(&project_root));
    }

    let state = Arc::clone(state);
    thread::spawn(move || {
        let gpu_offload = spartan_settings::load().gpu_offload;
        let provider = OllamaProvider::local(LEO_MODEL).with_gpu_layers(gpu_offload.num_gpu());
        let result: Result<ImplementationPlan, PlanError> = generate_plan(&provider, &task);

        let event = {
            let Ok(mut guard) = state.lock() else {
                return;
            };
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
}
