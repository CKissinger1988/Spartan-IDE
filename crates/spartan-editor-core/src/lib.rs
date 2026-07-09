//! Real (non-spike) Tier 1 core-engine code. Exposes only the modules with
//! no GPU/window dependency, so `tests/` can exercise real `Document`
//! <-> render-input mapping, viewport windowing logic, and (as of
//! §75.6-§75.11) real live LSP/DAP sessions, real build-system integration,
//! and real tree-sitter syntax highlighting headlessly (no wgpu device, no
//! window, no display needed) -- the same split `render-spike` (§39.1,
//! §47.9-§47.10) established. Everything GPU/winit-facing (`gpu`, `text`,
//! `cursor`, `input`, `latency`, `fixture`) stays private to the `main.rs`
//! binary target. `lsp`/`lsp_session`/`dap`/`dap_session`/`build`/
//! `highlight` have no GPU dependency either (`glyphon::Color` is just a
//! plain data type, no device needed to construct one), so they're public
//! here too.
pub mod accessibility;
pub mod agent_panel;
pub mod build;
pub mod command_palette;
pub mod dap;
pub mod dap_session;
pub mod editor_view;
pub mod file_tree;
pub mod git_panel;
pub mod gui_bridge;
pub mod highlight;
pub mod language;
pub mod leo_bridge;
pub mod lsp;
pub mod lsp_session;
pub mod mode_toggle;
pub mod settings_panel;
pub mod tab_bar;
pub mod viewport;
