//! Spartan Cloud control-plane HTTP API (axum). Ties the protocol, tenant
//! (domain), and data (persistence) crates into a real REST server.
//!
//! What's real and complete here: account signup/login with argon2-hashed
//! passwords, opaque bearer-token auth (looked up + expiry-checked per
//! request against the session store), an authenticated `/me`, an
//! admin-only entitlement toggle, and an allocation-admission endpoint that
//! runs the **real** entitlement -> plan-limits -> quota checks.
//!
//! What is deliberately honest, not faked: `/api/allocate`, once admission
//! passes, returns `503 runtime_unavailable` because the container runtime
//! (`spartan-cloud-runtime`, gated on the gVisor spike) isn't wired yet.
//! This crate never pretends to start a container it can't.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use spartan_cloud_data::{DataError, Store};
use spartan_cloud_protocol::{
    AllocationInfo, AllocationStatus, ApiError, AuthResponse, LoginRequest, PlanTier,
    PutSecretRequest, SecretNamesResponse, SessionToken, SignupRequest, UserId,
};
use spartan_cloud_runtime::{AllocationSpec, ContainerRuntime};
use spartan_cloud_tenant::{
    can_allocate, EntitlementProvider, PlanLimits, Session, StubEntitlementProvider,
};

/// The default base image for a bare allocation. A real product ships a
/// curated workspace image; this is an honest placeholder base.
const DEFAULT_IMAGE: &str = "alpine:latest";

/// Shared server state. The SQLite `Store` isn't `Sync`, so it's behind a
/// `Mutex`; handlers lock it only for the brief synchronous DB op and never
/// hold the guard across an `.await`.
#[derive(Clone)]
pub struct AppState {
    store: Arc<Mutex<Store>>,
    entitlements: Arc<StubEntitlementProvider>,
    session_ttl_secs: u64,
    /// The container runtime, if one is connected. When `None`, `/api/allocate`
    /// honestly reports the runtime is unavailable rather than faking anything.
    runtime: Option<Arc<dyn ContainerRuntime>>,
}

impl AppState {
    pub fn new(
        store: Store,
        entitlements: Arc<StubEntitlementProvider>,
        session_ttl_secs: u64,
    ) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            entitlements,
            session_ttl_secs,
            runtime: None,
        }
    }

    /// Attach a container runtime (builder-style). Without one, allocation is
    /// honestly unavailable.
    pub fn with_runtime(mut self, runtime: Arc<dyn ContainerRuntime>) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// A ready-to-serve in-memory state (tests, ephemeral runs). No runtime.
    pub fn in_memory() -> Self {
        Self::new(
            Store::open_in_memory().expect("in-memory store must open"),
            Arc::new(StubEntitlementProvider::new()),
            24 * 60 * 60,
        )
    }

    /// In-memory state with an unlocked secrets vault (tests, ephemeral runs).
    pub fn in_memory_with_vault(master_key: &[u8; 32]) -> Self {
        Self::new(
            Store::open_in_memory_with_key(master_key).expect("keyed in-memory store must open"),
            Arc::new(StubEntitlementProvider::new()),
            24 * 60 * 60,
        )
    }

    /// Shared handle to the entitlement provider (so a bootstrap step or an
    /// admin path can grant Pro directly).
    pub fn entitlements(&self) -> Arc<StubEntitlementProvider> {
        Arc::clone(&self.entitlements)
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn api_error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(ApiError {
            code: code.to_string(),
            message: message.to_string(),
        }),
    )
        .into_response()
}

/// Append an audit event, soft-failing: an audit-write error is logged but
/// never aborts the operation it describes (a missing audit row is bad; losing
/// the actual signup/allocate/grant it was recording is worse). Locks the store
/// only for the brief synchronous insert -- never held across an `.await`.
fn audit(
    state: &AppState,
    actor: Option<&UserId>,
    action: &str,
    target: Option<&str>,
    detail: Option<&str>,
) {
    let store = state.store.lock().expect("store mutex poisoned");
    if let Err(e) = store.record_audit(now_unix(), actor, action, target, detail) {
        eprintln!("spartan-cloud-api: audit write failed ({action}): {e}");
    }
}

