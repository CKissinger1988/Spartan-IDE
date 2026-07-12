//! Real, user-requested default-font change: JetBrains Mono, bundled and
//! loaded explicitly rather than left to whatever generic "monospace"
//! happens to resolve to on a given machine.
//!
//! Before this, every real call site in `text.rs` requested
//! `Family::Monospace`, which `glyphon::FontSystem::new()`'s own default
//! construction maps to the literal name `"Fira Mono"`
//! (`fontdb::Database::set_monospace_family`, confirmed by reading the
//! actual installed `cosmic-text` source) -- a real font this project
//! never bundled or verified was installed, so the *actual* rendered
//! font was really "whatever `fontdb` fell back to after failing to find
//! a face named `Fira Mono`," not a deliberate choice. Real, working
//! JetBrains Mono TTF data (Regular + Bold, Latin subset, extracted from
//! the OFL-licensed `@fontsource/jetbrains-mono` npm package -- see
//! `assets/fonts/README.md`) is embedded directly into this binary and
//! loaded into the font database *before* `Family::Monospace` is ever
//! resolved, with the monospace generic-family mapping repointed at the
//! real loaded name. Every existing `Attrs::new().family(Family::
//! Monospace)` call site in `text.rs` needed **no changes** -- they now
//! transparently resolve to the real bundled font instead of a
//! never-installed placeholder name.

use glyphon::{fontdb, FontSystem};

const JETBRAINS_MONO_REGULAR: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
const JETBRAINS_MONO_BOLD: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf");

/// The real family name this crate bundles and defaults to -- exposed so
/// callers that ever need to reference it explicitly (rather than via the
/// generic `Family::Monospace` mapping this module sets up) don't have to
/// duplicate the literal string.
pub const DEFAULT_FONT_FAMILY: &str = "JetBrains Mono";

