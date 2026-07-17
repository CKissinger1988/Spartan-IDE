//! Opaque, server-issued sessions. The token itself is a random 256-bit
//! handle (hex) with no embedded claims -- the server looks it up in the
//! data layer to find the user and validity. This is the deliberate choice
//! over JWTs (see `SessionToken`'s own doc in the protocol crate):
//! revocation is just deleting the stored row, which a JWT can't do before
//! its natural expiry -- a hard requirement for abuse/cost control.
//!
//! This module owns token *generation* and expiry *math* only. Persistence
//! and revocation live in the (later) data layer; a `Session` value here is
//! what gets stored and what a lookup returns.

use spartan_cloud_protocol::{SessionToken, UserId};

/// A real, cryptographically-random 256-bit opaque token, hex-encoded. Fresh
/// per issue; never derived from user data, so it leaks nothing and collides
/// with negligible probability.
pub fn new_opaque_token() -> SessionToken {
    let bytes: [u8; 32] = rand::random();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    SessionToken(hex)
}

/// A stored session: which user, issued when, valid until. The server is
/// always the authority -- a session can also be revoked early by deleting
/// it from the store, independent of `expires_at_unix`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub token: SessionToken,
    pub user_id: UserId,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
}

impl Session {
    /// Issue a fresh session for `user_id`, valid for `ttl_secs` from
    /// `now_unix`. `now_unix` is passed in (not read from the clock here) so
    /// the logic stays pure and testable.
    pub fn issue(user_id: UserId, now_unix: u64, ttl_secs: u64) -> Self {
        Session {
            token: new_opaque_token(),
            user_id,
            created_at_unix: now_unix,
            expires_at_unix: now_unix.saturating_add(ttl_secs),
        }
    }

    /// Whether this session is still within its lifetime at `now_unix`.
    /// (Early revocation is a separate, store-level concern -- a revoked
    /// session simply won't be found by a lookup at all.)
    pub fn is_valid_at(&self, now_unix: u64) -> bool {
        now_unix < self.expires_at_unix
    }

    /// Seconds until expiry at `now_unix`, saturating to 0 once expired.
    pub fn remaining_secs(&self, now_unix: u64) -> u64 {
        self.expires_at_unix.saturating_sub(now_unix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_64_hex_chars_and_unique_per_issue() {
        let a = new_opaque_token();
        let b = new_opaque_token();
        assert_eq!(a.0.len(), 64);
        assert!(a.0.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "two issued tokens must differ");
    }

    #[test]
    fn a_session_is_valid_before_expiry_and_invalid_after() {
        let s = Session::issue(UserId("bob".into()), 1_000, 3_600);
        assert_eq!(s.created_at_unix, 1_000);
        assert_eq!(s.expires_at_unix, 4_600);
        assert!(s.is_valid_at(1_000), "valid at issue time");
        assert!(s.is_valid_at(4_599), "valid one second before expiry");
        assert!(!s.is_valid_at(4_600), "invalid exactly at expiry");
        assert!(!s.is_valid_at(9_999), "invalid well after expiry");
    }

    #[test]
    fn remaining_secs_counts_down_and_saturates_at_zero() {
        let s = Session::issue(UserId("bob".into()), 1_000, 3_600);
        assert_eq!(s.remaining_secs(1_000), 3_600);
        assert_eq!(s.remaining_secs(4_000), 600);
        assert_eq!(s.remaining_secs(4_600), 0);
        assert_eq!(s.remaining_secs(10_000), 0, "never goes negative");
    }

    #[test]
    fn issue_saturates_rather_than_overflowing_on_a_huge_ttl() {
        let s = Session::issue(UserId("bob".into()), u64::MAX - 1, 100);
        assert_eq!(s.expires_at_unix, u64::MAX, "saturates, no panic");
    }
}
