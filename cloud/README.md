# Spartan Cloud (Track B)

A genuinely **separate, optional** multi-tenant backend that allocates
isolated containers per user for building/testing projects — positioned
**alongside** the local-first Spartan IDE product, not replacing it. Access
is intended to be via a paid subscription (billing **deferred**; see the
entitlement seam below).

This is its **own Cargo workspace**, deliberately *not* a member of the
repo-root `Cargo.toml`. The root lists its members explicitly (not a glob),
so this sibling directory is invisible to `cargo build --workspace` /
`cargo test --workspace` at the repo root — the same isolation
`crates/plugins` already uses for a different reason. Build/test it from
inside `cloud/`:

```bash
cd cloud && cargo test          # runs the pure-domain unit tests
cd cloud && cargo clippy --all-targets && cargo fmt -- --check
```

## What's real right now (first increment)

Pure, infra-free **domain logic**, fully unit-tested with no server and no
live daemon:

- **`spartan-cloud-protocol`** — shared serde DTOs: `UserId`,
  `SessionToken` (opaque, revocable — deliberately *not* a JWT),
  `AllocationId`, `PlanTier`, `AllocationStatus`, and the control-plane
  request/response shapes. New types, not an import of `spartan-backend`'s
  own single-process `Request`/`Response`.
- **`spartan-cloud-tenant`** — the multi-tenancy core:
  - **`EntitlementProvider`** trait + `StubEntitlementProvider` — the seam
    behind which real billing (e.g. a `StripeEntitlementProvider`) swaps in
    later with zero caller changes, mirroring the `ModelProvider` pattern.
    Billing is deferred by explicit decision; the stub is a manual/admin
    Pro toggle, honestly not a fabricated subscription.
  - **`PlanLimits` + `can_allocate`** — real per-tier quotas (CPU millis,
    memory, pids, hard wall-clock lifetime, max concurrency) and the
    allocation-admission check. No plan is ever "unlimited" — the whole
    point of running untrusted tenant code under caps.
  - **`Session`** — opaque 256-bit token generation + expiry math;
    persistence and early revocation live in the (later) data layer.

- **`spartan-cloud-data`** — persistence: SQLite (embedded via `rusqlite`'s
  `bundled` feature, zero infra) with real **argon2** password hashing and
  the session store where opaque-token **revocation** lives (a row delete).
  `verify_login` returns `Ok(None)` for both wrong-password and unknown-email
  (no account enumeration). 6 unit tests.
- **`spartan-cloud-api`** — the axum control-plane server, **real and
  testable**: `POST /api/signup`, `POST /api/login`, `GET /api/me`,
  `POST /api/admin/grant_pro` (admin-only), and `POST /api/allocate` (runs the
  real entitlement → plan-limits → quota admission). Opaque bearer-token auth
  via a `FromRequestParts` extractor that looks up + expiry-checks the session
  per request. Env-driven admin bootstrap (`SPARTAN_CLOUD_ADMIN_EMAIL`/
  `_PASSWORD`), never a hardcoded credential. 6 tests (tower `oneshot`), plus
  a live over-the-socket smoke test.

  **Deliberately honest, not faked:** `/api/allocate`, once admission passes,
  returns `503 runtime_unavailable` because the container runtime isn't wired
  yet — this crate never pretends to start a container it can't.

## What's NOT here yet (next increments)

1. **`spartan-cloud-runtime`** — the `ContainerRuntime` trait + a real
   `DockerGvisorRuntime` driver (bollard). **Gated behind its own real
   go/no-go spike**: start a live Docker daemon, install `runsc` (gVisor),
   confirm isolation + resource-cap enforcement. gVisor is the MVP choice
   because it's the only strong-isolation option testable in a KVM-less
   environment (Firecracker/Kata need `/dev/kvm`, absent here per §75.74);
   Firecracker stays a documented future upgrade behind the same trait. Once
   it lands, `/api/allocate` allocates for real and the current-active count
   feeding quota admission comes from it.
2. **WebAuthn admin auth + audit log** on the API (defensive concepts adapted
   from `SpartanAI_Security_Core`, rebuilt safely) and the per-container WS
   session endpoint (reusing `spartan-backend`'s envelope shape).

## Standing safety posture

This backend runs **other people's** build/test code, so isolation is the
highest bar in this repo. Nothing here integrates any offensive, autonomous,
stealth, or lateral-movement capability — the tenant caps + reaper + no
cross-tenant mounts + per-tenant networks are precisely the *defense* against
the class of tenant abuse (cryptomining, resource exhaustion, escape
attempts) that the declined SpartanAI malware repos performed.
