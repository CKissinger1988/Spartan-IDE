//! Real IPC backend service for the new Electron desktop shell
//! (`desktop/`, user-requested pivot away from the wgpu-native
//! `spartan-editor-core` UI). Deliberately thin: all the real editing
//! logic already lives in `spartan-buffer::Document` (branching undo
//! tree, char-indexed edits) -- this crate only adds a real newline-
//! delimited JSON-RPC-style protocol on top of it so a Node/Electron
//! process can drive it as a child process over stdin/stdout, the same
//! "spawn a real subprocess, talk JSON over a pipe" shape this
//! workspace already uses for LSP/DAP adapters, just in the opposite
//! direction (this crate is the server, not the client).
//!
//! `spartan-editor-core` (the original wgpu shell) is not being deleted
//! -- it stays as the real, tested, working proof that the underlying
//! Rust core (this crate's own dependency, plus LSP/DAP/tree-sitter/Leo/
//! git in the sibling crates) is sound. This crate is the first real
//! step of exposing that same core to a *different* UI layer.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use spartan_buffer::Document;

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

struct OpenDoc {
    path: PathBuf,
    document: Document,
}

/// Real, in-memory session state -- every open document's real
/// `spartan_buffer::Document` (with its own real branching undo tree),
/// keyed by a real sequential id the renderer refers to for every
/// subsequent edit/save/undo/close call. Not `Send`/`Sync`-guarded
/// deliberately: `main.rs`'s stdio loop is single-threaded, one request
/// processed at a time, matching the real protocol's own one-request-
/// one-response contract (no concurrent requests to race).
#[derive(Default)]
pub struct BackendState {
    open_docs: HashMap<u64, OpenDoc>,
    next_doc_id: u64,
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
/// calls per real line of input, kept pure/synchronous and fully
/// testable without any real stdin/stdout, matching this crate's own
/// doc-comment discipline of separating real I/O from real logic.
pub fn handle_request(state: &mut BackendState, req: Request) -> Response {
    let result = match req.method.as_str() {
        "list_dir" => get_str_param(&req.params, "path").and_then(|p| list_dir(&p)),
        "open_file" => get_str_param(&req.params, "path").and_then(|p| open_file(state, &p)),
        "edit" => (|| {
            let doc_id = get_u64_param(&req.params, "doc_id")?;
            let start_char = get_u64_param(&req.params, "start_char")? as usize;
            let end_char = get_u64_param(&req.params, "end_char")? as usize;
            let text = get_str_param(&req.params, "text").unwrap_or_default();
            edit(state, doc_id, start_char, end_char, &text)
        })(),
        "save_file" => get_u64_param(&req.params, "doc_id").and_then(|id| save_file(state, id)),
        "undo" => get_u64_param(&req.params, "doc_id").and_then(|id| undo(state, id)),
        "close_file" => get_u64_param(&req.params, "doc_id").and_then(|id| close_file(state, id)),
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

    fn req(id: u64, method: &str, params: serde_json::Value) -> Request {
        Request {
            id,
            method: method.to_string(),
            params,
        }
    }

    #[test]
    fn list_dir_lists_a_real_temp_directory_dirs_first() {
        let dir = std::env::temp_dir().join(format!("spartan-backend-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("zsubdir")).unwrap();
        std::fs::write(dir.join("afile.txt"), "hi").unwrap();
        let mut state = BackendState::new();
        let resp = handle_request(
            &mut state,
            req(
                1,
                "list_dir",
                serde_json::json!({ "path": dir.to_string_lossy() }),
            ),
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
        let mut state = BackendState::new();
        let resp = handle_request(
            &mut state,
            req(
                1,
                "list_dir",
                serde_json::json!({ "path": "/definitely/not/a/real/path/xyz" }),
            ),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
    }

    #[test]
    fn open_edit_save_round_trips_through_a_real_file() {
        let file =
            std::env::temp_dir().join(format!("spartan-backend-test-{}.txt", std::process::id()));
        std::fs::write(&file, "hello world").unwrap();
        let mut state = BackendState::new();

        let open_resp = handle_request(
            &mut state,
            req(
                1,
                "open_file",
                serde_json::json!({ "path": file.to_string_lossy() }),
            ),
        );
        let open_result = open_resp.result.unwrap();
        assert_eq!(open_result["content"], "hello world");
        let doc_id = open_result["doc_id"].as_u64().unwrap();

        // Real insert: "hello " -> "hello, " (insert a comma at char 5).
        let edit_resp = handle_request(
            &mut state,
            req(
                2,
                "edit",
                serde_json::json!({ "doc_id": doc_id, "start_char": 5, "end_char": 5, "text": "," }),
            ),
        );
        assert!(edit_resp.error.is_none());

        let save_resp = handle_request(
            &mut state,
            req(3, "save_file", serde_json::json!({ "doc_id": doc_id })),
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
        let mut state = BackendState::new();
        let open_result = handle_request(
            &mut state,
            req(
                1,
                "open_file",
                serde_json::json!({ "path": file.to_string_lossy() }),
            ),
        )
        .result
        .unwrap();
        let doc_id = open_result["doc_id"].as_u64().unwrap();

        // Delete "hello " (chars 0..6).
        handle_request(
            &mut state,
            req(
                2,
                "edit",
                serde_json::json!({ "doc_id": doc_id, "start_char": 0, "end_char": 6, "text": "" }),
            ),
        );
        handle_request(
            &mut state,
            req(3, "save_file", serde_json::json!({ "doc_id": doc_id })),
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
        let mut state = BackendState::new();
        let doc_id = handle_request(
            &mut state,
            req(
                1,
                "open_file",
                serde_json::json!({ "path": file.to_string_lossy() }),
            ),
        )
        .result
        .unwrap()["doc_id"]
            .as_u64()
            .unwrap();

        handle_request(
            &mut state,
            req(
                2,
                "edit",
                serde_json::json!({ "doc_id": doc_id, "start_char": 3, "end_char": 3, "text": "d" }),
            ),
        );
        let undo_resp = handle_request(
            &mut state,
            req(3, "undo", serde_json::json!({ "doc_id": doc_id })),
        );
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
        let mut state = BackendState::new();
        let doc_id = handle_request(
            &mut state,
            req(
                1,
                "open_file",
                serde_json::json!({ "path": file.to_string_lossy() }),
            ),
        )
        .result
        .unwrap()["doc_id"]
            .as_u64()
            .unwrap();
        handle_request(
            &mut state,
            req(2, "close_file", serde_json::json!({ "doc_id": doc_id })),
        );
        let resp = handle_request(
            &mut state,
            req(3, "save_file", serde_json::json!({ "doc_id": doc_id })),
        );
        assert!(resp.error.is_some());
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn an_unknown_method_errors_honestly_instead_of_panicking() {
        let mut state = BackendState::new();
        let resp = handle_request(
            &mut state,
            req(1, "not_a_real_method", serde_json::json!({})),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("unknown method"));
    }

    #[test]
    fn editing_a_real_unopened_doc_id_errors_honestly() {
        let mut state = BackendState::new();
        let resp = handle_request(
            &mut state,
            req(
                1,
                "edit",
                serde_json::json!({ "doc_id": 999, "start_char": 0, "end_char": 0, "text": "x" }),
            ),
        );
        assert!(resp.error.is_some());
    }
}