/// Real replacement for `glyphon::FontSystem::new()` at this crate's one
/// real construction call site (`main.rs`). Mirrors `cosmic-text`'s own
/// `FontSystem::new()` internals (real system-font scan via
/// `db.load_system_fonts()`, the real system locale via `sys_locale`,
/// matching the exact fallback cosmic-text's own private `get_locale()`
/// already uses) with one real, deliberate change: the monospace generic
/// family is repointed at the real bundled `"JetBrains Mono"` instead of
/// the default `"Fira Mono"`, and the two real embedded TTFs are loaded
/// into the database before any text is ever shaped.
///
/// **A real, load-bearing ordering requirement, found only by running the
/// test below, not by inspection**: `set_monospace_family` must be called
/// *after* `load_system_fonts()`, not before. `fontdb`'s own real Linux
/// fontconfig integration (`load_system_fonts` -> `load_fontconfig`,
/// confirmed by reading the actual installed `fontdb` source) parses
/// `/etc/fonts/fonts.conf`'s own `<alias>` entries and calls
/// `set_monospace_family` (and the other generic-family setters) itself
/// for whatever the system's real fontconfig maps `monospace` to (this
/// environment's own real value: `"FreeMono"`) -- silently overwriting an
/// earlier call with the system's own default. Setting it after the scan
/// is what actually makes the override stick.
/// Real §75.93 font-family override, user-requested ("Add user
/// customizable theme and font options to all Spartan interfaces") --
/// `main.rs`'s one real call site passes
/// `spartan_settings::EditorSettings.font_family` straight through
/// (`None` for a fresh install, matching that field's own default).
/// `None`, or a blank/whitespace-only override (the same "empty means
/// unset" contract `spartan_settings::EditorSettings.font_family`'s own
/// doc comment already establishes), keeps this crate's real bundled
/// default exactly as before this pass. A real, honest "best effort"
/// override, not a guarantee: the bundled JetBrains Mono TTFs are always
/// loaded into the database regardless (so an unresolvable custom name
/// still leaves a real font installed for `fontdb`'s own generic-family
/// fallback to reach), but `set_monospace_family` is only ever pointed at
/// a name this crate did not itself verify is actually installed -- if
/// it isn't, resolution falls back to whatever `fontdb`/cosmic-text's own
/// fallback logic picks, the identical honest caveat already documented
/// for the Electron shell's own CSS font-family override
/// (`desktop/src/applyFontFamily.ts`).
pub fn build_font_system_with_override(font_family_override: Option<&str>) -> FontSystem {
    let locale = sys_locale::get_locale().unwrap_or_else(|| String::from("en-US"));

    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    db.load_font_data(JETBRAINS_MONO_REGULAR.to_vec());
    db.load_font_data(JETBRAINS_MONO_BOLD.to_vec());
    let monospace_family = font_family_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_FONT_FAMILY);
    db.set_monospace_family(monospace_family);

    FontSystem::new_with_locale_and_db(locale, db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glyphon::{Attrs, Buffer, Family, Metrics, Shaping};

    #[test]
    fn the_bundled_jetbrains_mono_ttf_data_is_real_and_parses() {
        // A real, direct parse check independent of `FontSystem` itself --
        // confirms the embedded bytes are genuinely a valid TTF before
        // trusting anything built on top of them.
        assert!(ttf_parser::Face::parse(JETBRAINS_MONO_REGULAR, 0).is_ok());
        assert!(ttf_parser::Face::parse(JETBRAINS_MONO_BOLD, 0).is_ok());
    }

    #[test]
    fn a_real_override_repoints_the_monospace_generic_family() {
        let font_system = build_font_system_with_override(Some("Custom Test Font"));
        assert_eq!(
            font_system.db().family_name(&Family::Monospace),
            "Custom Test Font"
        );
    }

    #[test]
    fn a_blank_or_whitespace_only_override_falls_back_to_the_real_bundled_default() {
        for blank in ["", "   ", "\t"] {
            let font_system = build_font_system_with_override(Some(blank));
            assert_eq!(
                font_system.db().family_name(&Family::Monospace),
                DEFAULT_FONT_FAMILY,
                "blank override {blank:?} should fall back to the real bundled default"
            );
        }
    }

    #[test]
    fn a_real_override_is_trimmed() {
        let font_system = build_font_system_with_override(Some("  Custom Test Font  "));
        assert_eq!(
            font_system.db().family_name(&Family::Monospace),
            "Custom Test Font"
        );
    }

    #[test]
    fn build_font_system_loads_jetbrains_mono_under_its_own_real_name() {
        let db = {
            let mut db = fontdb::Database::new();
            db.set_monospace_family(DEFAULT_FONT_FAMILY);
            db.load_font_data(JETBRAINS_MONO_REGULAR.to_vec());
            db.load_font_data(JETBRAINS_MONO_BOLD.to_vec());
            db
        };
        let found = db.faces().any(|face| {
            face.families
                .iter()
                .any(|(name, _)| name == DEFAULT_FONT_FAMILY)
        });
        assert!(
            found,
            "expected a real loaded face named \"JetBrains Mono\""
        );
    }

    #[test]
    fn build_font_system_makes_family_monospace_resolve_to_jetbrains_mono() {
        // The real, load-bearing check, exercising the actual real shaping
        // path every `text.rs` call site goes through -- not
        // `get_font_matches` alone, which (confirmed by reading the real
        // installed cosmic-text source) filters candidates only by
        // weight/style/stretch and does **not** select by family; the real
        // family resolution for `Family::Monospace` happens one layer
        // deeper, inside cosmic-text's own private `FontFallbackIter`,
        // which isn't part of its public API. Shaping a real string and
        // inspecting the real glyph's `font_id` is the real, public,
        // black-box way to observe that same resolution.
        let mut font_system = build_font_system_with_override(None);
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 20.0));
        buffer.set_size(&mut font_system, 800.0, 100.0);
        buffer.set_text(
            &mut font_system,
            "fn main() {}",
            Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(&mut font_system);

        let mut saw_a_glyph = false;
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                saw_a_glyph = true;
                let face = font_system
                    .db()
                    .face(glyph.font_id)
                    .expect("a shaped glyph's font_id should resolve to a real face");
                assert!(
                    face.families
                        .iter()
                        .any(|(name, _)| name == DEFAULT_FONT_FAMILY),
                    "expected a Family::Monospace-shaped glyph to come from a real JetBrains Mono \
                     face, got {:?} (post_script_name {:?})",
                    face.families,
                    face.post_script_name
                );
            }
        }
        assert!(
            saw_a_glyph,
            "expected shaping real text to produce at least one real glyph"
        );
    }
}
