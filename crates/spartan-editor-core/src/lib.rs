//! Real (non-spike) Tier 1 core-engine code. Exposes only the modules with
//! no GPU/window dependency, so `tests/` can exercise real `Document`
//! <-> render-input mapping and viewport windowing logic headlessly (no
//! wgpu device, no window, no display needed) -- the same split
//! `render-spike` (§39.1, §47.9-§47.10) established. Everything
//! GPU/winit-facing (`gpu`, `text`, `cursor`, `input`, `latency`,
//! `fixture`) stays private to the `main.rs` binary target.
pub mod editor_view;
pub mod language;
pub mod viewport;
