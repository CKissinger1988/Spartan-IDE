# Spartan IDE

**A from-scratch, agent-first desktop IDE.** A real Electron + React frontend
(`desktop/`) drives a real Rust core — a hand-built rope buffer, tree-sitter syntax
highlighting, in-house LSP/DAP clients, real git integration, and **Leo**, an agentic
coding assistant with a plan → approve → execute → verify loop — over a local IPC
service. No VS Code, Monaco, or CodeMirror code is forked or vendored anywhere in this
repository.

This README says exactly what's real and what isn't. See
["What's actually real right now"](#whats-actually-real-right-now) before assuming
anything beyond that — and see [`CLAUDE.md`](CLAUDE.md) for the full, continuously
updated status log this section summarizes.

## Why from scratch

Forking VS Code/Monaco is the fast path every other AI-native editor in this space has
taken (Cursor, Windsurf, Antigravity IDE) — and it's also where their documented failure
modes come from: forced app splits, extension-host isolation gaps, editor surfaces that
can't be redesigned because they're not really yours. [`docs/architecture-spec.md`](docs/architecture-spec.md)
§36 catalogs these failures by name and root-causes each one. "Own the buffer, own the
editing surface" is a locked decision, not an open question revisited per feature — the
text-editing surface in `desktop/src/components/Editor.tsx` is real, hand-built React
chrome, backed by the real Rust buffer over IPC, not a vendored component.

## Architecture

```
desktop/                    Real Electron + React desktop shell — the current, primary UI
  electron/                 Main process: spawns spartan-backend, exposes a narrow IPC bridge
  src/                      React renderer: editor, file tree, Git panel, terminal, Leo chat,
                             GUI Builder / live preview, Workflows canvas, Settings

crates/spartan-backend/     Real Rust IPC service the Electron shell drives — wraps every
                             other crate below behind a newline-delimited JSON-RPC protocol
crates/spartan-buffer/      Real rope-based document/buffer model — branching undo tree,
                             bounded checkpoint ring, char-indexed edits
crates/spartan-languages/   Real LanguageProfile registry — LSP/DAP commands, build systems,
                             marker-file project detection, for 7 languages (Rust, TS/JS,
                             Python, Kotlin, Java, Go, C#)
crates/spartan-leo/         Real agentic core — state machine, sandboxed tool execution,
                             risk-classified approval gating, project-tier memory
crates/spartan-model/       Real ModelProvider implementations — Ollama, Claude, LiteLLM
crates/spartan-git/         Real git integration (via libgit2) — status, stage, commit,
                             Leo's own checkpoint/rollback mechanism
crates/spartan-settings/    Real persisted settings (~/.spartan/settings.json)
crates/spartan-security/    Real secrets detection/redaction (credential-shaped regexes)
crates/spartan-crash/       Real local-first crash reporter (panic hook, redacted, no upload)
crates/spartan-updater/     Real "check for updates" against this repo's own GitHub API
crates/spartan-plugin-host/ Real WASM Component Model plugin host (wasmtime), capability-gated
crates/spartan-editor-core/ The original wgpu-native shell — kept as the tested reference
                             implementation and backend proof-of-concept, not deleted; every
                             feature it proved was later promoted into the crates above

gui-builder/                Real, separate npm project — parses/edits JSX via Babel + recast,
                             bundles a live preview via esbuild, powers the Design screen
mobile/                     Real Expo/React Native companion app — Spartan Mobile IDE
spikes/                     Real Tier 0 risk-gate spikes (rope perf, LSP/DAP clients, GPU
                             rendering, local-model tool-call parsing) — not the product itself
legacy/agent-deck-console/  This repo's prior product, preserved for feature-parity reference
```

## What it actually is

### Leo — the agent

- **Real plan → approve → execute → verify loop**: a task produces an Implementation Plan
  (goal, approach, files, risk notes) you approve or reject; approval creates a real git
  checkpoint; execution proposes one real tool call at a time — `read_file`, `edit_file`
  (with a real diff preview before you approve it), `run_terminal`, `search_files`,
  `list_directory` — each one risk-classified and gated by your configured approval mode
- **Configurable approval mode**: manual (every call needs a click) or auto-approve-safe
  (read-only exploration runs immediately; `edit_file`/`run_terminal` are *never*
  auto-approved, by construction, regardless of setting)
- **Real project-tier memory**: a plain, hand-editable Markdown file at
  `.spartan/memory/project.md`, read into planning context and appended to on completion
- **Multi-provider**: switch Leo between local Ollama, Anthropic's Claude API, or a local
  LiteLLM proxy — from Settings, no code changes
- **Voice input/output**: dictate a task via the browser's native speech recognition;
  optionally have plan/completion/error events read aloud

### The editor

- **Real rope-based buffer** (not a text-diffing hack) with branching undo/redo, real
  save-to-disk, a real file tree, tabs, and click-to-select/drag-select
- **Real syntax highlighting** across the languages `spartan-languages` knows about
- **Real Git panel**: stage/unstage/commit against the actual repository on disk, wired to
  the same `spartan-git` crate Leo's own checkpointing uses
- **Real integrated terminal** (Console) and a **real multi-CLI session manager**
  (Sessions) — spawn `claude`/`codex`/`gemini`/any named CLI as a real PTY, streamed live
  over IPC via `xterm.js`
- **Real GUI Builder + live preview** (Design screen): parses a JSX/TSX component's real
  AST, renders it live in a sandboxed iframe via a real esbuild bundle, supports
  click-to-select and a Canvas → Code round trip (prop/style edits land back in the real
  source file, formatting-preserving, via `recast`)
- **Real node-graph Workflows canvas** for visualizing/launching multi-CLI orchestration

### Trust & security

- **Sandboxed, path-jailed tool execution**: Leo's tools resolve every path through a real
  jail that rejects `..` escapes and symlink tricks, confirmed by tests, not just a prompt
  instruction
- **Risk-classified approval gating**: `Safe` vs. `Destructive` is a real Rust enum Leo's
  own execution path checks — a `Destructive` call can never auto-run, in any mode
- **Secrets detection**: opened files are scanned for credential-shaped strings (AWS keys,
  GitHub/Slack/Stripe tokens, PEM blocks) and flagged
- **Local-first crash reporting**: panics are caught, redacted, and written to
  `~/.spartan/crashes/` — no auto-upload of any kind exists

Full detail on all of the above — and the much larger set of design-stage features not yet
built — lives in [`docs/architecture-spec.md`](docs/architecture-spec.md).

## Source of truth

[`docs/architecture-spec.md`](docs/architecture-spec.md) is the spec. [`CLAUDE.md`](CLAUDE.md)
is the index into it and the behavioral contract for working in this repo — read it before
touching security, sandboxing, or approval flows (§9, §36).

## What's actually real right now

- **Real, working, tested code**: everything under [Architecture](#architecture) above.
  488 Rust tests across 12 real crates + 6 Tier 0 spikes + `xtask`, all passing; clippy and
  `cargo fmt` clean. The Electron shell's own TypeScript typechecks and builds clean, and
  every increment of it has been verified via a live, screenshotted Playwright pass driving
  a real Vite dev server against a test-only mock of the Electron preload bridge.
- **One honest, standing gap**: the *actual* Electron window has never been launched from
  inside this project's own development sessions, because Electron's postinstall script
  downloads its runtime binary from `github.com/electron/electron/releases`, and every
  sandboxed session used to build this repo so far has had that host blocked by its own
  network policy. Everything else — the real Rust backend, the full IPC protocol, every
  React component — is built and tested; it needs a real `npm install` (no
  `ELECTRON_SKIP_BINARY_DOWNLOAD`) run somewhere with normal internet access to actually
  see the window. See [`desktop/README.md`](desktop/README.md).
- **Reference-only**: [`prototypes/*.jsx`](prototypes/) are early React mockups of the
  intended UI, not wired to anything. [`legacy/agent-deck-console/`](legacy/agent-deck-console/)
  is this repo's prior, different product, kept for feature-parity reference (§55).
- **Not yet built**: Android support, a packaged/signed Electron installer, an automated
  crash-report *upload* service (local-only today), a `Reparent`/`ComponentInsert` GUI
  Builder operation, and the larger design-stage surface (§35 is the prioritized roadmap).

This project's own history includes real bugs found only by actually running code and
adversarially testing it — a UTF-8 char-boundary panic, a cross-adapter DAP deadlock, an
intermittent LSP race, a stale-selection bug caught only by testing paste-after-a-click.
See `CLAUDE.md`'s own §75.x log for the full, honest account of what was found and fixed,
pass by pass.

## Getting started

### Rust core

```bash
cargo build --release --workspace
cargo test --workspace --release
cargo clippy --workspace --release --all-targets
cargo fmt --all -- --check
```

Some integration tests spawn real external tools (`rust-analyzer`, `lldb-dap`, `debugpy`,
`cargo-component`, a live Ollama instance) and self-skip with a printed message if the
tool isn't on `$PATH` — not a failure. See `CLAUDE.md`'s "Build & test" section for the
full list and known flakiness notes.

### Desktop shell

```bash
cargo build --release -p spartan-backend   # the IPC service the Electron shell drives
cd desktop
npm install                                # needs real internet access to fetch Electron
npm start
```

See [`desktop/README.md`](desktop/README.md) for the full setup story, including the
`ELECTRON_SKIP_BINARY_DOWNLOAD=1` fallback used in this project's own restricted sessions
(which lets everything except the actual window launch build and typecheck).

### GUI Builder

```bash
cd gui-builder
npm install
npm test
```

## Repository layout

```
CLAUDE.md                    Index + behavioral contract for this repo
docs/architecture-spec.md    Full technical & design spec (source of truth, 75+ sections)
desktop/                     Electron + React desktop shell — the current primary UI
gui-builder/                 Real, separate npm project — JSX AST sync + live preview
crates/                      Real Rust product code (spartan-backend, spartan-buffer,
                              spartan-leo, spartan-model, spartan-git, spartan-editor-core,
                              and more — see Architecture above)
spikes/                      Real, tested Tier 0 Rust spikes (see spikes/README.md)
mobile/                      Spartan Mobile IDE — real Expo/React Native companion app
prototypes/                  Reference-only React UI mockups, not wired to anything
legacy/agent-deck-console/   Prior product, preserved for feature-parity reference (§55)
.github/workflows/           CI — fmt/clippy/test for Rust, typecheck/build/test for the
                              three npm projects (desktop/, gui-builder/, mobile/)
LICENSE                      Proprietary, all rights reserved
Cargo.toml / Cargo.lock      Rust workspace (spikes/ + crates/, excluding crates/plugins/
                              and mobile/, which are separate toolchains)
```

## Contributing / where to start reading

1. Read [`CLAUDE.md`](CLAUDE.md) in full — it's short, and it's the contract, not a
   suggestion.
2. Check [`docs/architecture-spec.md`](docs/architecture-spec.md) §35 for what's actually
   next in the build order before picking up new scope.
3. If you're touching security, sandboxing, or approval flows, read §9 and §36 first —
   they exist because of documented, named failures in comparable tools, not
   hypothetically.

## License

Proprietary — all rights reserved. See [`LICENSE`](LICENSE).
