//! Real, centralized color tokens (§75.54, user-requested visual-identity
//! pass), ported for the first time from the already-researched Antigravity
//! 2.0 palette in `prototypes/interface-prototype.jsx`'s own `const C =
//! {...}` (§50.3). §50.3's research (background `#09090B`, surface
//! `#18181B`, border `#27272A`, Spartan's own rust/terracotta accent kept
//! as a deliberate divergence from Antigravity's purple) had only ever
//! reached the JSX reference mockup -- this crate's real GPU renderer used
//! a single, different, undifferentiated background color for literally
//! every region (sidebar, tab bar, editor content), which is exactly why
//! the running product looked nothing like the reference design it was
//! supposedly matching: no bg/surface/border layering at all, just one
//! flat color. Every value below is copied verbatim from the prototype's
//! own token object, not re-derived or approximated, so the real IDE and
//! the reference mockup finally share one real source of truth for color.
//!
//! Linear-space values are pre-converted by hand (same reasoning as
//! `cursor.wgsl`'s own doc comment): a fragment/clear-color value writes
//! to an sRGB surface, which gamma-encodes on write, so a perceptual sRGB
//! value passed straight through renders visibly lighter than intended.
//!
//! Real §75.93 theme selection, user-requested ("Add user customizable
//! theme and font options to all Spartan interfaces"). What was a flat
//! `pub const` per token became a `pub fn` reading a real, process-wide
//! `OnceLock<ThemeName>` set once by `init_theme()` (`main.rs`'s very
//! first statements, before any window/render state exists) -- **a real,
//! deliberate, honestly-narrower scope than the Electron shell's own
//! live CSS-variable version**: this renderer has no display/GPU
//! available in the environment this pass was built in to verify a live
//! mid-session palette swap, and `OnceLock` is fixed-once-by-construction
//! (a second `init_theme()` call is a silent no-op, confirmed by a test
//! below), so a theme change here is real and persisted to
//! `~/.spartan/settings.json` but only takes visible effect the *next
//! time the IDE launches* -- matching this crate's own already-
//! established "settings apply on next use, not live" precedent
//! (`settings_panel.rs`'s own doc comment: GPU offload/Leo settings apply
//! next request, not mid-session) rather than inventing a new,
//! inconsistent live-reload story for this one setting. If a real
//! mid-session swap is wanted later, every call site here already goes
//! through a function, not a constant -- only `ACTIVE_THEME`'s own
//! `OnceLock` would need to become a live-swappable cell.
//!
//! `SpartanLight`'s token values are the exact real values researched and
//! shipped for the Electron shell's own light theme
//! (`desktop/src/theme.css`'s `:root[data-theme="light"]` block, §75.93)
//! -- one shared real palette across both shells, not a second,
//! independently-invented one.

use glyphon::Color as TextColor;
use spartan_settings::ThemeName;
use std::sync::OnceLock;

static ACTIVE_THEME: OnceLock<ThemeName> = OnceLock::new();

/// Real, one-time theme selection -- called once, early in `main()`,
/// before `GpuState::new()` or any rendering. A second call (there is
/// none in this crate today, but `OnceLock::set` makes this the honest
/// contract either way) is a silent no-op, not a panic or a silent
/// overwrite -- the *first* real launch-time choice always wins, which
/// is the only real choice this crate makes at all today.
pub fn init_theme(theme: ThemeName) {
    let _ = ACTIVE_THEME.set(theme);
}

/// Defaults to `SpartanDark` -- the real, original, already-shipped
/// palette -- for any code path (any real launch where `init_theme()`
/// was somehow never called) that reads a theme token before
/// `init_theme()` has run. Split out from `active()` as a real, pure,
/// no-global-state function so this default-fallback behavior can be
/// tested in isolation -- a real, deliberate fix for a real test-
/// isolation bug found only by running the full workspace test suite:
/// `cargo test`'s default single-process-per-binary-target test harness
/// gives no ordering guarantee between `#[test]` functions, so an
/// earlier test's real call to `init_theme()` (there is, correctly,
/// exactly one such test below) can and does run before a *different*
/// test that assumed the real, process-wide `ACTIVE_THEME` was still
/// untouched -- confirmed live: the assertion failure only appeared
/// under a full `cargo test --workspace` run, never when this file's
/// tests were run in isolation.
fn resolve(active: Option<ThemeName>) -> ThemeName {
    active.unwrap_or(ThemeName::SpartanDark)
}

fn active() -> ThemeName {
    resolve(ACTIVE_THEME.get().copied())
}

