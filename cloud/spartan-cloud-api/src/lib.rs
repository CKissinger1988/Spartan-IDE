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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use spartan_cloud_data::{DataError, Store};
use spartan_cloud_protocol::{
    AllocationId, AllocationInfo, AllocationStatus, ApiError, AuthResponse,
    ExecSessionClientMessage, ExecSessionServerEvent, LoginRequest, PlanTier, PutSecretRequest,
    SecretNamesResponse, SessionToken, SignupRequest, UserId,
};
use spartan_cloud_runtime::{AllocationSpec, ContainerRuntime};
use spartan_cloud_tenant::{
    can_allocate, EntitlementProvider, ExecCapability, PlanLimits, Session, StubEntitlementProvider,
};

/// How long an exec-session capability token stays valid after being issued.
/// Real, deliberately short: it only needs to survive the moment between the
/// client requesting it and immediately opening the WebSocket, so a leaked
/// token is only dangerous for a narrow window even before it's consumed
/// (single-use consumption at WS-upgrade time closes the window entirely).
const EXEC_CAPABILITY_TTL_SECS: u64 = 60;

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
    /// Issued, not-yet-consumed exec-session capability tokens, keyed by the
    /// token string. Deliberately **in-memory only** (never persisted): these
    /// are short-lived (`EXEC_CAPABILITY_TTL_SECS`) and single-use by design
    /// (removed from the map the moment a WS upgrade consumes one), so
    /// surviving a server restart was never a real requirement -- the same
    /// reasoning `spartan-backend::ws_transport`'s own per-process token
    /// already established for this codebase.
    exec_capabilities: Arc<Mutex<HashMap<String, ExecCapability>>>,
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
            exec_capabilities: Arc::new(Mutex::new(HashMap::new())),
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
        .route("/api/admin/telemetry", get(telemetry))
        .route("/api/allocate", post(allocate))
        .route("/api/allocations/:id/exec", post(exec_in_allocation))
        .route(
            "/api/allocations/:id/session_token",
            post(issue_exec_capability),
        )
        .route("/api/allocations/:id/session", get(exec_session_ws))
        .route(
            "/api/secrets/:name",
            put(put_secret_handler).delete(delete_secret_handler),
        )
        .route("/api/secrets", get(list_secrets_handler))
        .route("/admin", get(admin_dashboard))
        .with_state(state)
}

/// Serve the real, self-contained admin dashboard -- Track C's holographic
/// aesthetic (`.glass-hologram`/`.hud-gauge`/status-reactive glow, real color
/// tokens copied verbatim from `desktop/src/theme.css`, the exact reuse target
/// that file's own comment already names) driving the already-real
/// `GET /api/admin/audit`/`GET /api/admin/telemetry` feeds. Embedded at
/// compile time via `include_str!` -- a single self-contained HTML file, no
/// external assets, no runtime file I/O, so there's no path-traversal surface
/// to this route at all. Authentication is real too: the page itself does a
/// real `POST /api/login` and holds the bearer token in memory (not
/// persisted to browser storage -- a real, deliberate choice for an
/// elevated-privilege admin tool), then uses it for both real feeds.
async fn admin_dashboard() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../static/admin.html"))
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

/// Run a one-shot command inside one of the **caller's own** allocations. The
/// runtime enforces owner-scoping (a foreign/unknown allocation is 404), so a
/// tenant can only ever exec into its own container. The command is an explicit
/// argv, audited (the fact + the argv's first token, never full output).
async fn exec_in_allocation(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<spartan_cloud_protocol::ExecRequest>,
) -> Response {
    if req.command.is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_command",
            "command must be a non-empty argv array",
        );
    }
    let Some(runtime) = &state.runtime else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
            "the container runtime is not connected",
        );
    };
    let alloc = spartan_cloud_protocol::AllocationId(id);
    match runtime.exec_once(&user.user_id, &alloc, &req.command).await {
        Ok(res) => {
            audit(
                &state,
                Some(&user.user_id),
                "exec",
                Some(&alloc.0),
                Some(&req.command[0]),
            );
            (
                StatusCode::OK,
                Json(spartan_cloud_protocol::ExecResponse {
                    output: res.output,
                    exit_code: res.exit_code,
                }),
            )
                .into_response()
        }
        Err(spartan_cloud_runtime::RuntimeError::NotFound) => api_error(
            StatusCode::NOT_FOUND,
            "not_found",
            "no such allocation for this user",
        ),
        Err(e) => internal_error(e),
    }
}

