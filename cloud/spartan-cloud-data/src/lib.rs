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

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
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
    /// A vault operation was attempted but no master key is configured (the
    /// store was opened without one). The secret is *not* silently stored in
    /// plaintext -- the operation is refused.
    VaultLocked,
    /// AES-GCM encryption/decryption failed. On decrypt this specifically also
    /// covers a failed authentication tag -- i.e. tampered or wrong-key
    /// ciphertext -- which is exactly what GCM (unlike the reference's CBC)
    /// detects. Never carries the underlying detail (no oracle).
    Crypto,
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataError::Sqlite(e) => write!(f, "database error: {e}"),
            DataError::Hash(e) => write!(f, "password hashing error: {e}"),
            DataError::EmailTaken => write!(f, "email is already registered"),
            DataError::VaultLocked => {
                write!(f, "the secrets vault is locked (no master key configured)")
            }
            DataError::Crypto => write!(f, "secret encryption/decryption failed"),
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
///
/// `cipher` is the at-rest secrets-vault key, set only when a master key was
/// provided at open time. When `None`, vault operations are refused
/// (`DataError::VaultLocked`) rather than silently storing plaintext.
pub struct Store {
    conn: Connection,
    cipher: Option<Aes256Gcm>,
}

impl Store {
    /// An in-memory database -- used by tests and ephemeral runs. Schema is
    /// created immediately. No vault key (vault operations are refused).
    pub fn open_in_memory() -> Result<Self, DataError> {
        let conn = Connection::open_in_memory()?;
        let store = Store { conn, cipher: None };
        store.init_schema()?;
        Ok(store)
    }

