//! Persistence for Spartan Cloud's control plane: users (with real argon2
//! password hashing) and sessions (opaque tokens whose *revocation* is a row
//! delete -- the concrete reason sessions are DB handles, not JWTs).
//!
//! SQLite for the MVP (embedded via `rusqlite`'s `bundled` SQLite, zero infra
//! to stand up or test), sync/blocking to match this workspace's convention.
//! Swappable to Postgres later -- every query here is plain SQL behind the
//! `Store` API, no SQLite-specific surface leaks to callers.
//!
//! Deliberately **not** here: the argon2 *parameters* are the crate defaults
//! (a sensible, widely-used baseline); tuning them for a specific deployment
//! is a real, named ops decision, not hardcoded cleverness.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rusqlite::{params, Connection, OptionalExtension};

use spartan_cloud_protocol::{SessionToken, UserId};
use spartan_cloud_tenant::Session;

#[derive(Debug)]
pub enum DataError {
    Sqlite(rusqlite::Error),
    /// An argon2 hashing/verification failure (not a wrong password -- that's
    /// a normal `Ok(None)` from `verify_login` -- but a genuine crypto error).
    Hash(String),
    /// The email is already registered. Surfaced distinctly so the API can
    /// return a clean 409 rather than a raw constraint-violation string.
    EmailTaken,
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataError::Sqlite(e) => write!(f, "database error: {e}"),
            DataError::Hash(e) => write!(f, "password hashing error: {e}"),
            DataError::EmailTaken => write!(f, "email is already registered"),
        }
    }
}

impl std::error::Error for DataError {}

impl From<rusqlite::Error> for DataError {
    fn from(e: rusqlite::Error) -> Self {
        DataError::Sqlite(e)
    }
}

