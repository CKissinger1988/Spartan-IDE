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

/// Parse a 64-hex-char vault master key into 32 raw bytes. Returns `None` for
/// any wrong length or non-hex input (a clear, early failure at startup rather
/// than a silently truncated/derived key).
fn parse_vault_key(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).ok()?;
        key[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(key)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let bind = parse_flag(&args, "--bind:")
        .unwrap_or("127.0.0.1:8080")
        .to_string();
    let db_path = parse_flag(&args, "--db:").unwrap_or("spartan-cloud.db");

    // The at-rest secrets-vault master key is env-provided (64 hex chars = 32
    // bytes), never persisted alongside the ciphertext and never hardcoded. If
    // absent, the store opens without a key and vault operations are refused
    // (a locked vault) rather than silently storing plaintext.
    let store = match std::env::var("SPARTAN_CLOUD_VAULT_KEY").ok() {
        Some(hex) => {
            let key = parse_vault_key(&hex)
                .ok_or("SPARTAN_CLOUD_VAULT_KEY must be exactly 64 hex chars (32 bytes)")?;
            eprintln!("spartan-cloud-api: secrets vault unlocked (master key from environment)");
            Store::open_with_key(db_path, &key)?
        }
        None => {
            eprintln!("spartan-cloud-api: no SPARTAN_CLOUD_VAULT_KEY set; secrets vault is locked");
            Store::open(db_path)?
        }
    };

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
            let runtime: std::sync::Arc<dyn ContainerRuntime> = std::sync::Arc::new(runtime);

            // Independent reaper task: enforces every allocation's hard
            // wall-clock lifetime (PlanLimits::max_lifetime_secs, §36.4.7's
            // "uncapped consumption" defense) regardless of tenant activity.
            // Runs on its own interval so a wedged or forgotten container is
            // still killed at its deadline.
            let reaper = std::sync::Arc::clone(&runtime);
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    ticker.tick().await;
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    match reaper.reap_expired(now).await {
                        Ok(ids) if !ids.is_empty() => {
                            eprintln!(
                                "spartan-cloud-api: reaper stopped {} expired allocation(s)",
                                ids.len()
                            );
                        }
                        Ok(_) => {}
                        Err(e) => eprintln!("spartan-cloud-api: reaper error: {e}"),
                    }
                }
            });

            state = state.with_runtime(runtime);
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

#[cfg(test)]
mod tests {
    use super::parse_vault_key;

    #[test]
    fn parse_vault_key_accepts_exactly_64_hex_chars() {
        let hex = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        let key = parse_vault_key(hex).expect("valid 64-hex key");
        assert_eq!(key[0], 0x00);
        assert_eq!(key[1], 0x11);
        assert_eq!(key[31], 0xff);
    }

    #[test]
    fn parse_vault_key_rejects_wrong_length_and_non_hex() {
        assert!(parse_vault_key("").is_none());
        assert!(parse_vault_key("abcd").is_none(), "too short");
        assert!(
            parse_vault_key(&"a".repeat(63)).is_none(),
            "63 chars is not 32 bytes"
        );
        assert!(
            parse_vault_key(&"zz".repeat(32)).is_none(),
            "non-hex characters are rejected"
        );
    }
}