    /// An in-memory database with a vault master key (tests, ephemeral runs).
    pub fn open_in_memory_with_key(master_key: &[u8; 32]) -> Result<Self, DataError> {
        let conn = Connection::open_in_memory()?;
        let store = Store {
            conn,
            cipher: Some(Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(master_key))),
        };
        store.init_schema()?;
        Ok(store)
    }

    /// A real on-disk database at `path`, creating it (and the schema) if
    /// absent. No vault key.
    pub fn open(path: &str) -> Result<Self, DataError> {
        let conn = Connection::open(path)?;
        let store = Store { conn, cipher: None };
        store.init_schema()?;
        Ok(store)
    }

    /// A real on-disk database with a vault master key. The key comes from the
    /// operator's environment and is never written to the database -- only the
    /// per-record nonce + ciphertext are persisted.
    pub fn open_with_key(path: &str, master_key: &[u8; 32]) -> Result<Self, DataError> {
        let conn = Connection::open(path)?;
        let store = Store {
            conn,
            cipher: Some(Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(master_key))),
        };
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
             );
             CREATE TABLE IF NOT EXISTS audit_log (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                at_unix      INTEGER NOT NULL,
                actor_id     TEXT,
                action       TEXT NOT NULL,
                target       TEXT,
                detail       TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_audit_actor ON audit_log(actor_id);
             CREATE TABLE IF NOT EXISTS secrets (
                owner_id   TEXT NOT NULL,
                name       TEXT NOT NULL,
                nonce      BLOB NOT NULL,
                ciphertext BLOB NOT NULL,
                PRIMARY KEY (owner_id, name)
             );
             CREATE TABLE IF NOT EXISTS webauthn_credentials (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                owner_id     TEXT NOT NULL,
                passkey_json TEXT NOT NULL,
                FOREIGN KEY(owner_id) REFERENCES users(id)
             );
             CREATE INDEX IF NOT EXISTS idx_webauthn_owner ON webauthn_credentials(owner_id);",
        )?;
        Ok(())
    }

    /// Store a newly-registered WebAuthn credential for `owner`. The value is
    /// an opaque, already-serialized JSON blob -- this crate deliberately has
    /// no dependency on `webauthn-rs`'s own `Passkey` type (the same
    /// separation this crate already keeps from `spartan-cloud-runtime`);
    /// `spartan-cloud-api` owns serializing/deserializing the real credential
    /// object. Returns the new row's id, needed later to update the same
    /// credential's stored authenticator counter after a real authentication.
    pub fn add_passkey(&self, owner: &UserId, passkey_json: &str) -> Result<i64, DataError> {
        self.conn.execute(
            "INSERT INTO webauthn_credentials (owner_id, passkey_json) VALUES (?1, ?2)",
            params![owner.0, passkey_json],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// All of `owner`'s registered credentials, as `(row_id, opaque_json)`
    /// pairs -- owner-scoped, the same enforced per-tenant isolation
    /// invariant every other real per-tenant table in this store keeps.
    pub fn list_passkeys(&self, owner: &UserId) -> Result<Vec<(i64, String)>, DataError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, passkey_json FROM webauthn_credentials WHERE owner_id = ?1")?;
        let rows = stmt.query_map(params![owner.0], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DataError::from)
    }

    /// Real count of `owner`'s registered credentials -- feeds the admin
    /// dashboard's own "N security keys registered" line without exposing
    /// the credential material itself.
    pub fn count_passkeys(&self, owner: &UserId) -> Result<u32, DataError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM webauthn_credentials WHERE owner_id = ?1",
            params![owner.0],
            |r| r.get(0),
        )?;
        Ok(count as u32)
    }

    /// Overwrite one already-registered credential's stored JSON by its real
    /// row id (never by content match) -- used after a real authentication
    /// ceremony to persist an updated authenticator counter/backup-state,
    /// the concrete defense against a cloned-authenticator replay that
    /// `Passkey::update_credential`'s own real return value signals is
    /// needed. Most passkeys (synced/platform ones) never actually change --
    /// see that method's own doc comment -- so this is a real, occasionally-
    /// no-op write, not dead code.
    pub fn update_passkey(&self, row_id: i64, passkey_json: &str) -> Result<(), DataError> {
        self.conn.execute(
            "UPDATE webauthn_credentials SET passkey_json = ?1 WHERE id = ?2",
            params![passkey_json, row_id],
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

    /// A plain email -> id lookup, no password involved -- used by the real
    /// WebAuthn login-start endpoint, which needs to know *which* account's
    /// registered credentials to challenge before any password is checked.
    /// Deliberately distinct from `verify_login` (which never reveals
    /// whether an email is registered); a WebAuthn login-start response
    /// necessarily does reveal that (the browser needs to know which
    /// credentials to offer), a real, named, narrower privacy trade-off than
    /// the password path, acceptable for this additional-factor admin flow.
    pub fn find_user_by_email(&self, email: &str) -> Result<Option<UserId>, DataError> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM users WHERE email = ?1",
                params![email],
                |r| r.get(0),
            )
            .optional()?;
        Ok(id.map(UserId))
    }

    /// The real email for an already-known user id -- used by the WebAuthn
    /// registration ceremony to give the authenticator a real, human-
    /// readable account label rather than the opaque id.
    pub fn email_of(&self, user_id: &UserId) -> Result<Option<String>, DataError> {
        let email: Option<String> = self
            .conn
            .query_row(
                "SELECT email FROM users WHERE id = ?1",
                params![user_id.0],
                |r| r.get(0),
            )
            .optional()?;
        Ok(email)
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

    /// Append a security-relevant event to the audit log. **Append-only** by
    /// design -- there is deliberately no update or delete method, so the log
    /// is tamper-evident against this crate's own API (a real deployment would
    /// additionally protect the DB file). `actor` is `None` for pre-auth
    /// events (e.g. a failed login where no user is established). A record
    /// failure never aborts the action it describes -- callers log-and-continue
    /// (an audit gap is bad, but silently dropping the actual operation is
    /// worse), so this returns a `Result` the caller may choose to soft-handle.
    pub fn record_audit(
        &self,
        at_unix: u64,
        actor: Option<&UserId>,
        action: &str,
        target: Option<&str>,
        detail: Option<&str>,
    ) -> Result<(), DataError> {
        self.conn.execute(
            "INSERT INTO audit_log (at_unix, actor_id, action, target, detail)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                at_unix as i64,
                actor.map(|u| u.0.as_str()),
                action,
                target,
                detail,
            ],
        )?;
        Ok(())
    }

    /// The most recent `limit` audit events, newest first. Feeds the (later)
    /// admin abuse/monitoring dashboard.
    pub fn recent_audit(&self, limit: u32) -> Result<Vec<AuditEvent>, DataError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, at_unix, actor_id, action, target, detail
             FROM audit_log ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(AuditEvent {
                id: r.get(0)?,
                at_unix: r.get::<_, i64>(1)? as u64,
                actor_id: r.get::<_, Option<String>>(2)?.map(UserId),
                action: r.get(3)?,
                target: r.get(4)?,
                detail: r.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DataError::from)
    }

    /// Store an encrypted per-tenant secret (SSH deploy keys, registry
    /// credentials, per-allocation capability tokens). Encrypted with
    /// **AES-256-GCM** under a fresh random 96-bit nonce; only the nonce +
    /// ciphertext (which includes GCM's authentication tag) are persisted --
    /// never the plaintext, never the key. Overwrites an existing secret of
    /// the same `(owner, name)`. Refused with `VaultLocked` if no master key.
    pub fn put_secret(
        &self,
        owner: &UserId,
        name: &str,
        plaintext: &[u8],
    ) -> Result<(), DataError> {
        let cipher = self.cipher.as_ref().ok_or(DataError::VaultLocked)?;
        // Fresh random 96-bit nonce per record (GCM's hard requirement: never
        // reuse a nonce under the same key). From the `rand` crate we already
        // depend on, same discipline as the argon2 salt.
        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|_| DataError::Crypto)?;
        self.conn.execute(
            "INSERT INTO secrets (owner_id, name, nonce, ciphertext) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(owner_id, name) DO UPDATE SET nonce = ?3, ciphertext = ?4",
            params![owner.0, name, nonce_bytes.to_vec(), ciphertext],
        )?;
        Ok(())
    }

    /// Decrypt and return a tenant's secret, or `None` if it doesn't exist.
    /// A decryption/authentication failure (tampered ciphertext, wrong key) is
    /// a real `DataError::Crypto`, never a silent empty result.
    pub fn get_secret(&self, owner: &UserId, name: &str) -> Result<Option<Vec<u8>>, DataError> {
        let cipher = self.cipher.as_ref().ok_or(DataError::VaultLocked)?;
        let row: Option<(Vec<u8>, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT nonce, ciphertext FROM secrets WHERE owner_id = ?1 AND name = ?2",
                params![owner.0, name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((nonce_bytes, ciphertext)) = row else {
            return Ok(None);
        };
        if nonce_bytes.len() != 12 {
            return Err(DataError::Crypto);
        }
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|_| DataError::Crypto)?;
        Ok(Some(plaintext))
    }

    /// The names of a tenant's stored secrets (never the values). Owner-scoped
    /// -- the enforced per-tenant isolation invariant: a query only ever sees
    /// its own owner's rows.
    pub fn list_secret_names(&self, owner: &UserId) -> Result<Vec<String>, DataError> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM secrets WHERE owner_id = ?1 ORDER BY name")?;
        let rows = stmt.query_map(params![owner.0], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DataError::from)
    }

    /// Delete a tenant's secret. Idempotent -- deleting an absent secret is a
    /// harmless no-op. Owner-scoped, so one tenant can never delete another's.
    pub fn delete_secret(&self, owner: &UserId, name: &str) -> Result<(), DataError> {
        self.conn.execute(
            "DELETE FROM secrets WHERE owner_id = ?1 AND name = ?2",
            params![owner.0, name],
        )?;
        Ok(())
    }
}

