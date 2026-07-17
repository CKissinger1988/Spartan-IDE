//! Real LSP client + session management shared by any Spartan surface that
//! wants live diagnostics without a render-loop tick to drive them --
//! currently `spartan-backend` (the Electron/`desktop/`+`web/` IPC
//! service). See `client.rs`'s and `session.rs`'s own doc comments for the
//! full rationale, including why this is a deliberate second promotion
//! from `spikes/lsp-spike` rather than an extraction of
//! `spartan-editor-core`'s own already-tested copy.

mod client;
mod session;

pub use client::{
    path_to_file_uri, LspClient, DEFAULT_TIMEOUT, INDEXING_TIMEOUT, INITIALIZE_TIMEOUT,
};
pub use session::{LspDiagnostic, LspSession, LspUpdate};

/// Walks up from `file_path`'s parent directories looking for any of
/// `marker_files` (e.g. `Cargo.toml` for Rust) -- the project root an LSP
/// server needs for real, multi-file analysis rather than degraded
/// single-file mode. A real, deliberate duplication (not an import) of
/// `spartan-editor-core::language::find_project_root` -- tiny, pure, and
/// this crate has no other reason to depend on that GPU-coupled crate.
pub fn find_project_root(
    file_path: &std::path::Path,
    marker_files: &[String],
) -> Option<std::path::PathBuf> {
    let start = file_path.parent()?;
    for ancestor in start.ancestors() {
        if marker_files
            .iter()
            .any(|marker| spartan_languages::marker_present_in(ancestor, marker))
        {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn find_project_root_walks_up_to_a_real_marker_file() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-lsp-root-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname=\"x\"").unwrap();
        let file = dir.join("src").join("main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();

        let root = find_project_root(&file, &["Cargo.toml".to_string()]);
        assert_eq!(root, Some(dir.clone()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_project_root_returns_none_with_no_marker_anywhere() {
        let dir = std::env::temp_dir().join(format!(
            "spartan-lsp-root-test-none-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("lonely.rs");
        std::fs::write(&file, "fn main() {}").unwrap();

        // A deliberately distinctive marker name (never a real project
        // marker) so this can never collide with something that genuinely
        // exists somewhere up the real filesystem's ancestry.
        let root = find_project_root(
            &file,
            &["spartan-lsp-test-marker-that-will-never-exist.toml".to_string()],
        );
        assert_eq!(root, None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn path_to_file_uri_handles_a_real_unix_absolute_path() {
        let uri = path_to_file_uri(&PathBuf::from("/tmp/foo/bar.rs"));
        assert_eq!(uri, "file:///tmp/foo/bar.rs");
    }
}
