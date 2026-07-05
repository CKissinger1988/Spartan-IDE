//! Exposes only the modules with no GPU/window dependency, so
//! `tests/editor_view_maps_document_state.rs` can exercise real
//! `Document` <-> render-input mapping logic headlessly (no wgpu device,
//! no window, no display needed). Everything GPU/winit-facing (`gpu`,
//! `text`, `cursor`, `input`, `latency`, `fixture`) stays private to the
//! `main.rs` binary target, which links against this library crate for
//! `editor_view` alone.
pub mod editor_view;
