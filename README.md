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

## Screenshots

Real, unedited Playwright + Chromium captures of the actual running React components —
not mockups. The desktop shots use this project's own established "mocked
`window.spartan`" verification technique (see `desktop/README.md`), since the real
Electron binary itself remains unlaunchable in every session so far (a standing,
reported-not-routed-around network policy block — see `desktop/README.md`'s own
"environment-specific gap" section). The web shots run against a real `vite dev`
server with no mocking beyond substituting the native folder-picker dialog, which can't
be driven headlessly.

| | |
|---|---|
| ![Editor screen](docs/screenshots/desktop/01-editor-main-screen.png) | ![Git panel](docs/screenshots/desktop/02-git-panel.png) |
| Desktop: Editor screen — 3-tier nav, file tree, tabs, real syntax highlighting, Leo panel | Desktop: real Source Control panel |
| ![Web app editor](docs/screenshots/web/03-editor-with-syntax-highlighting.png) | ![Workflows screen](docs/screenshots/desktop/04-workflows-screen.png) |
| Web: the browser-based editor (`web/`), File System Access API + WASM buffer | Desktop: Workflows — a real `@xyflow/react` multi-CLI node graph |

More screens (Settings, Design/GUI Builder, Dev Containers, the web app's file tree and
live-editing states) are in `docs/screenshots/desktop/` and `docs/screenshots/web/`, and
embedded with full captions in `desktop/README.md` and `web/README.md`.

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
crates/spartan-model/       Real ModelProvider implementations — Ollama, Claude, LiteLLM,
                             llama.cpp (direct, in-process GGUF inference)
crates/spartan-git/         Real git integration (via libgit2) — status, stage, commit,
                             Leo's own checkpoint/rollback mechanism
crates/spartan-settings/    Real persisted settings (~/.spartan/settings.json)
crates/spartan-security/    Real secrets detection/redaction (credential-shaped regexes)
crates/spartan-crash/       Real local-first crash reporter (panic hook, redacted, no upload)
crates/spartan-updater/     Real "check for updates" against this repo's own GitHub API
crates/spartan-plugin-host/ Real WASM Component Model plugin host (wasmtime), capability-gated
crates/spartan-devcontainer/ Real OCI/Docker dev containers (containers.dev spec) via bollard
crates/spartan-android/     Real Android SDK/toolchain + project detection (first increment)
crates/spartan-editor-core/ The original wgpu-native shell — kept as the tested reference
                             implementation and backend proof-of-concept, not deleted; every
                             feature it proved was later promoted into the crates above

gui-builder/                Real, separate npm project — parses/edits JSX via Babel + recast,
                             bundles a live preview via esbuild, powers the Design screen
