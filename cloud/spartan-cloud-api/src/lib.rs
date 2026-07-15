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

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use spartan_cloud_data::{DataError, Store};
use spartan_cloud_protocol::{
    ApiError, AuthResponse, LoginRequest, PlanTier, SessionToken, SignupRequest, UserId,
};
use spartan_cloud_tenant::{
    can_allocate, EntitlementProvider, PlanLimits, Session, StubEntitlementProvider,
};

/// Shared server state. The SQLite `Store` isn't `Sync`, so it's behind a
/// `Mutex`; handlers lock it only for the brief synchronous DB op and never
/// hold the guard across an `.await`.
#[derive(Clone)]
pub struct AppState {
    store: Arc<Mutex<Store>>,
    entitlements: Arc<StubEntitlementProvider>,
    session_ttl_secs: u64,
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
        }
    }

    /// A ready-to-serve in-memory state (tests, ephemeral runs).
    pub fn in_memory() -> Self {
        Self::new(
            Store::open_in_memory().expect("in-memory store must open"),
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
        .route("/api/allocate", post(allocate))
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
        Ok(user_id) => match issue_and_store_session(&state, user_id) {
            Ok(auth) => (StatusCode::OK, Json(auth)).into_response(),
            Err(e) => internal_error(e),
        },
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
        Ok(Some(user_id)) => match issue_and_store_session(&state, user_id) {
            Ok(auth) => (StatusCode::OK, Json(auth)).into_response(),
            Err(e) => internal_error(e),
        },
        // Wrong password AND unknown email both land here -- no enumeration.
        Ok(None) => api_error(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "incorrect email or password",
        ),
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
    state.entitlements.grant_pro(UserId(req.user_id));
    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

async fn allocate(State(state): State<AppState>, user: AuthUser) -> Response {
    // Real admission: entitlement -> effective tier -> plan limits -> quota.
    let ent = state.entitlements.check(&user.user_id);
    let limits = PlanLimits::for_tier(ent.effective_tier());

    // No runtime yet, so the current active-container count is 0. Once
    // `spartan-cloud-runtime` lands, this count comes from the real runtime.
    let current_active = 0;
    if let Err(quota_err) = can_allocate(&limits, current_active) {
        return api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "quota_exceeded",
            &quota_err.to_string(),
        );
    }

    // Admission passed -- but honestly report that the container runtime
    // isn't connected yet rather than faking an allocation.
    api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "runtime_unavailable",
        "admission passed, but the container runtime is not connected yet",
    )
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