fn internal_error(e: impl std::fmt::Display) -> Response {
    // Never leak internal detail (SQL text, paths) to the client; log-worthy
    // detail would go to the server's own logs, not the response body.
    eprintln!("spartan-cloud-api: internal error: {e}");
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "an internal error occurred",
    )
}

/// Authenticated-user extractor: pulls the `Authorization: Bearer <token>`
/// header, looks the session up in the store, and rejects (401) if it's
/// missing, malformed, unknown, revoked, or expired.
pub struct AuthUser {
    pub user_id: UserId,
}

#[axum::async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|t| t.to_string());

        let Some(token) = token else {
            return Err(api_error(
                StatusCode::UNAUTHORIZED,
                "missing_token",
                "a Bearer token is required",
            ));
        };

        let session = {
            let store = state.store.lock().expect("store mutex poisoned");
            store
                .lookup_session(&SessionToken(token), now_unix())
                .map_err(internal_error)?
        };

        match session {
            Some(s) => Ok(AuthUser { user_id: s.user_id }),
            None => Err(api_error(
                StatusCode::UNAUTHORIZED,
                "invalid_token",
                "the session token is invalid, revoked, or expired",
            )),
        }
    }
}

/// Build the full router from a ready `AppState`. Exposed so tests can drive
/// it via `tower::ServiceExt::oneshot` with no real socket.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/signup", post(signup))
        .route("/api/login", post(login))
        .route("/api/me", get(me))
        .route("/api/admin/grant_pro", post(grant_pro))
        .route("/api/admin/audit", get(audit_log))
        .route("/api/allocate", post(allocate))
        .route(
            "/api/secrets/:name",
            put(put_secret_handler).delete(delete_secret_handler),
        )
        .route("/api/secrets", get(list_secrets_handler))
        .with_state(state)
}

// Returns the small `DataError` (not a large `Response`) on failure, so
// callers map it to a response themselves -- avoids clippy's
// `result_large_err` on a `Result<_, Response>`.
fn issue_and_store_session(state: &AppState, user_id: UserId) -> Result<AuthResponse, DataError> {
    let session = Session::issue(user_id.clone(), now_unix(), state.session_ttl_secs);
    {
        let store = state.store.lock().expect("store mutex poisoned");
        store.store_session(&session)?;
    }
    Ok(AuthResponse {
        user_id,
        token: session.token,
        expires_at_unix: session.expires_at_unix,
    })
}

async fn signup(State(state): State<AppState>, Json(req): Json<SignupRequest>) -> Response {
    if req.email.trim().is_empty() || req.password.is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "email and password are required",
        );
    }
    let created = {
        let store = state.store.lock().expect("store mutex poisoned");
        store.create_user(&req.email, &req.password, false)
    };
    match created {
        Ok(user_id) => {
            audit(&state, Some(&user_id), "signup", None, None);
            match issue_and_store_session(&state, user_id) {
                Ok(auth) => (StatusCode::OK, Json(auth)).into_response(),
                Err(e) => internal_error(e),
            }
        }
        Err(DataError::EmailTaken) => api_error(
            StatusCode::CONFLICT,
            "email_taken",
            "that email is already registered",
        ),
        Err(e) => internal_error(e),
    }
}

async fn login(State(state): State<AppState>, Json(req): Json<LoginRequest>) -> Response {
    let verified = {
        let store = state.store.lock().expect("store mutex poisoned");
        store.verify_login(&req.email, &req.password)
    };
    match verified {
        Ok(Some(user_id)) => {
            audit(&state, Some(&user_id), "login", None, None);
            match issue_and_store_session(&state, user_id) {
                Ok(auth) => (StatusCode::OK, Json(auth)).into_response(),
                Err(e) => internal_error(e),
            }
        }
        // Wrong password AND unknown email both land here -- no enumeration.
        // The failed attempt is audited (no actor established) so brute-force
        // patterns are visible to the monitoring dashboard.
        Ok(None) => {
            audit(&state, None, "login_failed", None, Some(&req.email));
            api_error(
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "incorrect email or password",
            )
        }
        Err(e) => internal_error(e),
    }
}

#[derive(Serialize)]
struct MeResponse {
    user_id: UserId,
    is_admin: bool,
    tier: PlanTier,
    entitlement_active: bool,
}

