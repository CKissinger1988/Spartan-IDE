//! Real, live LiteLLM proxy lifecycle test -- spawns an actual `litellm`
//! proxy process and drives it through the real `litellm_proxy` module's
//! spawn/health-check/stop path. Self-skips honestly (prints a message,
//! doesn't fail) if `litellm` isn't installed in this environment, matching
//! every other real-external-tool integration suite in this repo (e.g.
//! `spartan-devcontainer`'s own `docker_integration.rs`). The always-on,
//! never-skipping mechanics test lives in `litellm_proxy`'s own `#[cfg(test)]`
//! module, using a stand-in subprocess instead.

use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use spartan_backend::litellm_proxy;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("real bind")
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn real_litellm_proxy_spawns_becomes_healthy_and_stops() {
    if !litellm_proxy::is_litellm_available() {
        println!("SKIP: `litellm` isn't installed in this environment");
        return;
    }

    let port = free_port();
    let (tx, rx) = mpsc::channel();

    let mut process = litellm_proxy::spawn(port, None, tx).expect("a real litellm spawn");

    litellm_proxy::wait_for_health(
        &mut process,
        litellm_proxy::DEFAULT_HEALTH_PATH,
        Duration::from_secs(60),
    )
    .expect("a real litellm proxy must become healthy within 60s");

    assert!(
        process.is_running(),
        "the real litellm process must still be up"
    );

    process
        .stop()
        .expect("a real running litellm process must stop cleanly");

    let mut saw_a_line = false;
    while rx.try_recv().is_ok() {
        saw_a_line = true;
    }
    assert!(
        saw_a_line,
        "expected at least one real streamed startup line from litellm"
    );
}