/// A real, opaque 128-bit user id, hex-encoded. Random (not derived from the
/// email) so it leaks nothing about the account.
fn new_user_id() -> UserId {
    let bytes: [u8; 16] = rand::random();
    UserId(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The persistence handle. Owns one SQLite connection; the (later) API layer
/// wraps it in whatever sharing it needs (e.g. a connection pool or a mutex).
pub struct Store {
    conn: Connection,
}

impl Store {
    /// An in-memory database -- used by tests and ephemeral runs. Schema is
    /// created immediately.
    pub fn open_in_memory() -> Result<Self, DataError> {
        let conn = Connection::open_in_memory()?;
        let store = Store { conn };
        store.init_schema()?;
        Ok(store)
    }

    /// A real on-disk database at `path`, creating it (and the schema) if
    /// absent.
    pub fn open(path: &str) -> Result<Self, DataError> {
        let conn = Connection::open(path)?;
        let store = Store { conn };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), DataError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (
                id            TEXT PRIMARY KEY,
                email         TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                is_admin      INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS sessions (
                token           TEXT PRIMARY KEY,
                user_id         TEXT NOT NULL,
                created_at_unix INTEGER NOT NULL,
                expires_at_unix INTEGER NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id)
             );",
        )?;
        Ok(())
    }

    /// Create a user with an argon2-hashed password. Returns the new opaque
    /// `UserId`. A duplicate email is a clean `DataError::EmailTaken`, not a
    /// raw constraint error.
    pub fn create_user(
        &self,
        email: &str,
        password: &str,
        is_admin: bool,
    ) -> Result<UserId, DataError> {
        // A real 128-bit random salt from the `rand` crate we already depend
        // on -- encoded into the B64 form `SaltString` wants. This avoids
        // pulling argon2's optional `getrandom`-gated `OsRng` re-export for
        // one salt, while still being cryptographically random per hash.
        let salt_bytes: [u8; 16] = rand::random();
        let salt =
            SaltString::encode_b64(&salt_bytes).map_err(|e| DataError::Hash(e.to_string()))?;
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| DataError::Hash(e.to_string()))?
            .to_string();

        let id = new_user_id();
        let result = self.conn.execute(
            "INSERT INTO users (id, email, password_hash, is_admin) VALUES (?1, ?2, ?3, ?4)",
            params![id.0, email, hash, is_admin as i64],
        );
        match result {
            Ok(_) => Ok(id),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(DataError::EmailTaken)
            }
            Err(e) => Err(DataError::Sqlite(e)),
        }
    }

    /// Verify a login. Returns `Some(UserId)` only on a real password match;
    /// `Ok(None)` for both "no such email" and "wrong password" (deliberately
    /// indistinguishable, so the API can't leak which emails are registered).
    pub fn verify_login(&self, email: &str, password: &str) -> Result<Option<UserId>, DataError> {
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT id, password_hash FROM users WHERE email = ?1",
                params![email],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;

        let Some((id, stored_hash)) = row else {
            return Ok(None);
        };
        let parsed = PasswordHash::new(&stored_hash).map_err(|e| DataError::Hash(e.to_string()))?;
        if Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
        {
            Ok(Some(UserId(id)))
        } else {
            Ok(None)
        }
    }

    /// Whether a user has the admin flag (bootstraps the admin-only
    /// entitlement-toggle endpoint).
    pub fn is_admin(&self, user_id: &UserId) -> Result<bool, DataError> {
        let flag: Option<i64> = self
            .conn
            .query_row(
                "SELECT is_admin FROM users WHERE id = ?1",
                params![user_id.0],
                |r| r.get(0),
            )
            .optional()?;
        Ok(flag.unwrap_or(0) != 0)
    }

    /// Persist an issued session. (Token generation + expiry math live in
    /// `spartan-cloud-tenant`; this just stores the result.)
    pub fn store_session(&self, session: &Session) -> Result<(), DataError> {
        self.conn.execute(
            "INSERT INTO sessions (token, user_id, created_at_unix, expires_at_unix)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                session.token.0,
                session.user_id.0,
                session.created_at_unix as i64,
                session.expires_at_unix as i64,
            ],
        )?;
        Ok(())
    }

    /// Look up a session by token, returning it only if it both **exists**
    /// (not revoked) and is **unexpired** at `now_unix`. This is where opaque
    /// tokens earn their keep: a revoked (deleted) row simply isn't found.
    pub fn lookup_session(
        &self,
        token: &SessionToken,
        now_unix: u64,
    ) -> Result<Option<Session>, DataError> {
        let row: Option<(String, i64, i64)> = self
            .conn
            .query_row(
                "SELECT user_id, created_at_unix, expires_at_unix FROM sessions WHERE token = ?1",
                params![token.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;

        let Some((user_id, created, expires)) = row else {
            return Ok(None);
        };
        let session = Session {
            token: token.clone(),
            user_id: UserId(user_id),
            created_at_unix: created as u64,
            expires_at_unix: expires as u64,
        };
        if session.is_valid_at(now_unix) {
            Ok(Some(session))
        } else {
            Ok(None)
        }
    }

    /// Revoke a session immediately (delete the row). Idempotent -- revoking
    /// an unknown/already-gone token is a harmless no-op, not an error.
    pub fn revoke_session(&self, token: &SessionToken) -> Result<(), DataError> {
        self.conn
            .execute("DELETE FROM sessions WHERE token = ?1", params![token.0])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_user_then_verify_correct_and_wrong_passwords() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .create_user("alice@example.com", "correct horse", false)
            .unwrap();

        let ok = store
            .verify_login("alice@example.com", "correct horse")
            .unwrap();
        assert_eq!(ok, Some(id.clone()), "correct password logs in as the user");

        let wrong = store
            .verify_login("alice@example.com", "wrong password")
            .unwrap();
        assert_eq!(wrong, None, "wrong password never authenticates");

        let unknown = store.verify_login("nobody@example.com", "x").unwrap();
        assert_eq!(
            unknown, None,
            "unknown email is indistinguishable from wrong pw"
        );
    }

    #[test]
    fn duplicate_email_is_a_clean_email_taken_error() {
        let store = Store::open_in_memory().unwrap();
        store.create_user("bob@example.com", "pw1", false).unwrap();
        let again = store.create_user("bob@example.com", "pw2", false);
        assert!(
            matches!(again, Err(DataError::EmailTaken)),
            "a duplicate email is EmailTaken, not a raw constraint error: {again:?}"
        );
    }

    #[test]
    fn admin_flag_round_trips() {
        let store = Store::open_in_memory().unwrap();
        let admin = store.create_user("admin@example.com", "pw", true).unwrap();
        let normal = store.create_user("user@example.com", "pw", false).unwrap();
        assert!(store.is_admin(&admin).unwrap());
        assert!(!store.is_admin(&normal).unwrap());
    }

    #[test]
    fn password_hash_is_never_stored_in_plaintext() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_user("carol@example.com", "super secret pw", false)
            .unwrap();
        let stored: String = store
            .conn
            .query_row(
                "SELECT password_hash FROM users WHERE email = 'carol@example.com'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !stored.contains("super secret pw"),
            "the plaintext password must never appear in storage"
        );
        assert!(
            stored.starts_with("$argon2"),
            "a real argon2 PHC-format hash is stored: {stored}"
        );
    }

    #[test]
    fn session_store_lookup_expiry_and_revocation() {
        let store = Store::open_in_memory().unwrap();
        let uid = store.create_user("dave@example.com", "pw", false).unwrap();
        let session = Session::issue(uid.clone(), 1_000, 3_600); // expires at 4_600
        store.store_session(&session).unwrap();

        // Valid within lifetime.
        let found = store.lookup_session(&session.token, 2_000).unwrap();
        assert_eq!(found.as_ref().map(|s| &s.user_id), Some(&uid));

        // Expired: not returned even though the row still exists.
        let expired = store.lookup_session(&session.token, 5_000).unwrap();
        assert_eq!(expired, None, "an expired session is not returned");

        // Revoked: gone immediately, before natural expiry (the JWT-killer).
        store.revoke_session(&session.token).unwrap();
        let after_revoke = store.lookup_session(&session.token, 2_000).unwrap();
        assert_eq!(after_revoke, None, "a revoked session is gone at once");

        // Revoking again is a harmless no-op.
        assert!(store.revoke_session(&session.token).is_ok());
    }

    #[test]
    fn an_unknown_session_token_looks_up_to_none() {
        let store = Store::open_in_memory().unwrap();
        let none = store
            .lookup_session(&SessionToken("deadbeef".into()), 1)
            .unwrap();
        assert_eq!(none, None);
    }
}
