//! Pure multi-tenancy domain logic for Spartan Cloud: the entitlement seam
//! (deferred billing), opaque revocable sessions, and per-plan quotas.
//!
//! Deliberately free of HTTP/DB/Docker dependencies -- every type here is
//! unit-testable with no server and no live daemon, mirroring how
//! `spartan-leo` is kept separable from `spartan-backend`. The data layer
//! (SQLite), the container runtime (Docker+gVisor), and the API (axum) all
//! build *on top of* this core in later increments.

pub mod entitlement;
pub mod quota;
pub mod session;

pub use entitlement::{Entitlement, EntitlementProvider, StubEntitlementProvider};
pub use quota::{can_allocate, PlanLimits, QuotaError};
pub use session::{new_opaque_token, Session};