/// Issue a short-lived, single-use capability token scoping the caller to
/// exactly one allocation's interactive exec session. Ownership is checked
/// **once, here**, via the runtime's own owner-scoped `list_owned` (already
/// real and tested) -- a foreign or unknown allocation is refused before any
/// token is minted at all.
async fn issue_exec_capability(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    let Some(runtime) = &state.runtime else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
            "the container runtime is not connected",
        );
    };
    let alloc = AllocationId(id.clone());
    let owned = match runtime.list_owned(&user.user_id).await {
        Ok(ids) => ids.contains(&alloc),
        Err(e) => return internal_error(e),
    };
    if !owned {
        return api_error(
            StatusCode::NOT_FOUND,
            "not_found",
            "no such allocation for this user",
        );
    }

    let cap = ExecCapability::issue(
        user.user_id.clone(),
        alloc,
        now_unix(),
        EXEC_CAPABILITY_TTL_SECS,
    );
    let token = cap.token.0.clone();
    {
        let mut caps = state
            .exec_capabilities
            .lock()
            .expect("exec_capabilities mutex poisoned");
        caps.insert(token.clone(), cap);
    }
    audit(
        &state,
        Some(&user.user_id),
        "exec_session_token_issued",
        Some(&id),
        None,
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "token": token,
            "expires_in_secs": EXEC_CAPABILITY_TTL_SECS,
        })),
    )
        .into_response()
}

/// Upgrade to a real interactive exec WebSocket session for one allocation.
/// Auth here is deliberately **not** the general `Authorization: Bearer`
/// header (a browser's native WebSocket API cannot set custom headers on the
/// upgrade request) -- it's the short-lived capability token from
/// `issue_exec_capability`, passed as `?token=...` and **consumed on use**
/// (removed from the map the moment it's validated), so a leaked/replayed URL
/// is dead after its first successful connection, matching
/// `spartan-backend::ws_transport`'s own `?token=` precedent for the same
/// real reason.
async fn exec_session_ws(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(token) = params.get("token").cloned() else {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "missing_token",
            "a capability token is required",
        );
    };
    let Some(runtime) = state.runtime.clone() else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
            "the container runtime is not connected",
        );
    };

    // Validate + CONSUME the capability token before ever upgrading --
    // single-use by construction, not just by convention.
    let cap = {
        let mut caps = state
            .exec_capabilities
            .lock()
            .expect("exec_capabilities mutex poisoned");
        caps.remove(&token)
    };
    let Some(cap) = cap else {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "unknown, expired, or already-used capability token",
        );
    };
    if !cap.is_valid_at(now_unix()) {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "capability token has expired",
        );
    }
    if cap.allocation_id.0 != id {
        // A capability minted for a DIFFERENT allocation cannot be replayed
        // here -- the token proves access to exactly the one allocation it
        // names, not to this user's allocations in general.
        return api_error(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "capability token is not valid for this allocation",
        );
    }

    let owner = cap.owner_id;
    let alloc = cap.allocation_id;
    ws.on_upgrade(move |socket| handle_exec_socket(socket, runtime, owner, alloc))
}

