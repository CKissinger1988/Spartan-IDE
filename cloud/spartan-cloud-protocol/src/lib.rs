//! Shared, serializable DTOs for Spartan Cloud's control plane -- the wire
//! vocabulary the (future) axum API and any client both speak. Pure data +
//! serde, deliberately with no logic and no HTTP/DB/Docker dependency, so
//! either side can depend on it without pulling in the other's guts.
//!
//! This is a genuinely new set of types, NOT a re-export of
//! `spartan-backend`'s own single-process-scoped `Request`/`Response` --
//! those describe one local IPC session, while these describe a multi-tenant
//! HTTP+WS control plane. The *per-container* streaming channel (a later
//! increment) will reuse `spartan-backend`'s `{id,method,params}` /
//! `{event,data}` envelope *shape* over its own WebSocket, but that is
//! prior-art reuse of a proven shape, not a type import.

use serde::{Deserialize, Serialize};

/// Opaque, stable identifier for a user account. A string (not an integer)
/// so the data layer is free to use UUIDs, ULIDs, etc. without a protocol
/// change.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
pub struct UserId(pub String);

/// An opaque, server-issued, DB-stored session token. **Deliberately not a
/// JWT** -- a JWT can't be revoked before its natural expiry, which conflicts
/// directly with this service's abuse/cost-control needs (a compromised or
/// abusive account must be killable *immediately*). Being an opaque handle
/// the server looks up, revocation is just deleting the stored row.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
pub struct SessionToken(pub String);

/// Identifier for one allocated container/workspace belonging to a user.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
pub struct AllocationId(pub String);

/// The subscription tier that governs a user's quotas. Billing is deferred
/// (see `spartan-cloud-tenant`'s `EntitlementProvider`); this enum is the
/// stable vocabulary a real billing integration will eventually drive.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlanTier {
    /// No active subscription -- minimal quotas, enough to evaluate.
    Free,
    /// A paying subscriber.
    Pro,
}

impl PlanTier {
    /// A stable, lowercase-free display name (matches the serde variant name),
    /// handy for logs/audit records without pulling in a serde round trip.
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanTier::Free => "Free",
            PlanTier::Pro => "Pro",
        }
    }
}

/// Lifecycle state of an allocated container, as reported to a client.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum AllocationStatus {
    Provisioning,
    Running,
    Stopping,
    Stopped,
    Failed,
}

// ---- Control-plane request/response DTOs ----

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SignupRequest {
    pub email: String,
    pub password: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Returned on a successful signup/login. The token is opaque (see
/// `SessionToken`); `expires_at_unix` lets a client pre-empt expiry, but the
/// server is always the authority (and can revoke earlier).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AuthResponse {
    pub user_id: UserId,
    pub token: SessionToken,
    pub expires_at_unix: u64,
}

/// Request to allocate a new build/test container. `image` is optional --
/// the server picks a safe default when absent; a client-supplied image is
/// still subject to the server's own allowlist/policy (enforced server-side,
/// never trusted from here).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AllocateRequest {
    pub image: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AllocationInfo {
    pub id: AllocationId,
    pub status: AllocationStatus,
    /// When this allocation will be reaped regardless of activity -- the hard
    /// wall-clock lifetime cap that answers §36.4.7's "uncapped consumption".
    pub expires_at_unix: u64,
}

/// Store (create or overwrite) one of the caller's own encrypted secrets.
/// The value is a UTF-8 string (deploy keys, registry tokens, capability
/// tokens are all text); it is encrypted at rest server-side and **never**
/// read back over the API -- secrets go in and are used server-side, they are
/// not a retrieval store (a deliberate exposure-reducing choice).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PutSecretRequest {
    pub value: String,
}

/// The caller's own secret *names* (never values).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SecretNamesResponse {
    pub names: Vec<String>,
}

/// A uniform error envelope for control-plane responses.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}
