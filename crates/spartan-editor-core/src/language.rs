use spartan_languages::{LanguageProfile, LanguageRegistry};
use std::path::Path;

/// The first real combination of `spartan-buffer` (via `editor_view`),
/// GPU rendering (via `text`/`gpu`/`cursor`, this crate's binary target),
/// and `spartan-languages` -- no spike has driven all three from one real
/// file open before (`render-spike` never touched `spartan-languages`;
/// `spartan-languages`'s own tests never touch rendering). Tree-sitter and
/// LSP/DAP themselves stay unwired here -- this only proves the *lookup*
/// step, per §75.3's own naming of that wiring as a real, separate,
/// not-yet-done next step.
pub fn detect_language_for_file(path: &Path) -> Option<LanguageProfile> {
    LanguageRegistry::curated_default()
        .profile_for_file(path)
        .cloned()
}