web/                        Real, separate Vite+React npm project — a vscode.dev-inspired
                             browser IDE, first increment: File System Access API + a real
                             WASM-compiled spartan-buffer, no LSP/DAP/Leo/git yet
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
- **Multi-provider**: switch Leo between local Ollama, Anthropic's Claude API, a local
  LiteLLM proxy, or direct in-process llama.cpp (point it at a local `.gguf` file, no
  separate server) — from Settings, no code changes. llama.cpp has real *native* tool
  calling too, via grammar-constrained sampling (a real GBNF grammar compiled from the
  tool schema forces the model's output to be structurally valid tool-call JSON) — see
  `crates/spartan-model/src/llamacpp.rs`'s own doc comment
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
- **Real Dev Containers** (OCI/Docker-based, following the open [containers.dev](https://containers.dev)
  `devcontainer.json` spec — the same one VS Code Dev Containers, GitHub Codespaces, and
  JetBrains Gateway use): detect a project's `devcontainer.json`, build or pull its image, start
  a real container with the project bind-mounted in, run its setup command, and open a real
  interactive shell into it — test a project's Linux environment in isolation without touching
  your host machine. Not a VM — genuinely different OS *families* (Windows/macOS guests) are out
  of scope by design; see [`crates/spartan-devcontainer`](crates/spartan-devcontainer)

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
  599 Rust tests across 15 real crates + 7 Tier 0 spikes + `xtask`, all passing; clippy and
  `cargo fmt` clean. The Electron shell's own TypeScript typechecks and builds clean, and
  every increment of it has been verified via a live, screenshotted Playwright pass driving
  a real Vite dev server against a test-only mock of the Electron preload bridge — see
  [Screenshots](#screenshots) above for real captures. GUI Builder's two-way AST sync is now
  fully closed (`Reparent`/`ComponentInsert` included), the last named Tier 1 gap for that row.
- **`web/` — a real, separate, first-increment browser IDE**: a vscode.dev-inspired
  Vite+React app running a real WASM compilation of `spartan-buffer` against the browser's
  File System Access API. No LSP/DAP/Leo/git yet — see [`web/README.md`](web/README.md)
  for exactly what's built and what's deliberately deferred.
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
- **Android (§21) is a real, narrow first increment, not first-class yet**: `crates/
  spartan-android` does real SDK/toolchain detection (`ANDROID_HOME`/`ANDROID_SDK_ROOT`,
  `adb`/`sdkmanager`/`avdmanager`/`emulator`) and real Android-project detection (the
  standard Gradle module layout), wired into `spartan-backend` as `android_detect`. No SDK
  install flow, emulator/device management, Compose LSP/preview, JDWP debugging, or UI
  surface yet — §35.9 itself names Android as Tier 1's biggest scope risk and explicitly
  sanctions shipping without it, which is exactly where this stands.
- **Not yet built**: a packaged/signed Electron installer, LSP/DAP/Leo/git connectivity for
  `web/` (pending an open token-delivery design question for its WebSocket transport), and
  the larger design-stage surface (§35 is the prioritized roadmap). Local-first crash
  reporting now has a real, user-triggered *upload* path too (never automatic) — see
  `crates/spartan-crash`.

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

### Web app (browser IDE)

```bash
cd web
npm install
npm run build:wasm   # compiles crates/spartan-buffer-wasm to WASM via wasm-bindgen
npm run dev          # real Vite dev server, http://localhost:5174
```

Needs `wasm-bindgen-cli` installed at the exact version pinned in `Cargo.lock`
(`cargo install wasm-bindgen-cli --version 0.2.126`). See [`web/README.md`](web/README.md)
for what's real (File System Access API + WASM-backed editing) vs. deferred (LSP/DAP/Leo/git).

## Repository layout

```
CLAUDE.md                    Index + behavioral contract for this repo
docs/architecture-spec.md    Full technical & design spec (source of truth, 75+ sections)
docs/screenshots/            Real Playwright + Chromium captures (desktop/ and web/)
desktop/                     Electron + React desktop shell — the current primary UI
gui-builder/                 Real, separate npm project — JSX AST sync + live preview
web/                         Real, separate npm project — vscode.dev-inspired browser IDE
crates/                      Real Rust product code (spartan-backend, spartan-buffer,
                              spartan-buffer-wasm, spartan-leo, spartan-model, spartan-git,
                              spartan-editor-core, and more — see Architecture above)
spikes/                      Real, tested Tier 0 Rust spikes + npm-based web-prep spikes
                              (tree-sitter-wasm-spike, git-browser-spike) — see
                              spikes/README.md
mobile/                      Spartan Mobile IDE — real Expo/React Native companion app
prototypes/                  Reference-only React UI mockups, not wired to anything
legacy/agent-deck-console/   Prior product, preserved for feature-parity reference (§55)
.github/workflows/           CI — fmt/clippy/test for Rust, typecheck/build/test for every
                              real npm project (desktop/, gui-builder/, mobile/, web/,
                              spikes/tree-sitter-wasm-spike, spikes/git-browser-spike)
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