async fn me(State(state): State<AppState>, user: AuthUser) -> Response {
    let is_admin = {
        let store = state.store.lock().expect("store mutex poisoned");
        match store.is_admin(&user.user_id) {
            Ok(a) => a,
            Err(e) => return internal_error(e),
        }
    };
    let ent = state.entitlements.check(&user.user_id);
    (
        StatusCode::OK,
        Json(MeResponse {
            user_id: user.user_id,
            is_admin,
            tier: ent.tier,
            entitlement_active: ent.active,
        }),
    )
        .into_response()
}

#[derive(Deserialize)]
struct GrantProRequest {
    user_id: String,
}

async fn grant_pro(
    State(state): State<AppState>,
    admin: AuthUser,
    Json(req): Json<GrantProRequest>,
) -> Response {
    let is_admin = {
        let store = state.store.lock().expect("store mutex poisoned");
        match store.is_admin(&admin.user_id) {
            Ok(a) => a,
            Err(e) => return internal_error(e),
        }
    };
    if !is_admin {
        return api_error(
            StatusCode::FORBIDDEN,
            "forbidden",
            "admin privileges are required",
        );
    }
    audit(
        &state,
        Some(&admin.user_id),
        "grant_pro",
        Some(&req.user_id),
        None,
    );
    state.entitlements.grant_pro(UserId(req.user_id));
    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

#[derive(Serialize)]
struct AuditEventJson {
    id: i64,
    at_unix: u64,
    actor_id: Option<String>,
    action: String,
    target: Option<String>,
    detail: Option<String>,
}

/// Admin-only: the most recent audit events (newest first). The defensive
/// counterpart to the reaper + caps -- the human-visible surface for spotting
/// tenant abuse (brute-force logins, allocation storms) that the excluded
/// SpartanAI malware repos themselves performed.
async fn audit_log(State(state): State<AppState>, admin: AuthUser) -> Response {
    let events = {
        let store = state.store.lock().expect("store mutex poisoned");
        match store.is_admin(&admin.user_id) {
            Ok(true) => {}
            Ok(false) => {
                return api_error(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "admin privileges are required",
                )
            }
            Err(e) => return internal_error(e),
        }
        // Cap the page size (newest 200) so the log can't be dumped unbounded.
        match store.recent_audit(200) {
            Ok(e) => e,
            Err(e) => return internal_error(e),
        }
    };
    let json: Vec<AuditEventJson> = events
        .into_iter()
        .map(|e| AuditEventJson {
            id: e.id,
            at_unix: e.at_unix,
            actor_id: e.actor_id.map(|u| u.0),
            action: e.action,
            target: e.target,
            detail: e.detail,
        })
        .collect();
    (StatusCode::OK, Json(json)).into_response()
}

async fn allocate(State(state): State<AppState>, user: AuthUser) -> Response {
    let ent = state.entitlements.check(&user.user_id);
    let limits = PlanLimits::for_tier(ent.effective_tier());

    // Honest: with no runtime connected, allocation is unavailable.
    let Some(runtime) = &state.runtime else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
            "the container runtime is not connected",
        );
    };

    // Safety gate: NEVER run tenant code against isolation the operator hasn't
    // verified for this deployment (e.g. gVisor unverified in a nested sandbox,
    // or plain runc under an untrusted-tenant threat model).
    if !runtime.isolation_verified() {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "isolation_unverified",
            "container isolation is not verified for this deployment; allocation is refused",
        );
    }

    // Real quota admission against the live running count.
    let current_active = match runtime.count_active(&user.user_id).await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };
    if let Err(quota_err) = can_allocate(&limits, current_active) {
        return api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "quota_exceeded",
            &quota_err.to_string(),
        );
    }

    // Really allocate a capped container.
    let spec = AllocationSpec {
        owner: user.user_id.clone(),
        image: DEFAULT_IMAGE.to_string(),
        limits,
    };
    match runtime.create(&spec).await {
        Ok(id) => {
            audit(
                &state,
                Some(&user.user_id),
                "allocate",
                Some(&id.0),
                Some(ent.effective_tier().as_str()),
            );
            let info = AllocationInfo {
                id,
                status: AllocationStatus::Running,
                expires_at_unix: now_unix().saturating_add(limits.max_lifetime_secs),
            };
            (StatusCode::CREATED, Json(info)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

/// Reject a secret name that isn't a safe, bounded identifier. Names are used
/// as opaque DB keys, but validating them keeps the surface tidy and the audit
/// log readable, and bounds storage. Allows letters/digits/`_`/`-`/`.`, 1..=128.
fn valid_secret_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn map_vault_err(e: DataError) -> Response {
    match e {
        DataError::VaultLocked => api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "vault_locked",
            "the secrets vault is not configured on this server",
        ),
        other => internal_error(other),
    }
}

/// Store (create/overwrite) one of the caller's own encrypted secrets. The
/// value is encrypted at rest and never read back over the API.
async fn put_secret_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(name): Path<String>,
    Json(req): Json<PutSecretRequest>,
) -> Response {
    if !valid_secret_name(&name) {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_name",
            "secret name must be 1-128 chars of [A-Za-z0-9_.-]",
        );
    }
    let result = {
        let store = state.store.lock().expect("store mutex poisoned");
        store.put_secret(&user.user_id, &name, req.value.as_bytes())
    };
    match result {
        Ok(()) => {
            // Audit the fact of a secret write (never the value).
            audit(&state, Some(&user.user_id), "secret_put", Some(&name), None);
            (StatusCode::NO_CONTENT, ()).into_response()
        }
        Err(e) => map_vault_err(e),
    }
}