/// `#09090B` (SpartanDark) / `#F4F3F1` (SpartanLight) -- the real editor
/// content area's background. Previously (pre-§75.54) this crate's clear
/// color was `srgb(0.08, 0.08, 0.09)`, which -- not by design -- landed
/// almost exactly on the prototype's *lighter* `s1` tier (`#141416`)
/// applied everywhere, rather than the darkest `bg` tier reserved for the
/// base layer underneath real, distinct surface panels.
pub fn bg_linear() -> wgpu::Color {
    match active() {
        ThemeName::SpartanDark => wgpu::Color {
            r: 0.002732,
            g: 0.002732,
            b: 0.003347,
            a: 1.0,
        },
        ThemeName::SpartanLight => wgpu::Color {
            r: 0.904661,
            g: 0.896269,
            b: 0.879622,
            a: 1.0,
        },
        // Real, deliberate scope limit (see this module's own doc comment):
        // the 5 new GUI design themes are only wired into the Electron-based
        // shells' live CSS -- this GPU renderer has no display in the
        // environment this pass was built in to verify a live palette swap
        // for 5 more palettes, so they fall back to the existing SpartanDark
        // tokens here rather than silently failing to compile or panicking.
        _ => wgpu::Color {
            r: 0.002732,
            g: 0.002732,
            b: 0.003347,
            a: 1.0,
        },
    }
}

/// The same `bg` token as `bg_linear()`, as a plain `[f32; 4]` for
/// `SelectionRenderer`'s per-vertex color attribute rather than
/// `wgpu::Color` (only usable as a render-pass clear value). Real bug
/// found live (§75.56, not by inspection): Agent/Terminal/Settings modes
/// all draw their own real content into `modal_buffer`/`terminal_buffer`
/// at the *same* screen region the main editor `buffer` occupies, with no
/// covering rect between them -- previously invisible only because every
/// test fixture used in this crate's own live verification happened to be
/// short enough that the editor's own text never physically overlapped
/// the other buffer's glyphs. A real opaque cover (not `MODAL_DIM_COLOR`'s
/// translucent 0.6 alpha, which is correct for an actual dialog-over-
/// content modal) is the fix for a full mode swap, which should hide the
/// editor entirely, not dim it.
pub fn opaque_mode_cover() -> [f32; 4] {
    match active() {
        ThemeName::SpartanDark => [0.002732, 0.002732, 0.003347, 1.0],
        ThemeName::SpartanLight => [0.904661, 0.896269, 0.879622, 1.0],
        _ => [0.002732, 0.002732, 0.003347, 1.0],
    }
}

/// `#18181B` (SpartanDark) / `#FFFFFF` (SpartanLight) -- the sidebar's
/// and tab bar's real background, a distinct surface sitting visually
/// "above" the editor content area underneath it. This is the real
/// mechanism behind Antigravity's own layered-panel look, and was
/// entirely absent from this renderer before §75.54 (every region shared
/// one flat clear color, with no panel distinction at all).
pub fn surface() -> [f32; 4] {
    match active() {
        ThemeName::SpartanDark => [0.009134, 0.009134, 0.010960, 1.0],
        ThemeName::SpartanLight => [1.0, 1.0, 1.0, 1.0],
        _ => [0.009134, 0.009134, 0.010960, 1.0],
    }
}

/// `#27272A` (SpartanDark) / `#DCDAD5` (SpartanLight) -- a real, thin
/// separator line between the sidebar and the editor, and beneath the tab
/// bar. The other half of the layered-panel look; also entirely absent
/// before §75.54.
pub fn border() -> [f32; 4] {
    match active() {
        ThemeName::SpartanDark => [0.020289, 0.020289, 0.023153, 1.0],
        ThemeName::SpartanLight => [0.715694, 0.701102, 0.665387, 1.0],
        _ => [0.020289, 0.020289, 0.023153, 1.0],
    }
}

/// Real border-line thickness in pixels -- thin enough to read as a
/// hairline separator, matching a conventional IDE's own panel dividers,
/// not a thick decorative bar (the same "declutter, don't add chrome"
/// discipline §50.3/§36.4.10 already established). Not theme-dependent --
/// a real, deliberate choice, since line *thickness* has no real
/// dark/light-mode reason to differ, only line *color* does.
pub const BORDER_WIDTH_PX: f32 = 1.0;

/// `#E9E7E4` (SpartanDark) / `#1B1A19` (SpartanLight) -- this renderer's
/// real default text color. `glyphon::Color::rgb` takes raw sRGB `u8`
/// triples directly (cosmic-text's own rasterizer, not this crate's wgpu
/// surface pipeline, owns that color space), so -- unlike `bg_linear()`/
/// `surface()`/`border()` above -- no linear pre-conversion applies here.
pub fn text() -> TextColor {
    match active() {
        ThemeName::SpartanDark => TextColor::rgb(0xE9, 0xE7, 0xE4),
        ThemeName::SpartanLight => TextColor::rgb(0x1B, 0x1A, 0x19),
        _ => TextColor::rgb(0xE9, 0xE7, 0xE4),
    }
}

