//! Real background-thread bridge to `spartan-leo`'s plan generation (§4,
//! task #5, §75.47) -- the same spawn-thread/channel/non-blocking-poll
//! shape `build.rs`'s DAP build integration (§75.10) and `gui_bridge.rs`'s
//! dev-server bridge (§75.41) already established, applied here to a real
//! blocking model call instead of a subprocess. `spartan_leo::plan::
//! generate_plan` can take a long time against a local, non-GPU-accelerated
//! model -- this must never run on the render thread.

use spartan_leo::plan::{generate_plan, ImplementationPlan, PlanError};
use spartan_model::OllamaProvider;
use std::sync::mpsc;
use std::thread;

/// The model this build targets for Leo's own plan generation -- the
/// real, live-proven §75.43/§75.46 target class, not a smaller stand-in.
pub const LEO_MODEL: &str = "llama3.1:8b";

/// Spawns a real, one-shot background thread that calls
/// `spartan_leo::plan::generate_plan` against a real local Ollama
/// instance and reports the result back over `mpsc`. The caller polls
/// the returned receiver non-blockingly (`try_recv`) once per frame,
/// matching every other real background-thread bridge in this crate.
pub fn spawn_plan_request(task: String) -> mpsc::Receiver<Result<ImplementationPlan, PlanError>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        // Real §57/§42 GPU offload setting (user-requested), read fresh
        // per request rather than cached -- a change made in the settings
        // panel takes effect on the *next* task, not retroactively.
        let gpu_offload = spartan_settings::load().gpu_offload;
        let provider = OllamaProvider::local(LEO_MODEL).with_gpu_layers(gpu_offload.num_gpu());
        let result = generate_plan(&provider, &task);
        let _ = tx.send(result);
    });
    rx
}
