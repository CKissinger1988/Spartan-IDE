//! Spartan Cloud control-plane server binary. Serves the real REST API on
//! `--bind:<addr>` (default `127.0.0.1:8080`), backed by a SQLite database
//! at `--db:<path>` (default `spartan-cloud.db`).
//!
//! An initial admin can be bootstrapped from the environment on first run:
//! set `SPARTAN_CLOUD_ADMIN_EMAIL` + `SPARTAN_CLOUD_ADMIN_PASSWORD` and the
//! server creates that admin account if it doesn't already exist. Never a
//! hardcoded credential -- matching ws_transport's own "no persisted
//! well-known secret" posture.

use std::sync::Arc;

use spartan_cloud_api::{router, AppState};
use spartan_cloud_data::Store;
use spartan_cloud_runtime::ContainerRuntime;
use spartan_cloud_tenant::StubEntitlementProvider;

fn parse_flag<'a>(args: &'a [String], prefix: &str) -> Option<&'a str> {
    args.iter().find_map(|a| a.strip_prefix(prefix))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let bind = parse_flag(&args, "--bind:")
        .unwrap_or("127.0.0.1:8080")
        .to_string();
    let db_path = parse_flag(&args, "--db:").unwrap_or("spartan-cloud.db");

    let store = Store::open(db_path)?;

    // Optional, env-driven admin bootstrap -- created once if absent.
    if let (Ok(email), Ok(password)) = (
        std::env::var("SPARTAN_CLOUD_ADMIN_EMAIL"),
        std::env::var("SPARTAN_CLOUD_ADMIN_PASSWORD"),
    ) {
        match store.create_user(&email, &password, true) {
            Ok(_) => eprintln!("spartan-cloud-api: bootstrapped admin account {email}"),
            Err(spartan_cloud_data::DataError::EmailTaken) => {
                eprintln!("spartan-cloud-api: admin account {email} already exists");
            }
            Err(e) => return Err(e.into()),
        }
    }

    let mut state = AppState::new(
        store,
        Arc::new(StubEntitlementProvider::new()),
        24 * 60 * 60,
    );

    // Optionally connect a container runtime. The OCI runtime is env-selected
    // (default `runc`); isolation is treated as UNVERIFIED unless the operator
    // explicitly asserts it for this deployment via
    // SPARTAN_CLOUD_ISOLATION_VERIFIED=1 -- a deliberate safe default, since
    // /api/allocate refuses to run tenant code against unverified isolation.
    match spartan_cloud_runtime::DockerRuntime::connect(
        std::env::var("SPARTAN_CLOUD_OCI_RUNTIME").unwrap_or_else(|_| "runc".to_string()),
        std::env::var("SPARTAN_CLOUD_ISOLATION_VERIFIED").as_deref() == Ok("1"),
    ) {
        Ok(runtime) => {
            eprintln!(
                "spartan-cloud-api: container runtime connected (oci={}, isolation_verified={})",
                runtime.oci_runtime(),
                runtime.isolation_verified()
            );
            state = state.with_runtime(std::sync::Arc::new(runtime));
        }
        Err(e) => {
            eprintln!(
                "spartan-cloud-api: no container runtime ({e}); /api/allocate reports unavailable"
            );
        }
    }

    let app = router(state);

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    eprintln!("spartan-cloud-api: listening on http://{bind}");
    axum::serve(listener, app).await?;
    Ok(())
}
