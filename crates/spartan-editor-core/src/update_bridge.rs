//! Real background-thread bridge to `spartan-updater`'s live GitHub check
//! (§75.49, user-requested) -- same spawn-thread/channel/non-blocking-poll
//! shape `leo_bridge.rs`/`gui_bridge.rs` already established, applied here
//! to a real HTTPS call instead of a local model/subprocess. A real
//! network round trip must never run on the render thread.

use spartan_updater::{check_for_updates, UpdateCheckError, UpdateCheckResult};
use std::sync::mpsc;
use std::thread;

/// This project's own real repository and default branch -- matching the
/// exact values `spartan-updater`'s own live integration test already
/// uses.
pub const REPO: &str = "ckissinger1988/spartan-ide";
pub const BRANCH: &str = "main";

pub fn spawn_update_check() -> mpsc::Receiver<Result<UpdateCheckResult, UpdateCheckError>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = check_for_updates(REPO, BRANCH);
        let _ = tx.send(result);
    });
    rx
}
