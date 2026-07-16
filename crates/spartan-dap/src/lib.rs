//! Real DAP client + session management shared by any Spartan surface that
//! wants live debugging without a render-loop tick to drive it --
//! currently `spartan-backend` (the Electron/`desktop/`+`web/` IPC
//! service). See `client.rs`'s and `session.rs`'s own doc comments for the
//! full rationale, including why this is a deliberate second promotion
//! from `spartan-editor-core::{dap, dap_session, build}` rather than an
//! extraction of that already-tested reference-shell code.

mod build;
mod client;
mod session;

pub use build::{build_debug_binary, BuildResult};
pub use client::{DapClient, DEFAULT_TIMEOUT};
pub use session::{DapCommand, DapFrame, DapSession, DapStopped, DapUpdate, DapVariable};
