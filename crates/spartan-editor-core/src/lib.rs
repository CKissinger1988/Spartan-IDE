//! Real (non-spike) Tier 1 core-engine code. Exposes only the modules with
//! no GPU/window dependency, so `tests/` can exercise real `Document`
//! <-> render-input mapping, viewport windowing logic, and (as of §75.6-§75.8)
//! real live LSP/DAP sessions headlessly (no wgpu device, no window, no
//! display needed) -- the same split `render-spike` (§39.1, §47.9-§47.10)
//! established. Everything GPU/winit-facing (`gpu`, `text`, `cursor`,
//! `input`, `latency`, `fixture`) stays private to the `main.rs` binary
//! target. `lsp`/`lsp_session`/`dap`/`dap_session` have no GPU dependency
//! either (subprocess + thread + JSON only), so they're public here too.
pub mod dap;
pub mod dap_session;
pub mod editor_view;
pub mod language;
pub mod lsp;
pub mod lsp_session;
pub mod viewport;
