//! Short-lived, single-allocation-scoped capability tokens for the
//! per-container interactive WebSocket exec session -- deliberately separate
//! from the general login `Session` (see `CapabilityToken`'s own doc in the
//! protocol crate for why), so a leaked container-session URL can't be
//! replayed against any other endpoint, and expires quickly since it only
//! needs to survive the moment between issuing it and opening the real WS.
//!
//! Pure domain logic only, mirroring `session.rs`'s own split -- token
//! generation and expiry math live here; the API layer owns the actual
//! (in-memory, single-use) storage of issued capabilities.

use spartan_cloud_protocol::{AllocationId, CapabilityToken, UserId};

/// A real, cryptographically-random 256-bit opaque token, hex-encoded --
/// the same real generation shape `session.rs::new_opaque_token` uses, kept
/// as its own small function (not shared) since `CapabilityToken` and
/// `SessionToken` are deliberately distinct types that must never be
/// type-punned at a call site.
pub fn new_capability_token() -> CapabilityToken {
    let bytes: [u8; 32] = rand::random();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    CapabilityToken(hex)
}

/// A stored capability: which user may use it, for which single allocation,
/// and until when. The server is always the authority -- issuing it doesn't
/// itself re-verify allocation ownership (the API layer checks that once, at
/// issue time, via the runtime's own owner-scoped listing); this struct just
/// carries the already-verified grant forward to the WS handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecCapability {
    pub token: CapabilityToken,
    pub owner_id: UserId,
    pub allocation_id: AllocationId,
    pub expires_at_unix: u64,
}

impl ExecCapability {
    /// Issue a fresh capability for `owner_id` on `allocation_id`, valid for
    /// `ttl_secs` from `now_unix`. `now_unix` is passed in (not read from the
    /// clock here) so the logic stays pure and testable, matching
    /// `Session::issue`'s own precedent exactly.
    pub fn issue(
        owner_id: UserId,
        allocation_id: AllocationId,
        now_unix: u64,
        ttl_secs: u64,
    ) -> Self {
        Self {
            token: new_capability_token(),
            owner_id,
            allocation_id,
            expires_at_unix: now_unix.saturating_add(ttl_secs),
        }
    }

    /// Whether this capability is still within its lifetime at `now_unix`.
    pub fn is_valid_at(&self, now_unix: u64) -> bool {
        now_unix < self.expires_at_unix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_64_hex_chars_and_unique_per_issue() {
        let a = new_capability_token();
        let b = new_capability_token();
        assert_eq!(a.0.len(), 64);
        assert!(a.0.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "two issued tokens must differ");
    }

    #[test]
    fn a_capability_is_valid_before_expiry_and_invalid_after() {
        let cap = ExecCapability::issue(
            UserId("alice".into()),
            AllocationId("alloc-1".into()),
            1_000,
            60,
        );
        assert_eq!(cap.expires_at_unix, 1_060);
        assert!(cap.is_valid_at(1_000), "valid at issue time");
        assert!(cap.is_valid_at(1_059), "valid one second before expiry");
        assert!(!cap.is_valid_at(1_060), "invalid exactly at expiry");
        assert!(!cap.is_valid_at(9_999), "invalid well after expiry");
    }

    #[test]
    fn issue_saturates_rather_than_overflowing_on_a_huge_ttl() {
        let cap = ExecCapability::issue(
            UserId("alice".into()),
            AllocationId("alloc-1".into()),
            u64::MAX - 1,
            100,
        );
        assert_eq!(cap.expires_at_unix, u64::MAX, "saturates, no panic");
    }

    #[test]
    fn a_capability_carries_the_exact_owner_and_allocation_it_was_issued_for() {
        let owner = UserId("bob".into());
        let alloc = AllocationId("alloc-9".into());
        let cap = ExecCapability::issue(owner.clone(), alloc.clone(), 0, 60);
        assert_eq!(cap.owner_id, owner);
        assert_eq!(cap.allocation_id, alloc);
    }
}
