//! Real LSP wiring for `spartan-backend` -- the first time either
//! Electron-based shell (`desktop/`, `web/`) has had LSP support at all
//! (it has only ever existed in the reference wgpu shell,
//! `spartan-editor-core`). Built entirely on `spartan-lsp` (a deliberate
//! second promotion of that shell's own already-tested `lsp.rs`/
//! `lsp_session.rs`, not a shared/extracted dependency -- see that crate's
//! own doc comments for the full rationale).
//!
//! Real, named scope for this first increment: diagnostics only. No
//! hover/completion IPC methods exist yet (`spartan_lsp::LspClient` itself
//! already has real `hover`/`completion` methods, just no caller here) --
//! a real, separate, unstarted follow-up, matching how this crate's own
//! git/devcontainer/Leo wiring each started with their own first, narrow
//! increment too.

use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use spartan_languages::LanguageRegistry;
use spartan_lsp::{LspDiagnostic, LspSession, LspUpdate};

use crate::Event;

/// Matches `spartan-editor-core`'s own established 150ms debounce default
/// (§75.6) -- no product reason to diverge.
const DEBOUNCE: Duration = Duration::from_millis(150);

/// `LanguageProfile::id` already matches the real LSP `languageId` string
/// for every Tier 1 language except `"typescript"`, which covers both
/// `.ts`/`.tsx` *and* `.js`/`.jsx` files in this registry's own curated
/// entry (§75.29's own "TypeScript's bundled query layers onto
/// JavaScript's" precedent, reused here for the same underlying reason:
/// one server, `typescript-language-server`, handles both). A `.js`/`.jsx`
/// file genuinely needs the real `"javascript"` languageId, not
/// `"typescript"`, so real servers apply the right per-language diagnostic
/// rules (e.g. not flagging TS-only syntax as invalid).
fn lsp_language_id(path: &Path, profile_id: &str) -> String {
    if profile_id == "typescript" {
        match path.extension().and_then(|e| e.to_str()) {
            Some("js") | Some("jsx") => "javascript".to_string(),
            _ => "typescript".to_string(),
        }
    } else {
        profile_id.to_string()
    }
}

fn diagnostics_event(doc_id: u64, path: &str, diagnostics: &[LspDiagnostic]) -> Event {
    Event {
        event: "lsp_diagnostics".to_string(),
        data: serde_json::json!({ "doc_id": doc_id, "path": path, "diagnostics": diagnostics }),
    }
}

fn error_event(doc_id: u64, path: &str, message: &str) -> Event {
    Event {
        event: "lsp_error".to_string(),
        data: serde_json::json!({ "doc_id": doc_id, "path": path, "message": message }),
    }
}

/// Real, best-effort LSP spawn for a just-opened file. Returns `None` (not
/// an error -- `open_file` itself must still succeed for a language with
/// no configured server, or a file outside any real, marker-file-rooted
/// project) whenever LSP genuinely isn't available for this file:
/// - the detected language profile has no `lsp_command` configured, or
/// - no real project root can be found by walking up from the file for any
///   of that language's marker files (single-file LSP mode is a real,
///   named scope cut -- the same one `spartan-editor-core::language::
///   find_project_root`'s own doc comment already names), or
/// - the real subprocess spawn itself fails (e.g. the configured server
///   binary genuinely isn't installed -- a real, honest, silent skip
///   rather than failing the whole `open_file` call over a missing
///   optional tool).
///
/// On success, spawns one additional real background thread that drains
/// the session's own updates and relays each one as a real backend
/// `Event` (`lsp_diagnostics`/`lsp_error`), keyed by `doc_id` so a UI with
/// multiple open tabs can correlate an update to the right one. Returns
/// the live `Arc<LspSession>` for the caller to store and later call
/// `notify_edit`/`signal_shutdown` on.
pub fn maybe_spawn_lsp(
    doc_id: u64,
    path: &Path,
    initial_text: &str,
    out_tx: Sender<String>,
) -> Option<Arc<LspSession>> {
    let registry = LanguageRegistry::curated_default();
    let profile = registry.profile_for_file(path)?;
    let command = profile.lsp_command.as_ref()?;
    let project_root = spartan_lsp::find_project_root(path, &profile.marker_files)?;
    let language_id = lsp_language_id(path, &profile.id);

    let session = LspSession::spawn(
        command,
        &project_root,
        path,
        &language_id,
        initial_text,
        DEBOUNCE,
    )
    .ok()?;
    let session = Arc::new(session);

    let drain_session = Arc::clone(&session);
    let path_string = path.to_string_lossy().into_owned();
    std::thread::spawn(move || {
        while let Some(update) = drain_session.recv_update() {
            let event = match update {
                LspUpdate::Diagnostics(diags) => diagnostics_event(doc_id, &path_string, &diags),
                LspUpdate::ServerError(msg) => error_event(doc_id, &path_string, &msg),
            };
            if let Ok(line) = serde_json::to_string(&event) {
                if out_tx.send(line).is_err() {
                    return;
                }
            }
        }
    });

    Some(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_language_id_distinguishes_js_from_ts_within_one_profile() {
        assert_eq!(
            lsp_language_id(Path::new("a.ts"), "typescript"),
            "typescript"
        );
        assert_eq!(
            lsp_language_id(Path::new("a.tsx"), "typescript"),
            "typescript"
        );
        assert_eq!(
            lsp_language_id(Path::new("a.js"), "typescript"),
            "javascript"
        );
        assert_eq!(
            lsp_language_id(Path::new("a.jsx"), "typescript"),
            "javascript"
        );
    }

    #[test]
    fn lsp_language_id_passes_through_every_other_real_profile_id() {
        assert_eq!(lsp_language_id(Path::new("a.rs"), "rust"), "rust");
        assert_eq!(lsp_language_id(Path::new("a.py"), "python"), "python");
        assert_eq!(lsp_language_id(Path::new("a.go"), "go"), "go");
    }

    #[test]
    fn maybe_spawn_lsp_returns_none_honestly_for_a_language_with_no_lsp_command() {
        // A real Tier 1 language with a real project marker but (per the
        // curated registry) no configured `lsp_command` -- Rust and Python
        // both have one, so use a synthetic unrecognized extension instead
        // to exercise the "no matching profile at all" branch, the
        // simplest real case of "LSP genuinely isn't available here."
        let dir = std::env::temp_dir().join(format!(
            "spartan-backend-lsp-none-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.unknownext");
        std::fs::write(&file, "hello").unwrap();

        let (tx, _rx) = std::sync::mpsc::channel();
        let result = maybe_spawn_lsp(0, &file, "hello", tx);
        assert!(result.is_none());

        std::fs::remove_dir_all(&dir).ok();
    }
}
