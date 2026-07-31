# Agent instructions for Spartan IDE

This file is the entry point for any AI coding agent working in this repository (Codex, Claude
Code, or otherwise). It is deliberately short — the real source of truth is `CLAUDE.md`, a large,
append-only historical log of every real feature pass this project has gone through, written in
detail specifically so a new agent with no prior context can pick up accurately. **Read
`CLAUDE.md` in full before making changes** — it is not optional background reading, it is the
actual specification of what's built, what isn't, and why.

## What this project is

A from-scratch, agent-first desktop IDE. The primary UI is a real Electron + React frontend
(`desktop/`) driving a real Rust core over a local IPC service (`crates/spartan-backend`). A
companion browser build lives in `web/`. An earlier custom wgpu-native shell
(`crates/spartan-editor-core`) is kept as a tested reference implementation, not deleted, not
primary. No VS Code/Monaco/CodeMirror code is forked or vendored anywhere, ever — this is a locked
decision, not an open question.

## Where to look, in order

1. **`CLAUDE.md`** — read top to bottom before touching code. It has an index table ("Where things
   live"), a full "Current status" narrative of every real pass this project has done (each one
   documents what was actually built, how it was actually verified, and what it explicitly does
   *not* confirm), a "Build & test" section with exact commands and every known environment
   caveat, a "Rules, not suggestions" section, and a "What NOT to do" section. All of it is real
   and current — nothing there is aspirational.
2. **`docs/architecture-spec.md`** — the 75-section behavioral spec. Read the relevant section
   before implementing anything; don't guess from a section title.
3. **`docs/FUTURE_FEATURES.md`** — the live backlog. Every row is a real, already-named gap, not
   speculative marketing. This is where to look for "what's next."

## The non-negotiable discipline this project runs on

- **Never claim something works without running it.** Real implementation, real verification —
  typecheck/build/clippy/tests, and where feasible a live end-to-end run (Playwright for the
  Electron/web shells, real subprocess integration tests for Rust) with genuine input, not a
  synthetic/mocked shortcut standing in for the real thing. If something truly can't be verified
  in the current environment (no GPU, no display, a tool that isn't installed, no live model
  backend), say so explicitly rather than estimating and presenting it as measured.
- **Don't fork or vendor VS Code/Monaco/CodeMirror code, ever, for any reason.**
- **Security hardening (see `CLAUDE.md` §9/§36 references) is not optional scope** —
  path-jailing, approval gating before destructive actions, secrets redaction. Don't simplify
  these away for convenience.
- **Follow the existing per-feature workflow**: implement in `desktop/` first, port to `web/`
  second (both are real, maintained shells). `web/` actually has two separate editing surfaces —
  `web/src/components/BackendEditor.tsx` (backend-connected, via `spartan-devserver`'s WebSocket
  transport) genuinely shares editor/LSP/DAP/git/Leo features with `desktop/src/components/
  Editor.tsx`; `web/src/components/Editor.tsx` is the **pure client-side** editor (File System
  Access API + WASM-compiled `spartan-buffer`, no backend process at all) and has none of
  those — only real, local editing/syntax-highlighting features apply there. Target whichever
  surface a feature actually reaches, don't claim parity it doesn't have. Verify, document the
  pass in both `CLAUDE.md` (in the same detailed historical-narrative style every existing entry
  uses) and `docs/FUTURE_FEATURES.md` (mark the row done, or add one), then commit. This
  project's history is full of real bugs found only by actually running things — match that bar,
  don't shortcut it.
- **This environment cannot launch a real Electron window** (a standing, documented network-policy
  constraint, not a code problem). The established workaround for `desktop/`-only features is a
  Playwright script serving the compiled `desktop/dist` via `crates/spartan-devserver` with a thin
  `window.spartan` shim that forwards calls over the real WebSocket transport — see any recent
  `CLAUDE.md` entry tagged "desktop/" for a working example script. Don't assume this constraint
  no longer applies without checking for yourself.

## Build & test — quick reference

The full, heavily-annotated version (with every known self-skip/environment caveat) is in
`CLAUDE.md`'s own "Build & test" section — read it before assuming a test failure is a real
regression rather than a missing external tool (many integration tests self-skip honestly when a
tool like `pyright-langserver`, `rust-analyzer`, `lldb-dap`, `debugpy`, Docker, `ollama`, or
`litellm` isn't present, matching a consistent convention across the whole repo).

```bash
# Rust workspace (crates/, spikes/, xtask)
cargo fmt --all -- --check
cargo clippy --workspace --release --all-targets
cargo test --workspace --release

# desktop/ (Electron + React) — build spartan-backend first, it's the IPC service desktop/ drives
cargo build --release -p spartan-backend
(cd desktop && npm install && npm run typecheck && npm run build && npm run build:electron)

# web/ (browser IDE) — separate Vite+React project, not part of the Cargo workspace
(cd web && npm install && npm run build:wasm && npm run typecheck && npm run build)

# mobile/ (Expo/React Native companion) — see mobile/CLAUDE.md and mobile/AGENTS.md
(cd mobile && npx tsc --noEmit && npx expo export --platform android)

# cloud/ (Spartan Cloud, Track B) — its own separate Cargo workspace, not part of the root one
(cd cloud && cargo fmt --all -- --check && cargo clippy --workspace --all-targets && cargo test --workspace)
```

Each command block above is wrapped in a subshell (`(cd X && ...)`) so it never changes your
actual working directory — safe to copy/paste any subset without needing to `cd` back to the
repo root in between.

## Current repo state

Branch, PR status, working-tree cleanliness, and CI results are all real but change constantly
across sessions — a hardcoded snapshot here would go stale the moment the next commit lands or PR
merges. Check `git status`, `git branch --show-current`, and the current PR/CI state directly
before relying on any of it; don't assume a prior agent's note about "CI is green" or "branch X"
still holds. Durable facts about the repo's own real, current architecture:

- The GUI Builder (a previously-shipped Design screen + `gui-builder/` npm project) was **removed
  from the product entirely** at the user's explicit request — it is not deferred, not
  placeholdered. Don't reintroduce it without discussing it first; see `CLAUDE.md`'s own removal
  entry for the full account and `docs/FUTURE_FEATURES.md`'s "GUI Builder — removed" section.
- `docs/FUTURE_FEATURES.md`'s "Recommended next 10" list at the top is genuinely current and is
  the right place to look for the next well-scoped, honestly-verifiable item. Several P1 editor
  items (code folding, multi-cursor) are explicitly marked architecturally blocked by the current
  `<textarea>`-backed editing surface — don't attempt them without first reading why in that file.
- Several LSP capabilities (code actions, inlay hints, semantic tokens, `workspace/symbol`) were
  investigated and found genuinely unusable in this specific dev environment (only
  `pyright-langserver` and `rust-analyzer` are installed, and pyright either doesn't declare the
  capability or returns empty results for it here) — real, live-probed findings, not assumptions.
  Re-probe before assuming this still holds in a different environment with a richer server
  installed.
