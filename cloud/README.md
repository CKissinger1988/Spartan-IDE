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

  When a runtime is wired (see below), `/api/allocate` creates a **real,
  resource-capped container** for the tenant and returns `201 CREATED` with
  the `AllocationInfo`; without one it returns `503 runtime_unavailable`, and
  with a runtime whose isolation is unverified it returns
  `503 isolation_unverified` — this crate never pretends to start a container
  it can't, and never runs tenant code against isolation it hasn't confirmed.

- **`spartan-cloud-runtime`** — the `ContainerRuntime` async trait + a real
  **`DockerRuntime`** driver on `bollard`, **now wired into `/api/allocate`**.
  Every method is tenant-scoped and resource-capped: `host_config` maps each
  `PlanLimits` to real Docker caps (`memory`, `nano_cpus`, `pids_limit`), pins
  the OCI `runtime`, and uses `network_mode: none` — no host bind-mounts, only
  a fresh per-allocation scratch. `create` **ensures the image is present**
  (inspect-then-pull, so a fresh host/CI runner with nothing cached still
  allocates). Managed containers carry `MANAGED_LABEL` + `OWNER_LABEL` so
  `count_active` (which feeds quota admission) and teardown are owner-scoped.
  An honest `isolation_verified` flag rides along on the runtime: it is
  `false` unless the operator explicitly asserts it for the deployment
  (`SPARTAN_CLOUD_ISOLATION_VERIFIED=1`), and the API refuses to allocate
  against an unverified runtime.
  - **A real reaper** enforces each allocation's hard wall-clock lifetime:
    `create` stamps an absolute deadline label (`now + max_lifetime_secs`),
    and `reap_expired(now)` stops+removes every managed container (any tenant,
    any state) past its deadline — the concrete answer to §36.4.7's "uncapped
    consumption". A managed container with a missing/unparseable deadline is
    reaped fail-safe. `main.rs` runs it on a 60-second background interval when
    a runtime is connected.
  - 4 tests, including a **real create → status → count → stop lifecycle** and
    a **real reaper test** (fresh container spared, past-deadline container
    killed) against a live daemon (self-skips if none is reachable, mirroring
    `spartan-devcontainer`'s `docker_integration.rs`).

### gVisor go/no-go — result: **no-go in this nested sandbox; `runc` is the verified baseline here**

The plan's Phase 0 spike was actually run. `runsc` (gVisor) installs from the
confirmed apt package, but does **not** work as a Docker runtime in this
nested environment:

- gVisor's cgroup enforcement needs the `cpuset` controller — it was missing
  and had to be mounted before caps could be enforced at all.
- `runsc` refuses to run in the root network namespace; `--network=none`
  works around it.
- Even then the sandbox **hangs** on startup: gVisor's platform needs either
  KVM (absent here, per §75.74) or working `ptrace`/`systrap`, neither usable
  in this nested container.

So gVisor is a genuine **no-go here** — not a code problem, an environment
one. The `ContainerRuntime` trait/driver is therefore verified against plain
**`runc`** (the default OCI runtime), which is a shared-kernel baseline, *not*
strong adversarial isolation. This is surfaced honestly rather than absorbed:
`DockerRuntime` ships with `isolation_verified: false` by default, `main.rs`
only flips it when the operator sets `SPARTAN_CLOUD_ISOLATION_VERIFIED=1`, and
the API **refuses to allocate** against an unverified runtime. A real
KVM-capable target (bare metal / Firecracker / a KVM-enabled instance) is the
documented path to a genuinely-strong verified isolation, swappable behind the
same trait — no API or domain-layer change needed.

## What's NOT here yet (next increments)

1. **Strong-isolation verification** — gVisor (or Firecracker/Kata) confirmed
   on a real KVM-capable target, flipping `isolation_verified` to `true` in a
   production deployment. The seam and the honest-default flag exist today.
2. **WebAuthn admin auth + audit log** on the API (defensive concepts adapted
   from `SpartanAI_Security_Core`, rebuilt safely), a per-tenant abuse/
   resource-monitoring dashboard, and the per-container WS session endpoint
   (reusing `spartan-backend`'s envelope shape).

## Standing safety posture

This backend runs **other people's** build/test code, so isolation is the
highest bar in this repo. Nothing here integrates any offensive, autonomous,
stealth, or lateral-movement capability — the tenant caps + reaper + no
cross-tenant mounts + per-tenant networks are precisely the *defense* against
the class of tenant abuse (cryptomining, resource exhaustion, escape
attempts) that the declined SpartanAI malware repos performed.