/// One row of the append-only audit log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub id: i64,
    pub at_unix: u64,
    /// The acting user, or `None` for pre-auth events.
    pub actor_id: Option<UserId>,
    pub action: String,
    pub target: Option<String>,
    pub detail: Option<String>,
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

    #[test]
    fn audit_log_is_append_only_and_newest_first() {
        let store = Store::open_in_memory().unwrap();
        let actor = store.create_user("ops@example.com", "pw", true).unwrap();

        // A pre-auth event (no actor) and two attributed events.
        store
            .record_audit(100, None, "login_failed", None, Some("bad@example.com"))
            .unwrap();
        store
            .record_audit(200, Some(&actor), "allocate", Some("alloc-1"), None)
            .unwrap();
        store
            .record_audit(300, Some(&actor), "grant_pro", Some("user-9"), None)
            .unwrap();

        let events = store.recent_audit(10).unwrap();
        assert_eq!(events.len(), 3);
        // Newest first.
        assert_eq!(events[0].action, "grant_pro");
        assert_eq!(events[0].actor_id.as_ref(), Some(&actor));
        assert_eq!(events[0].target.as_deref(), Some("user-9"));
        assert_eq!(events[1].action, "allocate");
        // The pre-auth event carries no actor.
        assert_eq!(events[2].action, "login_failed");
        assert_eq!(events[2].actor_id, None);
        assert_eq!(events[2].detail.as_deref(), Some("bad@example.com"));

        // The limit is respected.
        let one = store.recent_audit(1).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].action, "grant_pro");
    }

    #[test]
    fn vault_round_trips_a_secret_and_never_stores_plaintext() {
        let key = [7u8; 32];
        let store = Store::open_in_memory_with_key(&key).unwrap();
        let owner = UserId("tenant-a".into());

        store
            .put_secret(&owner, "registry_token", b"s3cr3t-ghp_value")
            .unwrap();

        // Round-trips exactly.
        let got = store.get_secret(&owner, "registry_token").unwrap();
        assert_eq!(got.as_deref(), Some(&b"s3cr3t-ghp_value"[..]));

        // The plaintext is never in the stored ciphertext.
        let raw: Vec<u8> = store
            .conn
            .query_row(
                "SELECT ciphertext FROM secrets WHERE owner_id = 'tenant-a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !raw.windows(6).any(|w| w == b"s3cr3t"),
            "the plaintext must never appear in the at-rest ciphertext"
        );

        // Names list without exposing values; delete is idempotent.
        assert_eq!(
            store.list_secret_names(&owner).unwrap(),
            vec!["registry_token".to_string()]
        );
        store.delete_secret(&owner, "registry_token").unwrap();
        assert_eq!(store.get_secret(&owner, "registry_token").unwrap(), None);
        assert!(store.delete_secret(&owner, "registry_token").is_ok());
    }

    #[test]
    fn vault_detects_tampering_via_gcm_authentication() {
        let key = [9u8; 32];
        let store = Store::open_in_memory_with_key(&key).unwrap();
        let owner = UserId("tenant-b".into());
        store
            .put_secret(&owner, "k", b"authenticated data")
            .unwrap();

        // Flip a byte of the stored ciphertext -- GCM's auth tag must reject it
        // on decrypt (this is exactly what the reference's unauthenticated CBC
        // could NOT detect). Read, flip the last byte (part of the auth tag),
        // write back -- a guaranteed, deterministic mutation.
        let mut ct: Vec<u8> = store
            .conn
            .query_row(
                "SELECT ciphertext FROM secrets WHERE owner_id = 'tenant-b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let last = ct.len() - 1;
        ct[last] ^= 0xFF;
        store
            .conn
            .execute(
                "UPDATE secrets SET ciphertext = ?1 WHERE owner_id = 'tenant-b'",
                params![ct],
            )
            .unwrap();
        assert!(
            matches!(store.get_secret(&owner, "k"), Err(DataError::Crypto)),
            "tampered ciphertext must fail authentication, not decrypt silently"
        );
    }

    #[test]
    fn vault_is_owner_scoped_and_wrong_key_cannot_read() {
        let store = Store::open_in_memory_with_key(&[1u8; 32]).unwrap();
        let a = UserId("owner-a".into());
        let b = UserId("owner-b".into());
        store.put_secret(&a, "shared_name", b"a-value").unwrap();

        // Another tenant with the same secret name sees nothing of tenant a's.
        assert_eq!(store.get_secret(&b, "shared_name").unwrap(), None);
        assert!(store.list_secret_names(&b).unwrap().is_empty());

        // A store opened with a DIFFERENT master key cannot decrypt a's secret
        // (GCM authentication fails) -- the key genuinely gates access.
        let path = format!(
            "{}/vault-key-test-{}.db",
            std::env::temp_dir().display(),
            std::process::id()
        );
        let _ = std::fs::remove_file(&path);
        {
            let s1 = Store::open_with_key(&path, &[2u8; 32]).unwrap();
            s1.put_secret(&a, "k", b"secret").unwrap();
        }
        let s2 = Store::open_with_key(&path, &[3u8; 32]).unwrap(); // wrong key
        assert!(matches!(s2.get_secret(&a, "k"), Err(DataError::Crypto)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn vault_operations_are_refused_without_a_master_key() {
        let store = Store::open_in_memory().unwrap(); // no key
        let owner = UserId("x".into());
        assert!(matches!(
            store.put_secret(&owner, "k", b"v"),
            Err(DataError::VaultLocked)
        ));
        assert!(matches!(
            store.get_secret(&owner, "k"),
            Err(DataError::VaultLocked)
        ));
    }

    #[test]
    fn find_user_by_email_finds_real_users_and_none_for_unknown() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_user("erin@example.com", "pw", false).unwrap();
        assert_eq!(
            store.find_user_by_email("erin@example.com").unwrap(),
            Some(id)
        );
        assert_eq!(
            store.find_user_by_email("nobody@example.com").unwrap(),
            None
        );
    }

    #[test]
    fn email_of_finds_a_real_email_and_none_for_unknown_id() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_user("frank@example.com", "pw", false).unwrap();
        assert_eq!(
            store.email_of(&id).unwrap(),
            Some("frank@example.com".to_string())
        );
        assert_eq!(store.email_of(&UserId("nonexistent".into())).unwrap(), None);
    }

    #[test]
    fn webauthn_credentials_add_list_count_and_update_are_owner_scoped() {
        let store = Store::open_in_memory().unwrap();
        let a = store.create_user("alice@example.com", "pw", true).unwrap();
        let b = store.create_user("bob@example.com", "pw", false).unwrap();

        assert_eq!(store.count_passkeys(&a).unwrap(), 0);
        let row_id = store.add_passkey(&a, r#"{"cred":"one"}"#).unwrap();
        assert_eq!(store.count_passkeys(&a).unwrap(), 1);
        assert_eq!(
            store.count_passkeys(&b).unwrap(),
            0,
            "owner-scoped: b sees none of a's"
        );

        let listed = store.list_passkeys(&a).unwrap();
        assert_eq!(listed, vec![(row_id, r#"{"cred":"one"}"#.to_string())]);
        assert!(store.list_passkeys(&b).unwrap().is_empty());

        // A second credential for the same owner is a distinct row.
        store.add_passkey(&a, r#"{"cred":"two"}"#).unwrap();
        assert_eq!(store.count_passkeys(&a).unwrap(), 2);

        // Updating by real row id changes exactly that row, not the other.
        store
            .update_passkey(row_id, r#"{"cred":"one-updated"}"#)
            .unwrap();
        let listed = store.list_passkeys(&a).unwrap();
        let updated = listed.iter().find(|(id, _)| *id == row_id).unwrap();
        assert_eq!(updated.1, r#"{"cred":"one-updated"}"#);
        assert!(listed.iter().any(|(_, j)| j == r#"{"cred":"two"}"#));
    }
}
