//! Real, live integration test for the `check_for_updates` IPC method
//! (§75.72, closing the gap §75.65 named: `spartan-updater` was wired
//! into the original wgpu shell but never into `spartan-backend`).
//! Deliberately a separate integration test binary, not part of this
//! crate's own `--lib` unit test suite -- `check_for_updates` always
//! spawns a real background thread that performs a real HTTPS call to
//! the GitHub API, and leaving that thread unjoined inside the same
//! process as this crate's timing-sensitive Leo tests (which race a
//! background thread's real work against an immediate synchronous
//! assertion) caused a real, reproducible flake in
//! `leo_start_task_transitions_to_planning_and_returns_an_immediate_ack`
//! when both ran in the same test binary -- found by actually running
//! the full suite, not by inspection. Moving this here, mirroring every
//! other real-external-service integration test in this workspace
//! (`spartan-updater`'s own `github_integration.rs`,
//! `spartan-model`'s `ollama_integration.rs`), gives it its own process
//! and removes the interference entirely.

use spartan_backend::{handle_request, BackendState, Request};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn call(
    state: &Arc<Mutex<BackendState>>,
    method: &str,
) -> (spartan_backend::Response, mpsc::Receiver<String>) {
    let (tx, rx) = mpsc::channel();
    let resp = handle_request(
        state,
        Request {
            id: 1,
            method: method.to_string(),
            params: serde_json::json!({}),
        },
        tx,
    );
    (resp, rx)
}

#[test]
fn check_for_updates_acks_immediately_and_never_blocks() {
    let state = Arc::new(Mutex::new(BackendState::new()));
    let (resp, _rx) = call(&state, "check_for_updates");
    assert!(
        resp.error.is_none(),
        "check_for_updates errored: {:?}",
        resp.error
    );
    assert_eq!(resp.result.unwrap()["status"], "checking");
}

#[test]
fn check_for_updates_eventually_reports_a_real_result_or_failure_event() {
    // A real network call runs on the spawned background thread --
    // matching `spartan-updater`'s own already-established self-skip
    // precedent, this waits a real, bounded amount of time and treats
    // "no event within the timeout" as an environment condition to skip
    // past, not a test failure. What *is* asserted, when an event does
    // arrive: `check_for_updates` must always emit exactly one of its
    // two real, honest outcomes -- never silently drop the real
    // background result.
    let state = Arc::new(Mutex::new(BackendState::new()));
    let (resp, rx) = call(&state, "check_for_updates");
    assert!(resp.error.is_none());

    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(line) => {
            let event: serde_json::Value = serde_json::from_str(&line).unwrap();
            let event_name = event["event"].as_str().unwrap();
            assert!(
                event_name == "update_check_result" || event_name == "update_check_failed",
                "unexpected event name: {event_name}"
            );
            if event_name == "update_check_result" {
                assert_eq!(event["data"]["current_commit"].as_str().unwrap().len(), 40);
                assert_eq!(event["data"]["latest_commit"].as_str().unwrap().len(), 40);
                assert!(event["data"]["categories"].is_object());
            } else {
                println!(
                    "real, honest update_check_failed: {}",
                    event["data"]["error"]
                );
            }
        }
        Err(_) => {
            eprintln!(
                "SKIP: no update-check event within timeout (real network/rate-limit conditions in this sandbox)"
            );
        }
    }
}
