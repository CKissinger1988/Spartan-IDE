//! Shared, serializable DTOs for Spartan Cloud's control plane -- the wire
//! vocabulary the (future) axum API and any client both speak. Pure data +
//! serde, deliberately with no logic and no HTTP/DB/Docker dependency, so
//! either side can depend on it without pulling in the other's guts.
//!
//! This is a genuinely new set of types, NOT a re-export of
//! `spartan-backend`'s own single-process-scoped `Request`/`Response` --
//! those describe one local IPC session, while these describe a multi-tenant
//! HTTP+WS control plane. The *per-container* streaming exec channel
//! (`ExecSessionClientMessage`/`ExecSessionServerEvent`, below) reuses
//! `spartan-backend`'s own real idea of a server sending unprompted named
//! events over a WebSocket (its `pty_output`/`pty_exit`), not its literal
//! wire strings or any type import -- prior-art reuse of a proven idea,
//! expressed as its own real, independently-typed enums.

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

/// Run a one-shot command inside one of the caller's own allocations. The
/// command is an explicit argv (`["npm", "test"]`), never a shell string, so
/// there's no shell-injection surface at this layer.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ExecRequest {
    pub command: Vec<String>,
}

/// The result of an `ExecRequest`: combined stdout+stderr and the real exit
/// code (`null` if the daemon didn't report one).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ExecResponse {
    pub output: String,
    pub exit_code: Option<i64>,
}

/// A short-lived, single-allocation-scoped opaque capability token for the
/// per-container interactive WebSocket exec session. Deliberately a
/// **distinct type** from `SessionToken` (the general login/bearer token)
/// even though the wire shape is identical (random hex) -- so the two can
/// never be type-punned at a call site: this proves "you may open exactly
/// this one WS session for this one allocation, once," not "you are logged
/// in as this user." A leaked container-session URL therefore can't be
/// replayed against any other endpoint, and expires quickly since it only
/// needs to survive the moment between issuing it and opening the WS.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
pub struct CapabilityToken(pub String);

/// Client -> server message on a per-container interactive exec WebSocket
/// session. Real, typed variants -- every message on this channel is
/// fire-and-forget (there is no request/response correlation to preserve,
/// unlike an RPC envelope with an `id`; this channel behaves exactly like a
/// terminal's own stdin stream, not a call-and-response API).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum ExecSessionClientMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}

/// Server -> client message on the same channel -- real, unprompted output as
/// it streams from the container, or the session ending. Reuses
/// `spartan-backend`'s own real idea that a server sends unprompted, named
/// events (its `pty_output`/`pty_exit`) -- not its literal wire strings or any
/// type import (`cloud/` deliberately has no dependency on `spartan-backend`,
/// keeping this workspace genuinely separate and portable) -- expressed here
/// as its own real, independently-typed, fully-checked enum instead of a raw
/// `{event, data: Value}` bag.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum ExecSessionServerEvent {
    Output { chunk: String },
    Exit,
    Error { message: String },
}

/// A uniform error envelope for control-plane responses.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_session_client_message_round_trips_over_the_typed_tagged_shape() {
        let input = ExecSessionClientMessage::Input {
            data: "echo hi\n".to_string(),
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["method"], "input");
        assert_eq!(json["params"]["data"], "echo hi\n");
        let round_tripped: ExecSessionClientMessage = serde_json::from_value(json).unwrap();
        assert!(
            matches!(round_tripped, ExecSessionClientMessage::Input { data } if data == "echo hi\n")
        );

        let resize = ExecSessionClientMessage::Resize {
            cols: 120,
            rows: 40,
        };
        let json = serde_json::to_value(&resize).unwrap();
        assert_eq!(json["method"], "resize");
        assert_eq!(json["params"]["cols"], 120);
        assert_eq!(json["params"]["rows"], 40);
    }

    #[test]
    fn exec_session_server_event_round_trips_including_the_unit_exit_variant() {
        let output = ExecSessionServerEvent::Output {
            chunk: "hello".to_string(),
        };
        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["event"], "output");
        assert_eq!(json["data"]["chunk"], "hello");

        let exit = ExecSessionServerEvent::Exit;
        let json = serde_json::to_value(&exit).unwrap();
        assert_eq!(json["event"], "exit");
        let round_tripped: ExecSessionServerEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(round_tripped, ExecSessionServerEvent::Exit));

        let error = ExecSessionServerEvent::Error {
            message: "boom".to_string(),
        };
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["event"], "error");
        assert_eq!(json["data"]["message"], "boom");
    }
}
