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
  (no account enumeration). Also an **append-only audit log** (`record_audit`/
  `recent_audit`, no update/delete method exists — tamper-evident against this
  crate's own API) and an **encrypted-at-rest secrets vault** (`put_secret`/
  `get_secret`/`list_secret_names`/`delete_secret`) using authenticated
  **AES-256-GCM** with a fresh random 96-bit nonce per record — deliberately
  *correcting* the `SpartanAI_Security_Core` concept, whose reference used
  unauthenticated AES-256-CBC despite claiming GCM. The master key comes from
  the operator's environment (`SPARTAN_CLOUD_VAULT_KEY`, 64 hex chars), is
  never persisted with the ciphertext, and when absent the vault is *locked*
  (operations refused, never a silent plaintext fallback). Owner-scoped so one
  tenant can never read/delete another's; a tampered ciphertext or wrong key is
  a real authentication failure, not a silent wrong result. Both are defensive
  concepts adapted from `SpartanAI_Security_Core`, rebuilt safely (no code
  ported). 11 unit tests (incl. GCM tamper-detection + tenant isolation).
- **`spartan-cloud-api`** — the axum control-plane server, **real and
  testable**: `POST /api/signup`, `POST /api/login`, `GET /api/me`,
  `POST /api/admin/grant_pro` (admin-only), `GET /api/admin/audit` (admin-only,
  the newest 200 audit events), `GET /api/admin/telemetry` (admin-only, a live
  per-container `docker stats`-style memory/pids snapshot via bollard, 503
  when no runtime is wired), and `POST /api/allocate` (runs the real
  entitlement → plan-limits → quota admission). Opaque bearer-token auth via a
  `FromRequestParts` extractor that looks up + expiry-checks the session per
  request. Env-driven admin bootstrap (`SPARTAN_CLOUD_ADMIN_EMAIL`/`_PASSWORD`),
  never a hardcoded credential. Security-relevant actions (signup, login,
  **failed** login, grant_pro, allocate, exec, secret writes) are audited
  (writes soft-fail so an audit error never aborts the real operation).

  When a runtime is wired (see below), `/api/allocate` creates a **real,
  resource-capped container** for the tenant and returns `201 CREATED` with
  the `AllocationInfo`; without one it returns `503 runtime_unavailable`, and
  with a runtime whose isolation is unverified it returns
  `503 isolation_unverified` — this crate never pretends to start a container
  it can't, and never runs tenant code against isolation it hasn't confirmed.

  **Making an allocation usable.** `POST /api/allocations/:id/exec` runs a
  one-shot argv command (never a shell string — no shell-injection surface)
  in the caller's own allocation, owner-scoped in the runtime, returning
  combined output + exit code. On top of that, a **real, streaming**
  interactive session: `POST /api/allocations/:id/session_token` mints a
  short-lived (60s), single-allocation-scoped `CapabilityToken` — ownership
  is checked once, here, via the runtime's own `list_owned` — and
  `GET /api/allocations/:id/session?token=...` upgrades to a real WebSocket
  driving a real `docker exec -it`-equivalent shell. The capability token
  (not the general bearer token, since a browser's native WebSocket API can't
  set custom headers on the upgrade request) is **consumed on first use** —
  removed from the in-memory map the moment it's validated — so a
  leaked/replayed session URL is dead after its first successful connection.
  The wire protocol (`ExecSessionClientMessage`/`ExecSessionServerEvent`,
  in `spartan-cloud-protocol`) reuses `spartan-backend`'s own real idea of a
  server sending unprompted named events (its `pty_output`/`pty_exit`) —
  not its literal wire strings or any type import, `cloud/` still has zero
  dependency on `spartan-backend` — expressed as fully-typed, tagged enums.
  The **encrypted-at-rest secrets vault** (AES-256-GCM) is also real end to
  end here: an owner-scoped REST surface — `PUT /api/secrets/:name`,
  `GET /api/secrets` (names only), `DELETE /api/secrets/:name`. Values are
  write-only over the API (never read back — a deliberate exposure-reducing
  choice; the server uses them when provisioning).

  **`GET /admin`** serves a real, self-contained monitoring-dashboard page —
  vanilla HTML/CSS/JS embedded at compile time via `include_str!` (no external
  assets, no runtime file I/O, so no path-traversal surface). It does a real
  `POST /api/login` (the bearer token lives only in a JS variable, never
  browser storage — a real, deliberate choice for an elevated-privilege admin
  tool) and polls the real `GET /api/admin/audit`/`GET /api/admin/telemetry`
  feeds every 5s. Color tokens and the `.glass-hologram`/`.hud-gauge`/
  status-reactive-glow classes are copied verbatim from `desktop/src/
  theme.css`'s own Track C layer — the exact reuse target that file's own
  comment already names for this dashboard. **Live-verified with a real
  Chromium browser (Playwright)** against a real running server with a real
  allocated container: login, a real telemetry gauge reflecting the real
  1 MiB/1024 MiB usage of a real alpine container, all four real audit rows
  in order, logout, a wrong-password rejection, and a non-admin account
  correctly blocked by the real 403s — not simulated, an actual end-to-end
  browser session against the actual binary.

  16 tests (tower `oneshot` for the REST surface; a real bound-socket
  end-to-end test drives the actual WebSocket upgrade and a real interactive
  shell command, then confirms a replayed capability token is refused), plus
  a live over-the-socket smoke test.

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
  - **Real exec, both shapes.** `exec_once` runs a one-shot command;
    `spawn_interactive_exec` spawns a real, long-lived `docker exec -it`
    session (a real pty, real stdin/stdout streaming, real resize) — the
    streaming counterpart backing the API's own WS session endpoint. Both
    share one `verify_owned` check: a container that doesn't exist, isn't
    managed, or belongs to a different tenant is `RuntimeError::NotFound`,
    deliberately indistinguishable from "doesn't exist" so this check can
    never be used to probe another tenant's allocations.
  - 7 tests, including a **real create → status → count → stop lifecycle**, a
    **real reaper test** (fresh container spared, past-deadline container
    killed), and a **real interactive session test** (a real shell echoes a
    real command, a real resize succeeds, `on_exit` fires on real shell exit,
    and a different tenant is denied) — all against a live daemon (self-skips
    if none is reachable, mirroring `spartan-devcontainer`'s own
    `docker_integration.rs`).

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
2. **WebAuthn admin auth** on the API (a defensive concept adapted from
   `SpartanAI_Security_Core`, rebuilt safely) — deliberately not attempted:
   this repo's own rule is never to claim something works without running it,
   and there's no FIDO2 hardware in this environment to test against. The
   admin dashboard's bearer-token login is real and tested; WebAuthn would be
   an additional, stronger auth factor layered on top of it.

Everything named in the plan's own "Explicitly deferred" list — real Stripe
billing, multi-node routing, cross-region deployment, an egress-allowlist
proxy, image/registry caching, org/team features — remains exactly that:
deferred by explicit decision, not forgotten.

## Standing safety posture

This backend runs **other people's** build/test code, so isolation is the
highest bar in this repo. Nothing here integrates any offensive, autonomous,
stealth, or lateral-movement capability — the tenant caps + reaper + no
cross-tenant mounts + per-tenant networks are precisely the *defense* against
the class of tenant abuse (cryptomining, resource exhaustion, escape
attempts) that the declined SpartanAI malware repos performed.
