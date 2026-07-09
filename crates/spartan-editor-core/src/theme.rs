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

use glyphon::Color as TextColor;

/// `#09090B` (the prototype's own `bg` token) -- the real editor content
/// area's background. Previously this crate's clear color was
/// `srgb(0.08, 0.08, 0.09)`, which -- not by design -- lands almost
/// exactly on the prototype's *lighter* `s1` tier (`#141416`) applied
/// everywhere, rather than the darkest `bg` tier reserved for the base
/// layer underneath real, distinct surface panels.
pub const BG_LINEAR: wgpu::Color = wgpu::Color {
    r: 0.002732,
    g: 0.002732,
    b: 0.003347,
    a: 1.0,
};

/// `#18181B` (the prototype's own `s2` token) -- the sidebar's and tab
/// bar's real background, a distinct, slightly lighter surface sitting
/// visually "above" the darker editor content area underneath it. This is
/// the real mechanism behind Antigravity's own layered-panel look, and
/// was entirely absent from this renderer before this pass (every region
/// shared one flat clear color, with no panel distinction at all).
pub const SURFACE: [f32; 4] = [0.009134, 0.009134, 0.010960, 1.0];

/// `#27272A` (the prototype's own `border` token) -- a real, thin
/// separator line between the sidebar and the editor, and beneath the tab
/// bar. The other half of the layered-panel look; also entirely absent
/// before this pass.
pub const BORDER: [f32; 4] = [0.020289, 0.020289, 0.023153, 1.0];

/// Real border-line thickness in pixels -- thin enough to read as a
/// hairline separator, matching a conventional IDE's own panel dividers,
/// not a thick decorative bar (the same "declutter, don't add chrome"
/// discipline §50.3/§36.4.10 already established).
pub const BORDER_WIDTH_PX: f32 = 1.0;

/// `#E9E7E4` -- this renderer's existing real default text color,
/// re-exported here so every real color token has one real home instead
/// of staying scattered across `text.rs`/`selection.rs`/`cursor.wgsl`.
pub const TEXT: TextColor = TextColor::rgb(0xE9, 0xE7, 0xE4);

/// `#84838A` -- the prototype's own real, researched `textDim` token
/// (§50.3: "textDim's luminance is raised... since it's the color most
/// often used for body text"), matched exactly. The pre-existing
/// `text.rs` mode-toggle constant was already close (`#8A8A8E`) but not
/// the exact researched value -- fixed here rather than left as a
/// near-miss.
pub const TEXT_DIM: TextColor = TextColor::rgb(0x84, 0x83, 0x8A);