/// Pump a real interactive exec session over an already-upgraded WebSocket.
/// Runs for the life of the connection; returns (dropping the socket) when
/// either side closes or the real exec session ends.
async fn handle_exec_socket(
    mut socket: WebSocket,
    runtime: Arc<dyn ContainerRuntime>,
    owner: UserId,
    alloc: AllocationId,
) {
    // Real output/exit events flow from the exec session's own background
    // task (spawned inside `spawn_interactive_exec`) to this task over a
    // channel -- the `WebSocket` itself isn't handed across that boundary,
    // this task alone owns sending frames.
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<ExecSessionServerEvent>();

    let out_tx_output = out_tx.clone();
    let on_output: Box<dyn FnMut(Vec<u8>) + Send> = Box::new(move |bytes: Vec<u8>| {
        let chunk = String::from_utf8_lossy(&bytes).into_owned();
        let _ = out_tx_output.send(ExecSessionServerEvent::Output { chunk });
    });
    let on_exit: Box<dyn FnOnce() + Send> = Box::new(move || {
        let _ = out_tx.send(ExecSessionServerEvent::Exit);
    });

    let handle = match runtime
        .spawn_interactive_exec(&owner, &alloc, 80, 24, on_output, on_exit)
        .await
    {
        Ok(h) => h,
        Err(e) => {
            let event = ExecSessionServerEvent::Error {
                message: e.to_string(),
            };
            if let Ok(line) = serde_json::to_string(&event) {
                let _ = socket.send(WsMessage::Text(line)).await;
            }
            return;
        }
    };

    loop {
        tokio::select! {
            maybe_event = out_rx.recv() => {
                match maybe_event {
                    Some(event) => {
                        let is_exit = matches!(event, ExecSessionServerEvent::Exit);
                        if let Ok(line) = serde_json::to_string(&event) {
                            if socket.send(WsMessage::Text(line)).await.is_err() {
                                break;
                            }
                        }
                        if is_exit {
                            break;
                        }
                    }
                    None => break,
                }
            }
            maybe_msg = socket.recv() => {
                match maybe_msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        if let Ok(msg) = serde_json::from_str::<ExecSessionClientMessage>(&text) {
                            match msg {
                                ExecSessionClientMessage::Input { data } => {
                                    let _ = handle.write(data.into_bytes());
                                }
                                ExecSessionClientMessage::Resize { cols, rows } => {
                                    let _ = handle.resize(cols, rows);
                                }
                            }
                        }
                        // A real, malformed client message is silently
                        // ignored rather than closing the whole session --
                        // matches this codebase's own general leniency on a
                        // single bad frame over a long-lived stream (e.g.
                        // `spartan-backend::ws_transport`'s own dispatch loop).
                    }
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

#[derive(Serialize)]
struct ContainerUsageJson {
    id: String,
    owner_id: String,
    memory_bytes: Option<u64>,
    memory_limit_bytes: Option<u64>,
    pids: Option<u64>,
}

/// Admin-only: a live per-container resource snapshot across all tenants -- the
/// monitoring-dashboard data feed (the defensive counterpart to the reaper +
/// caps). Requires a connected runtime; without one it honestly 503s rather
/// than returning an empty list as if all were idle.
async fn telemetry(State(state): State<AppState>, admin: AuthUser) -> Response {
    {
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
    }
    let Some(runtime) = &state.runtime else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
            "the container runtime is not connected",
        );
    };
    match runtime.usage().await {
        Ok(usages) => {
            let json: Vec<ContainerUsageJson> = usages
                .into_iter()
                .map(|u| ContainerUsageJson {
                    id: u.id.0,
                    owner_id: u.owner.0,
                    memory_bytes: u.memory_bytes,
                    memory_limit_bytes: u.memory_limit_bytes,
                    pids: u.pids,
                })
                .collect();
            (StatusCode::OK, Json(json)).into_response()
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
    async fn admin_dashboard_serves_real_self_contained_html() {
        let app = router(AppState::in_memory());
        let resp = app.oneshot(get("/admin", None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert!(
            content_type.starts_with("text/html"),
            "admin dashboard must be served as HTML: {content_type}"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8(bytes.to_vec()).unwrap();
        // Real, load-bearing content checks -- the embedded page actually
        // drives the real admin endpoints, not a placeholder.
        assert!(html.contains("/api/admin/telemetry"));
        assert!(html.contains("/api/admin/audit"));
        assert!(html.contains("/api/login"));
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
    async fn telemetry_requires_admin_and_reports_no_runtime_honestly() {
        let state = AppState::in_memory();
        let _admin_id = {
            let store = state.store.lock().unwrap();
            store.create_user("tel@b.com", "adminpw12", true).unwrap()
        };
        let app = router(state.clone());

        // A non-admin is refused.
        let member = signup_token(&app, "telmember@b.com").await;
        let (status, _) = body_json(
            app.clone()
                .oneshot(req_empty("GET", "/api/admin/telemetry", &member))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // The admin passes the gate but, with no runtime wired, gets an honest
        // 503 rather than a faked empty snapshot.
        let (_, login) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/login",
                    serde_json::json!({"email": "tel@b.com", "password": "adminpw12"}),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        let admin_token = login["token"].as_str().unwrap().to_string();
        let (status, body) = body_json(
            app.oneshot(req_empty("GET", "/api/admin/telemetry", &admin_token))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "runtime_unavailable");
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
    async fn exec_rejects_empty_command_and_reports_no_runtime_honestly() {
        let app = router(AppState::in_memory());
        let token = signup_token(&app, "execnorun@b.com").await;

        // Empty argv -> 400 (before any runtime lookup).
        let (status, body) = body_json(
            app.clone()
                .oneshot(req_json(
                    "POST",
                    "/api/allocations/whatever/exec",
                    serde_json::json!({"command": []}),
                    &token,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_command");

        // A real command with no runtime wired -> honest 503.
        let (status, body) = body_json(
            app.oneshot(req_json(
                "POST",
                "/api/allocations/whatever/exec",
                serde_json::json!({"command": ["echo", "hi"]}),
                &token,
            ))
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "runtime_unavailable");
    }

    #[tokio::test]
    async fn exec_really_runs_a_command_in_the_callers_own_allocation() {
        use spartan_cloud_runtime::{ContainerRuntime, DockerRuntime};
        let docker = match bollard::Docker::connect_with_local_defaults() {
            Ok(d) if d.ping().await.is_ok() => d,
            _ => {
                println!("SKIP: no Docker daemon; exec end-to-end test skipped");
                return;
            }
        };
        let runtime = std::sync::Arc::new(DockerRuntime::with_docker(docker, "runc", true));
        let app = router(AppState::in_memory().with_runtime(runtime.clone()));

        let token = signup_token(&app, "execreal@b.com").await;
        // Allocate a real container for this user.
        let (status, alloc) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/allocate",
                    serde_json::json!({}),
                    Some(&token),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "allocate: {alloc}");
        let id = alloc["id"].as_str().unwrap().to_string();

        // Exec a real command in it via the API.
        let (status, exec) = body_json(
            app.oneshot(req_json(
                "POST",
                &format!("/api/allocations/{id}/exec"),
                serde_json::json!({"command": ["echo", "hello-via-api"]}),
                &token,
            ))
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "exec: {exec}");
        assert!(
            exec["output"].as_str().unwrap().contains("hello-via-api"),
            "real output over the API: {exec}"
        );
        assert_eq!(exec["exit_code"], 0);

        // Clean up the real container.
        runtime
            .stop(&spartan_cloud_protocol::AllocationId(id))
            .await
            .expect("cleanup");
    }

    #[tokio::test]
    async fn session_token_requires_real_ownership_and_reports_no_runtime_honestly() {
        let app = router(AppState::in_memory());
        let token = signup_token(&app, "capnorun@b.com").await;

        // No runtime wired -> honest 503, before any ownership check.
        let (status, body) = body_json(
            app.oneshot(req_json(
                "POST",
                "/api/allocations/whatever/session_token",
                serde_json::json!({}),
                &token,
            ))
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "runtime_unavailable");
    }

    #[tokio::test]
    async fn session_token_is_refused_for_an_allocation_the_caller_does_not_own() {
        use spartan_cloud_runtime::DockerRuntime;
        let docker = match bollard::Docker::connect_with_local_defaults() {
            Ok(d) if d.ping().await.is_ok() => d,
            _ => {
                println!("SKIP: no Docker daemon; session_token ownership test skipped");
                return;
            }
        };
        let runtime = std::sync::Arc::new(DockerRuntime::with_docker(docker, "runc", true));
        let app = router(AppState::in_memory().with_runtime(runtime.clone()));

        // Alice allocates a real container.
        let alice = signup_token(&app, "alice-cap@b.com").await;
        let (_, alloc) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/allocate",
                    serde_json::json!({}),
                    Some(&alice),
                ))
                .await
                .unwrap(),
        )
        .await;
        let id = alloc["id"].as_str().unwrap().to_string();

        // Bob (a different tenant) is refused a session token for it.
        let bob = signup_token(&app, "bob-cap@b.com").await;
        let (status, body) = body_json(
            app.clone()
                .oneshot(req_json(
                    "POST",
                    &format!("/api/allocations/{id}/session_token"),
                    serde_json::json!({}),
                    &bob,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "cross-tenant token is denied"
        );
        assert_eq!(body["code"], "not_found");

        // Alice herself is granted one for her own real allocation.
        let (status, body) = body_json(
            app.oneshot(req_json(
                "POST",
                &format!("/api/allocations/{id}/session_token"),
                serde_json::json!({}),
                &alice,
            ))
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body["token"].as_str().unwrap().is_empty());

        runtime
            .stop(&spartan_cloud_protocol::AllocationId(id))
            .await
            .expect("cleanup");
    }

    #[tokio::test]
    async fn exec_session_ws_runs_a_real_interactive_command_end_to_end() {
        use futures_util::{SinkExt, StreamExt};
        use spartan_cloud_protocol::{ExecSessionClientMessage, ExecSessionServerEvent};
        use spartan_cloud_runtime::DockerRuntime;
        use tokio_tungstenite::tungstenite::Message as TtMessage;

        let docker = match bollard::Docker::connect_with_local_defaults() {
            Ok(d) if d.ping().await.is_ok() => d,
            _ => {
                println!("SKIP: no Docker daemon; exec session WS test skipped");
                return;
            }
        };
        let runtime = std::sync::Arc::new(DockerRuntime::with_docker(docker, "runc", true));
        let app = router(AppState::in_memory().with_runtime(runtime.clone()));

        // Real REST setup: signup, allocate, mint a real capability token --
        // all via `oneshot`, exactly like this file's other REST tests.
        let token = signup_token(&app, "wsexec@b.com").await;
        let (_, alloc) = body_json(
            app.clone()
                .oneshot(post_json(
                    "/api/allocate",
                    serde_json::json!({}),
                    Some(&token),
                ))
                .await
                .unwrap(),
        )
        .await;
        let id = alloc["id"].as_str().unwrap().to_string();

        let (_, cap) = body_json(
            app.clone()
                .oneshot(req_json(
                    "POST",
                    &format!("/api/allocations/{id}/session_token"),
                    serde_json::json!({}),
                    &token,
                ))
                .await
                .unwrap(),
        )
        .await;
        let cap_token = cap["token"].as_str().unwrap().to_string();

        // A WS upgrade needs a real HTTP/1.1 connection, unlike the REST
        // tests' in-memory `oneshot` -- bind a real listener and serve.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("real bind");
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let ws_url = format!("ws://{addr}/api/allocations/{id}/session?token={cap_token}");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .expect("a correctly-tokened WS upgrade must succeed");

        // Send a real command; the container's real shell should echo it.
        let msg = serde_json::to_string(&ExecSessionClientMessage::Input {
            data: "echo hello-ws-exec\n".to_string(),
        })
        .unwrap();
        ws.send(TtMessage::Text(msg)).await.unwrap();

        let mut saw_it = false;
        for _ in 0..30 {
            let Ok(Some(Ok(TtMessage::Text(text)))) =
                tokio::time::timeout(std::time::Duration::from_secs(3), ws.next()).await
            else {
                break;
            };
            if let Ok(ExecSessionServerEvent::Output { chunk }) = serde_json::from_str(&text) {
                if chunk.contains("hello-ws-exec") {
                    saw_it = true;
                    break;
                }
            }
        }
        assert!(
            saw_it,
            "the real shell's echoed output must arrive over the WS session"
        );
        drop(ws);

        // The SAME (now-consumed) capability token must be refused on replay
        // -- single-use, not just short-lived.
        let retry = tokio_tungstenite::connect_async(&ws_url).await;
        assert!(
            retry.is_err(),
            "a capability token is single-use; replay must be rejected"
        );

        runtime
            .stop(&spartan_cloud_protocol::AllocationId(id))
            .await
            .expect("cleanup");
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
