use spartan_languages::{LanguageProfile, LanguageRegistry};
use std::path::{Path, PathBuf};

/// The first real combination of `spartan-buffer` (via `editor_view`),
/// GPU rendering (via `text`/`gpu`/`cursor`, this crate's binary target),
/// and `spartan-languages` -- no spike has driven all three from one real
/// file open before (`render-spike` never touched `spartan-languages`;
/// `spartan-languages`'s own tests never touch rendering). Tree-sitter
/// stays unwired here; LSP is wired for the first time in `lsp_session.rs`
/// (§75.6), which is what `find_project_root` below exists to support.
pub fn detect_language_for_file(path: &Path) -> Option<LanguageProfile> {
    LanguageRegistry::curated_default()
        .profile_for_file(path)
        .cloned()
}

/// Walks up from `file_path`'s parent directories looking for any of the
/// already-detected profile's own `marker_files` (e.g. `Cargo.toml` for
/// Rust) -- the project root an LSP server needs to run real, multi-file
/// analysis rather than degraded single-file mode. Deliberately reuses the
/// profile data `spartan-languages` already provides instead of
/// duplicating its marker-file matching logic (that crate walks *down* from
/// a known root to detect a project's languages; this walks *up* from a
/// single file to find that root in the first place -- a complementary,
/// not overlapping, direction). As of §75.51, also reuses that crate's own
/// `marker_present_in` for the actual per-directory check, so a glob-style
/// marker (`"*.csproj"` -- C# has no single fixed project-file name the
/// way `Cargo.toml`/`package.json` do) resolves identically here and in
/// `detect_project_languages`, one real matching rule, not two.
///
/// Returns `None` if no ancestor directory contains any marker file --
/// callers should fall back to the file's own parent directory in that
/// case and treat it as a named, real limitation (single-file mode), the
/// same category of carve-out as this crate's `--synthetic:N` case.
pub fn find_project_root(file_path: &Path, marker_files: &[String]) -> Option<PathBuf> {
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
