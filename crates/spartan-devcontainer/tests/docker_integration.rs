//! Real, live integration test against an actual local Docker daemon.
//! Self-skips (rather than fails) when no daemon is reachable, matching
//! every other real-external-service integration test in this workspace
//! (`spartan-updater`'s `github_integration.rs`, `spartan-model`'s
//! `ollama_integration.rs`) -- this sandboxed development environment
//! itself has no Docker daemon running (confirmed directly: `docker
//! version` succeeds against the CLI, but there is no
//! `/var/run/docker.sock` to connect to), so this test is expected to
//! self-skip here, not a gap being papered over.

use spartan_devcontainer::docker;
use spartan_devcontainer::spec::DevContainerConfig;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn real_container_lifecycle_pull_create_exec_stop_remove() {
    if !docker::is_docker_available() {
        eprintln!("SKIP: no real Docker daemon reachable in this environment");
        return;
    }

    let image = "alpine:latest";
    let mut pulled_lines = Vec::new();
    docker::pull_image(image, |line| pulled_lines.push(line)).expect("real image pull failed");
    assert!(
        !pulled_lines.is_empty(),
        "expected real pull progress output"
    );

    let project_dir = tempfile::tempdir().unwrap();
    std::fs::write(project_dir.path().join("marker.txt"), "hello from the host").unwrap();

    let config = DevContainerConfig {
        image: Some(image.to_string()),
        workspace_folder: Some("/workspace".to_string()),
        ..Default::default()
    };

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let container_name = format!("spartan-devcontainer-test-{nonce}");
    let project_label = format!("test-project-{nonce}");

    let container_id = docker::create_and_start_container(
        image,
        &config,
        project_dir.path(),
        &project_label,
        &container_name,
    )
    .expect("real container creation/start failed");

    let status = docker::container_status(&container_id).unwrap();
    assert_eq!(status.as_deref(), Some("running"));

    let (exit_code, output) = docker::run_command(
        &container_id,
        &["cat".to_string(), "/workspace/marker.txt".to_string()],
    )
    .expect("real exec failed");
    assert_eq!(exit_code, 0);
    assert!(
        output.contains("hello from the host"),
        "expected the real host file to be visible inside the container via the real bind mount, got: {output}"
    );

    let managed = docker::list_managed_containers().unwrap();
    assert!(
        managed
            .iter()
            .any(|c| c.id == container_id && c.project_label == project_label),
        "expected the real container to appear in the real managed-containers list"
    );

    docker::stop_and_remove(&container_id).expect("real stop+remove failed");
    let status_after = docker::container_status(&container_id).unwrap();
    assert_eq!(
        status_after, None,
        "expected the real container to be genuinely gone"
    );
}

#[test]
fn real_interactive_exec_round_trip() {
    if !docker::is_docker_available() {
        eprintln!("SKIP: no real Docker daemon reachable in this environment");
        return;
    }

    let image = "alpine:latest";
    docker::pull_image(image, |_| {}).expect("real image pull failed");

    let project_dir = tempfile::tempdir().unwrap();
    let config = DevContainerConfig {
        image: Some(image.to_string()),
        ..Default::default()
    };
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let container_name = format!("spartan-devcontainer-exec-test-{nonce}");
    let container_id = docker::create_and_start_container(
        image,
        &config,
        project_dir.path(),
        "exec-test",
        &container_name,
    )
    .expect("real container creation/start failed");

    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (exit_tx, exit_rx) = std::sync::mpsc::channel::<()>();
    let handle = docker::spawn_interactive_exec(
        &container_id,
        80,
        24,
        move |bytes| {
            let _ = tx.send(bytes);
        },
        move || {
            let _ = exit_tx.send(());
        },
    )
    .expect("real interactive exec spawn failed");

    handle.write(b"echo REAL_EXEC_MARKER\n").unwrap();

    let mut collected = String::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline && !collected.contains("REAL_EXEC_MARKER") {
        if let Ok(bytes) = rx.recv_timeout(std::time::Duration::from_millis(500)) {
            collected.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    assert!(
        collected.contains("REAL_EXEC_MARKER"),
        "expected real echoed output from the real interactive exec session, got: {collected}"
    );

    handle.close();
    let _ = exit_rx.recv_timeout(std::time::Duration::from_secs(5));

    docker::stop_and_remove(&container_id).expect("real stop+remove failed");
}