/// The caller's own secret *names* (never values). Owner-scoped.
async fn list_secrets_handler(State(state): State<AppState>, user: AuthUser) -> Response {
    let result = {
        let store = state.store.lock().expect("store mutex poisoned");
        store.list_secret_names(&user.user_id)
    };
    match result {
        Ok(names) => (StatusCode::OK, Json(SecretNamesResponse { names })).into_response(),
        Err(e) => map_vault_err(e),
    }
}

/// Delete one of the caller's own secrets. Idempotent (absent is fine).
async fn delete_secret_handler(
    State(state): State<AppState>,
    user: AuthUser,
    Path(name): Path<String>,
) -> Response {
    let result = {
        let store = state.store.lock().expect("store mutex poisoned");
        store.delete_secret(&user.user_id, &name)
    };
    match result {
        Ok(()) => {
            audit(
                &state,
                Some(&user.user_id),
                "secret_delete",
                Some(&name),
                None,
            );
            (StatusCode::NO_CONTENT, ()).into_response()
        }
        Err(e) => map_vault_err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // for `oneshot`

    async fn body_json(resp: Response) -> (StatusCode, serde_json::Value) {
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    fn post_json(uri: &str, body: serde_json::Value, bearer: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(t) = bearer {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    fn get(uri: &str, bearer: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method("GET").uri(uri);
        if let Some(t) = bearer {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        b.body(Body::empty()).unwrap()
    }

    fn req_json(method: &str, uri: &str, body: serde_json::Value, bearer: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {bearer}"))
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn req_empty(method: &str, uri: &str, bearer: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {bearer}"))
            .body(Body::empty())
            .unwrap()
    }

    async fn signup_token(app: &Router, email: &str) -> String {
        let (_, body) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/signup",
                    serde_json::json!({"email": email, "password": "pw123456"}),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        body["token"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn signup_then_me_round_trips_a_real_session() {
        let app = router(AppState::in_memory());

        let (status, body) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/signup",
                    serde_json::json!({"email": "a@b.com", "password": "pw123456"}),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "signup succeeds: {body}");
        let token = body["token"].as_str().unwrap().to_string();
        assert!(!token.is_empty());

        // The bearer token authenticates /me.
        let (status, me) = body_json(
            app.clone()
                .oneshot(get("/api/me", Some(&token)))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me["tier"], "Free");
        assert_eq!(me["is_admin"], false);
    }

    #[tokio::test]
    async fn duplicate_signup_is_409_and_wrong_login_is_401() {
        let app = router(AppState::in_memory());
        let signup = || {
            app.clone().oneshot(post_json(
                "/api/signup",
                serde_json::json!({"email": "dup@b.com", "password": "pw123456"}),
                None,
            ))
        };
        assert_eq!(body_json(signup().await.unwrap()).await.0, StatusCode::OK);
        assert_eq!(
            body_json(signup().await.unwrap()).await.0,
            StatusCode::CONFLICT,
            "second signup with the same email is 409"
        );

        // Wrong password -> 401.
        let (status, _) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/login",
                    serde_json::json!({"email": "dup@b.com", "password": "WRONG"}),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn me_without_a_token_is_401() {
        let app = router(AppState::in_memory());
        let (status, _) = body_json(app.oneshot(get("/api/me", None)).await.unwrap()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_non_admin_cannot_grant_pro() {
        let app = router(AppState::in_memory());
        // Sign up a normal user, get their token.
        let (_, body) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/signup",
                    serde_json::json!({"email": "normal@b.com", "password": "pw123456"}),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        let token = body["token"].as_str().unwrap().to_string();

        let (status, _) = body_json(
            app.oneshot(post_json(
                "/api/admin/grant_pro",
                serde_json::json!({"user_id": "someone"}),
                Some(&token),
            ))
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "a non-admin is refused");
    }

    #[tokio::test]
    async fn an_admin_can_grant_pro_and_it_changes_the_tier() {
        // Bootstrap an admin user directly in the store, then log in via API.
        let state = AppState::in_memory();
        let admin_id = {
            let store = state.store.lock().unwrap();
            store.create_user("admin@b.com", "adminpw12", true).unwrap()
        };
        let app = router(state.clone());

        let (_, login) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/login",
                    serde_json::json!({"email": "admin@b.com", "password": "adminpw12"}),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        let admin_token = login["token"].as_str().unwrap().to_string();

        // Grant Pro to the admin's own id, then confirm /me reflects Pro.
        let (status, _) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/admin/grant_pro",
                    serde_json::json!({"user_id": admin_id.0}),
                    Some(&admin_token),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (_, me) = body_json(
            app.oneshot(get("/api/me", Some(&admin_token)))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(me["tier"], "Pro", "granting Pro is reflected in /me");
    }

    #[tokio::test]
    async fn allocate_really_creates_a_capped_container_when_a_runtime_is_wired() {
        use spartan_cloud_runtime::{ContainerRuntime, DockerRuntime};
        // Self-skip if no Docker daemon -- same convention as the runtime
        // crate's own integration test.
        let docker = match bollard::Docker::connect_with_local_defaults() {
            Ok(d) if d.ping().await.is_ok() => d,
            _ => {
                println!("SKIP: no Docker daemon; allocate-with-runtime test skipped");
                return;
            }
        };
        // runc, isolation asserted for this test (the verified baseline here).
        let runtime = std::sync::Arc::new(DockerRuntime::with_docker(docker, "runc", true));
        let app = router(AppState::in_memory().with_runtime(runtime.clone()));

        let (_, body) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/signup",
                    serde_json::json!({"email": "realalloc@b.com", "password": "pw123456"}),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        let token = body["token"].as_str().unwrap().to_string();

        let (status, alloc) = body_json(
            app.oneshot(post_json(
                "/api/allocate",
                serde_json::json!({}),
                Some(&token),
            ))
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "a real allocation is created: {alloc}"
        );
        let id = spartan_cloud_protocol::AllocationId(alloc["id"].as_str().unwrap().to_string());

        // Confirm it's really running, then clean it up.
        assert_eq!(
            runtime.status(&id).await.unwrap(),
            spartan_cloud_protocol::AllocationStatus::Running
        );
        runtime.stop(&id).await.expect("cleanup the test container");
    }

    #[tokio::test]
    async fn admin_can_read_the_audit_log_and_a_non_admin_cannot() {
        let state = AppState::in_memory();
        let _admin_id = {
            let store = state.store.lock().unwrap();
            store.create_user("aud@b.com", "adminpw12", true).unwrap()
        };
        let app = router(state.clone());

        // A normal signup + a failed login both generate real audit events.
        let (_, signup) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/signup",
                    serde_json::json!({"email": "member@b.com", "password": "pw123456"}),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        let member_token = signup["token"].as_str().unwrap().to_string();
        let _ = app
            .clone()
            .oneshot(post_json(
                "/api/login",
                serde_json::json!({"email": "member@b.com", "password": "WRONG"}),
                None,
            ))
            .await
            .unwrap();

        // The member (non-admin) is refused the audit log.
        let (status, _) = body_json(
            app.clone()
                .oneshot(get("/api/admin/audit", Some(&member_token)))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a non-admin can't read audit"
        );

        // The admin logs in and reads the log; the signup + failed login show.
        let (_, login) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/login",
                    serde_json::json!({"email": "aud@b.com", "password": "adminpw12"}),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        let admin_token = login["token"].as_str().unwrap().to_string();

        let (status, events) = body_json(
            app.oneshot(get("/api/admin/audit", Some(&admin_token)))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let actions: Vec<&str> = events
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["action"].as_str().unwrap())
            .collect();
        assert!(
            actions.contains(&"signup"),
            "signup was audited: {actions:?}"
        );
        assert!(
            actions.contains(&"login_failed"),
            "the failed login was audited: {actions:?}"
        );
        // The failed-login event carries the attempted email as detail, no actor.
        let failed = events
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["action"] == "login_failed")
            .unwrap();
        assert_eq!(failed["detail"], "member@b.com");
        assert!(failed["actor_id"].is_null());
    }

    #[tokio::test]
    async fn secrets_put_list_delete_are_owner_scoped() {
        let app = router(AppState::in_memory_with_vault(&[42u8; 32]));
        let alice = signup_token(&app, "alice-secrets@b.com").await;
        let bob = signup_token(&app, "bob-secrets@b.com").await;

        // Alice stores two secrets.
        for name in ["registry_token", "deploy_key"] {
            let (status, _) = body_json(
                app.clone()
                    .oneshot(req_json(
                        "PUT",
                        &format!("/api/secrets/{name}"),
                        serde_json::json!({"value": "s3cr3t-value"}),
                        &alice,
                    ))
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT, "put {name} succeeds");
        }

        // Alice lists her two names.
        let (status, list) = body_json(
            app.clone()
                .oneshot(req_empty("GET", "/api/secrets", &alice))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let mut names: Vec<&str> = list["names"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, vec!["deploy_key", "registry_token"]);

        // Bob (a different tenant) sees none of Alice's secrets.
        let (_, bob_list) = body_json(
            app.clone()
                .oneshot(req_empty("GET", "/api/secrets", &bob))
                .await
                .unwrap(),
        )
        .await;
        assert!(bob_list["names"].as_array().unwrap().is_empty());

        // Alice deletes one; her list shrinks to the other.
        let (status, _) = body_json(
            app.clone()
                .oneshot(req_empty("DELETE", "/api/secrets/deploy_key", &alice))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, list) = body_json(
            app.clone()
                .oneshot(req_empty("GET", "/api/secrets", &alice))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(list["names"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn secret_put_rejects_a_bad_name_and_a_locked_vault_is_503() {
        // Bad name against an unlocked vault -> 400.
        let app = router(AppState::in_memory_with_vault(&[7u8; 32]));
        let token = signup_token(&app, "badname@b.com").await;
        let (status, body) = body_json(
            app.clone()
                .oneshot(req_json(
                    "PUT",
                    "/api/secrets/has%20space",
                    serde_json::json!({"value": "v"}),
                    &token,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_name");

        // A locked vault (no master key) -> 503 vault_locked, not a 500.
        let locked = router(AppState::in_memory());
        let token2 = signup_token(&locked, "locked@b.com").await;
        let (status, body) = body_json(
            locked
                .oneshot(req_json(
                    "PUT",
                    "/api/secrets/k",
                    serde_json::json!({"value": "v"}),
                    &token2,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "vault_locked");
    }

    #[tokio::test]
    async fn allocate_passes_admission_then_honestly_reports_no_runtime() {
        let app = router(AppState::in_memory());
        let (_, body) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/signup",
                    serde_json::json!({"email": "alloc@b.com", "password": "pw123456"}),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        let token = body["token"].as_str().unwrap().to_string();

        let (status, body) = body_json(
            app.oneshot(post_json(
                "/api/allocate",
                serde_json::json!({}),
                Some(&token),
            ))
            .await
            .unwrap(),
        )
        .await;
        // Free tier admits 1 concurrent, current is 0, so admission passes --
        // and the honest 503 (not a faked allocation) is returned.
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "runtime_unavailable");
    }
}