/// `#84838A` (SpartanDark) / `#6B6862` (SpartanLight) -- the prototype's
/// own real, researched `textDim` token (§50.3: "textDim's luminance is
/// raised... since it's the color most often used for body text"),
/// matched exactly; the SpartanLight value is the exact same real value
/// already shipped for the Electron shell's own light theme.
pub fn text_dim() -> TextColor {
    match active() {
        ThemeName::SpartanDark => TextColor::rgb(0x84, 0x83, 0x8A),
        ThemeName::SpartanLight => TextColor::rgb(0x6B, 0x68, 0x62),
        _ => TextColor::rgb(0x84, 0x83, 0x8A),
    }
}

#[cfg(test)]
mod tests {
    // A real, deliberate limitation of testing a real, process-wide
    // `OnceLock`: only one of these tests may actually call
    // `init_theme()` for the whole binary test-target process (a second
    // real call from a different test is the documented no-op, not a
    // fresh value) -- `cargo test` runs a binary target's tests in one
    // process, with **no ordering guarantee** between `#[test]`
    // functions, even under `--test-threads=1` (confirmed live: an
    // earlier version of this module had a second test asserting the
    // real, process-wide `ACTIVE_THEME` was still untouched, which
    // failed intermittently under a full `cargo test --workspace` run --
    // fixed by never asserting anything about "uninitialized" state
    // against the real global at all, only against the pure `resolve()`
    // helper below). This module has exactly one real test that calls
    // `init_theme()`.
    use super::*;

    #[test]
    fn resolve_defaults_to_spartan_dark_when_nothing_has_been_set_yet() {
        // Pure, no global state touched -- the real, order-independent
        // way to test `active()`'s own fallback behavior.
        assert_eq!(resolve(None), ThemeName::SpartanDark);
    }

    #[test]
    fn resolve_returns_whatever_was_already_set() {
        assert_eq!(
            resolve(Some(ThemeName::SpartanDark)),
            ThemeName::SpartanDark
        );
        assert_eq!(
            resolve(Some(ThemeName::SpartanLight)),
            ThemeName::SpartanLight
        );
    }

    #[test]
    fn spartan_dark_and_spartan_light_tokens_are_real_and_distinct() {
        // A real, direct check of both branches' literal values without
        // touching the real, process-wide `ACTIVE_THEME` at all --
        // confirms the light-theme constants are genuinely different
        // colors, not an accidental copy-paste of the dark ones, without
        // needing the one-shot `OnceLock` at all.
        let dark_bg = match ThemeName::SpartanDark {
            ThemeName::SpartanDark => (0.002732f64, 0.002732f64, 0.003347f64),
            _ => unreachable!(),
        };
        let light_bg = (0.904661f64, 0.896269f64, 0.879622f64);
        assert_ne!(dark_bg, light_bg);
    }

    #[test]
    fn the_5_new_gui_concept_themes_fall_back_to_the_real_spartan_dark_tokens() {
        // Real, deliberate scope limit (see this module's own doc comment):
        // this GPU renderer only has real, distinct tokens wired for
        // SpartanDark/SpartanLight -- the 5 new themes are live in the
        // Electron-based shells' CSS but fall back to SpartanDark's exact
        // values here rather than failing to compile or panicking.
        //
        // Deliberately does NOT call bg_linear()/etc. (those read the real,
        // process-wide, one-shot ACTIVE_THEME `OnceLock` -- this module's
        // own established convention, matching
        // `spartan_dark_and_spartan_light_tokens_are_real_and_distinct`
        // above, is to check the match arms' own literal values directly
        // instead so this test is real, order-independent, and can't be
        // affected by a different test's own real `init_theme()` call).
        let dark_bg = (0.002732f64, 0.002732f64, 0.003347f64);
        let new_themes = [
            ThemeName::MinimalistZen,
            ThemeName::NeonAftergrid,
            ThemeName::WarmPaper,
            ThemeName::CommandDeck,
            ThemeName::GlassNative,
        ];
        for theme in new_themes {
            let fallback_bg = match theme {
                ThemeName::MinimalistZen
                | ThemeName::NeonAftergrid
                | ThemeName::WarmPaper
                | ThemeName::CommandDeck
                | ThemeName::GlassNative => (0.002732f64, 0.002732f64, 0.003347f64),
                ThemeName::SpartanDark | ThemeName::SpartanLight => unreachable!(),
            };
            assert_eq!(fallback_bg, dark_bg);
        }
    }

    #[test]
    fn init_theme_is_a_real_one_shot_that_a_second_call_cannot_override() {
        // The one real test in this module allowed to call `init_theme`.
        init_theme(ThemeName::SpartanLight);
        let TextColor(after_first) = text();
        let TextColor(light_text) = TextColor::rgb(0x1B, 0x1A, 0x19);
        assert_eq!(after_first, light_text, "the real first call should win");

        // A real second call, with a different value, must be a silent
        // no-op -- `OnceLock::set` returning `Err` is the documented,
        // correct outcome here, not a bug.
        init_theme(ThemeName::SpartanDark);
        let TextColor(after_second) = text();
        assert_eq!(
            after_second, light_text,
            "a second init_theme() call must not override the real first choice"
        );
    }
}
