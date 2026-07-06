# Spartan IDE

**A from-scratch, agent-first, multi-OS engineering studio** — not a VS Code/Monaco/CodeMirror
fork, not an Electron wrapper around another editor. Custom Rust rope buffer + `wgpu` GPU
renderer (with an honest CPU fallback), tree-sitter for syntax, in-house LSP/DAP clients, a
hybrid Claude + Ollama/LM Studio agent named **Leo**, a two-way-synced GUI builder, and
first-class support for every major language, build system, debugger, and testing framework —
one coherent product, not a patchwork of bolted-on panels.

This is a **design-and-plan pass with real, tested Tier 0 engineering underneath it**, and this
README says exactly which parts are which rather than blurring the line. See
["What's actually real right now"](#whats-actually-real-right-now) before assuming anything
beyond that.

## Why from scratch

Forking VS Code/Monaco is the fast path every other AI-native editor in this space has taken
(Cursor, Windsurf, Antigravity IDE) — and it's also where their documented failure modes come
from: forced app splits, extension-host isolation gaps, editor surfaces that can't be redesigned
because they're not really yours. Spartan's architecture spec (§36) catalogs these failures by
name, root-causes each one, and treats "own the rope buffer and the renderer" as the
precondition for actually fixing them rather than working around someone else's editor. That's
a locked decision (`CLAUDE.md`), not an open question revisited per feature.

## What it actually is

### Leo — the agent at the center

- **Real planning, tool-calling, checkpointing, and artifact-based trust**: every task produces
  an Implementation Plan, a live Task List, per-file Diff Cards with Accept/Reject, a
  Verification record (test results, not vibes), and a Walkthrough recap after the fact — each
  one **commentable like a doc** (§67), not a raw tool-call log you scroll through
- **Two chat surfaces, deliberately distinct** (§71): a side panel for conversations and broad
  changes, and inline chat (`Cmd`/`Ctrl`+`I`) scoped to the exact line you're on — researched
  against Antigravity 2.0's actual design rather than guessed
- **Four autonomy levels** — Manual, Plan-Approve, Autonomous, and Vibe Mode (§45) — plus
  plan-scope enforcement: an agent that wants to touch files outside its declared plan needs a
  separate, visible Scope Expansion approval, never a silently widened diff (§36.4.3)
- **Checkpoint time travel**: rope snapshots measured at ~0.0002 ms in the Tier 0 spike (§47.1)
  make scrubbing between implementation attempts — each run against the real test suite —
  effectively free (§49)
- **Layered memory** (Project / Global / Team) stored as real, editable markdown — not a black
  box you can't audit (§4.3)
- **Hybrid model routing**: hand-rolled `ClaudeProvider` (for prompt caching) and local
  providers for **Ollama and LM Studio** (§57), everything else through **LiteLLM** (§44) with
  fallback chains — plus a **Hugging Face model downloader** (§41) and a curated local-model
  manifest. A local model's tool calls go through an adversarially-tested streaming fallback
  parser (§3.4) — one of the four real, tested Rust spikes in this repo
- **Model Integrity Guarantee** (§36.4.5): the model badge on every response is populated from
  the provider that actually executed the call — there is structurally no code path where the
  label and the reality diverge, because a competitor shipped exactly that bug
- **Pre-flight cost estimates** (§36.4.4): estimated cost shown on the plan itself before an
  expensive task runs, not discovered on an invoice afterward

### The core engine

- **Custom Rust rope buffer** (`ropey`-based) + **wgpu GPU renderer** with damage-region
  rendering — plus an honest **CPU software-rasterizer fallback** (§66) that reuses existing
  backends rather than rebuilding one, states plainly that it won't hit GPU-path latency
  targets, and auto-simplifies motion the same way `prefers-reduced-motion` does
- **Tree-sitter** for syntax — any language gets highlighting immediately, even before full
  LSP/build support is configured
- **In-house LSP and DAP clients** — no third-party protocol crates — already proven in this
  repo against two independent real servers/adapters each (`rust-analyzer` + `pyright`,
  `lldb-dap` + `debugpy`), which caught a real cross-adapter deadlock that testing only one
  would never have surfaced (§47.7)

### Every target is first-class — not just one headline platform

- **Desktop**: Windows, macOS, Linux — one codebase, no Electron
- **Android**: full ADB command surface (§33), scrcpy-backed screen mirroring, Compose preview
  rendering in an isolated auto-restarting subprocess, wireless pairing
- **The web**: a live, driveable **Playwright browser panel** (§65) — click-to-inspect feeds
  selectors/styles to Leo like a stack trace, interactions record into a real reviewable test
  script, and Leo can drive the browser itself to verify its own work
- **Windows subsystems**: WSL distros as workspace roots and WSA as a device target (§61) —
  not shelling out and hoping
- **Remote & CI**: the Spartan CLI (§46) — one Leo across desktop and terminal, shared session
  store, headless mode with a committable non-interactive approval policy, `spartan mcp serve`
  to expose a project as an MCP endpoint, and piping (`... | spartan explain`)
- **Your phone**: the Spartan Mobile IDE companion app (§69), real and built in `mobile/` — an
  Inbox mirror, biometric-gated approve/reject, chat, and the edge-first features from §69.5:
  an offline review queue with conflict-safe queued decisions, edge-cached repo context,
  notification-surface actions, camera and voice-to-task capture. On-device model Q&A stayed a
  deliberate, honestly-labeled stub rather than a faked integration — see §69.6

### Languages, builds, debuggers, tests

- **~40 language profiles out of the box** via the pluggable `LanguageProfile` registry (§20.1)
  — LSP command, DAP adapter, build systems, formatter, grammar — with auto-detection from
  marker files, not hardcoded special cases
- **Default formatters named per language** and enforced on save if you want: **Prettier** for
  the JS/TS/web ecosystem, `rustfmt`, `black`, `ktlint`, `gofmt` (§20.1.1) — applied for real
  to this repo's own code, both Rust and JSX
- **One Task model for every build system** (§20.2) — cargo, gradle, cmake, maven, npm, and the
  rest normalize into the same async, never-blocks-the-editor abstraction
- **Debuggers across platforms** (§32): LLDB, debugpy, Delve and more behind the same in-house
  DAP client, with rope-anchored breakpoints that survive edits (tested, §47.5)
- **Test Studio** (§24), plus a **Language Profile Conformance Certifier** (§19): new
  language-profile adapters get a scripted conformance probe against the real binary before
  being trusted — a standing regression gate born from a real bug this project actually hit
- **IoT & embedded development** (§72): a board/toolchain registry (ESP32, Arduino, Raspberry Pi
  Pico, STM32, nRF, Particle) defaulting to PlatformIO, a Serial Monitor as a real Devices-panel
  tab, flashing as a normal Task with OTA updates explicitly gated as network-capable, an MQTT
  Inspector, and RTOS-aware debugging (named FreeRTOS/Zephyr threads, not raw stack pointers)

### The studio around the editor

- **Test / Ops / Data / Manage views** (§22–§26) — CI/CD, notebooks and experiment tracking,
  ticket workflows — unified by the **Project Graph** (§30), which ties tickets, tests, and
  deploys to the code that touches them (visible as a link strip right on the open file)
- **Source Control panel** (§56) with local git and GitHub PRs — plus **GitHub fully integrated**
  via credentialed API access (§58) rather than screen-scraping a web view
- **A real terminal panel** (§59) with natural-language-to-command and a dry-run preview —
  and a permanent guarantee that no decluttering pass ever removes it (§36.4.10)
- **A workspace you can pare down yourself** — every panel (left rail, file tree, plan tracker,
  auxiliary pane) carries its own hide button, with one central View menu as the way back
  (§62); slash commands and a command palette cover the keyboard-first path
- **Design View**, a two-way-synced visual GUI builder (§6, §34): canvas and code are one
  source of truth via structured `CanvasEdit` events into the same rope pipeline as every other
  edit — not a preview that drifts from what ships — with Open Design integration (§38)
- **A high-contrast, Antigravity-2.0-researched theme** (§50.3) — tokens matched to real, cited
  values, decluttering scoped to secondary chrome only, permanently barred (§36.4.10) from
  repeating Antigravity's own documented regression of stripping the terminal, inline
  diagnostics, and direct editing

### Extensibility without a fork

- **WASM Plugin API** (§5): capability-sandboxed by construction (a plugin without `network`
  in its manifest has no network import to call), per-plugin CPU/memory budgets, and a
  marketplace performance gate (§36.4.9) — plugins can even register new tools for Leo, subject
  to the same approval flow as built-ins
- **Skills** (§63): lightweight markdown+script capability packages — team conventions,
  debugging recipes — installed, imported, or from a marketplace, without compiling anything
- **MCP server management** (§64): connect stdio/SSE/HTTP servers with per-tool allowlists and
  health checks; adding one is a security-relevant, approved change — not a config file edit
  nobody sees
- **Antigravity/VS Code extension manifest import** (§68): converts an extension's static
  contributions (commands, keybindings, config schemas, snippets) into a real sandboxed plugin,
  with a per-capability conversion report that names what could and couldn't come over — never
  a silent partial success, and never a vendored extension host
- **Import & Migration** (§70): detects other AI tools' config in your project (Cursor,
  Windsurf, Copilot, Cline, Continue, Aider, Claude Code) and converts what's real — rules
  files become Skills, MCP servers carry over near-losslessly, themes and keymaps map onto
  Spartan's own. One hard rule: an imported auto-approval/"YOLO" posture **never** applies
  silently, no matter the source

### Trust, security, and cost — designed from documented failures

Spartan's hardening (§9, §36) starts from a named failure catalog across Cursor, Windsurf,
Antigravity, JetBrains, VS Code, Eclipse, Xcode, and Android Studio — each failure root-caused
and mapped to a concrete mechanism:

- **Single Writer Invariant** (§36.4.1): every edit — agent, canvas, or keystroke — goes through
  one write lock; concurrent writes become a visible Conflict artifact, never a silent revert
- **Untrusted-repo quarantine** (§36.4.2): unfamiliar repos get no auto-run, no auto-approval,
  no secrets access until explicitly trusted — even if your global setting is "autonomous"
- **Path-jailing + MCP/plugin registration approval** (§36.4.6) and **external-content fetch
  gating** (§50.2): rendered external references are never auto-fetched just because they're
  displayed
- **Developer Mode** (§60): the one deliberate, user-confirmed exception to path-jailing —
  scoped in the open, with two hard stops (destructive ops and first-write-outside-project
  still confirm) that survive at every revision, and explicitly not a template for other
  invariants to grow exceptions
- **Settings you can audit**: four visible resolution layers (System / Global / Project /
  Session) on every row, one-click presets with a diff shown before applying, settings-as-code
  in the repo, and a change history — plus a settings surface for every subsystem above,
  24 categories deep, including **API keys & credentials** with OS-keychain-backed storage (§58)

### Security research: exploit auditing and decompilation

Scoped, defensive tooling — never a bolted-on afterthought:

- **Security & Exploit Auditor** (§73): verifies whether a static finding (SQLi, XSS, SSRF, IDOR,
  a flagged CVE, a hardcoded secret) is actually exploitable against a locally-running instance
  of *this project only*, structurally refusing any third-party host — never a warning dialog,
  the same posture path-jailing already takes toward the filesystem. Every active-verification
  run needs its own explicit approval, even under Autonomous/Vibe autonomy. Verified findings
  become normal reviewable diffs, never auto-applied fixes
- **Open source decompiler integration** (§74): Ghidra as the default engine (broadest
  architecture coverage of any open source decompiler), radare2 for fast triage, and
  JADX/CFR-Fernflower/ILSpy tied to the language/platform support Spartan already has. Any
  binary that isn't a build artifact of the open, trusted project is treated as untrusted
  content by construction — the same Quarantine posture §36.4.2 already applies to repos,
  reused rather than redesigned

### The External Agent Fleet

Sixteen third-party CLI engines — Claude Code, Codex, Gemini CLI, Aider, OpenCode, Cursor
Agent, Cline, Qwen Code, Copilot CLI, Windsurf, and more — supervised as managed sessions
(§52) with per-engine fallback chains on quota exhaustion, usage tracking, and a periodic
health self-check that catches a silently-broken CLI before you find out mid-task. Honest by
construction: a Fleet session is process-supervised, not token-supervised, and the UI says so
rather than fabricating plan artifacts a third-party CLI never produced. This carries forward
everything this repo's prior product could do — natively, per the §55 parity matrix.

### Companion surfaces

- **Ops Cockpit** (§54): a read-only web dashboard for second-screen monitoring of fleet and
  agent activity
- **Neural Link** (§53): local workspace-analysis reports bridging repo state into the agent's
  planning context
- Both design-stage, both scoped and security-reviewed in the spec before a line is written

Full detail on all of the above — and everything else — lives in
[`docs/architecture-spec.md`](docs/architecture-spec.md), 75 sections and growing, each one
cross-referenced rather than left to imply more than it says.

**This repository previously shipped a different product** — an Electron-based "Agent Deck
Console" terminal launcher for third-party AI CLIs. That product was replaced by this
from-scratch architecture, not deleted: its real, working code is preserved unmodified at
[`legacy/agent-deck-console/`](legacy/agent-deck-console/), and
[`docs/architecture-spec.md` §55](docs/architecture-spec.md) is the exact traceability matrix
mapping every legacy feature to its new home in this architecture.

## Source of truth

[`docs/architecture-spec.md`](docs/architecture-spec.md) is the spec — read the relevant section
before implementing anything, don't guess from a section title. [`CLAUDE.md`](CLAUDE.md) is the
index into it and the behavioral contract for working in this repo, including the hard invariants
(security hardening, no VS Code forking, honest verification) that hold at every revision.

## What's actually real right now

This is the section that keeps the rest of this README honest. Two categories, and nothing in
between:

- **Real, working, tested Rust code**: [`spikes/rope-spike`](spikes/rope-spike),
  [`spikes/fallback-parser-spike`](spikes/fallback-parser-spike),
  [`spikes/dap-spike`](spikes/dap-spike), and [`spikes/lsp-spike`](spikes/lsp-spike) — Tier 0
  risk-gate spikes for the rope buffer's performance characteristics, the local-model tool-call
  fallback parser, and in-house DAP/LSP clients proven against **two independent
  adapter/language pairs each** (`lldb-dap`+`rust-analyzer` for Rust, `debugpy`+`pyright` for
  Python): full breakpoint-hit-and-inspect and diagnostics/completion/hover cycles, a
  rope-anchored breakpoint surviving a line-shifting edit, a debounced-didChange dispatcher, a
  real cross-adapter DAP sequencing bug found and fixed, and a real intermittent hover-timing
  race found and fixed during a later audit pass. See spec §47.5–§47.8 for exactly what was run
  and found. Run it yourself:

  ```bash
  cargo test --workspace --release   # 95 tests: 6 spikes + 3 real crates below
  cargo clippy --workspace --all-targets --release   # clean
  cargo fmt --check                  # clean
  cargo build --release --workspace
  ```

- **Real, working, tested Rust code — Tier 1 implementation begun**:
  [`crates/spartan-buffer`](crates/spartan-buffer) and [`crates/spartan-languages`](crates/spartan-languages)
  (§75) — deliberately under `crates/`, not `spikes/`, since these are product code, not Tier 0
  risk-gate experiments. `spartan-buffer` is the real §2.1 document/buffer model: a branching
  undo tree (not a linear stack — you can jump back to an abandoned branch directly), a bounded
  checkpoint ring, and char-indexed edits that can't split a multi-byte character by
  construction. `spartan-languages` is the real §20.1 `LanguageProfile` registry, seeded with
  exactly Tier 1's six launch languages (§35.4) and able to detect a genuinely polyglot project.
  15 and 10 tests respectively. Two real bugs were found only by running the tests, not by
  inspection — see §75.2 for both.

- **Real, working code — [`crates/spartan-editor-core`](crates/spartan-editor-core) (§75.5)**:
  the first crate combining `spartan-buffer`, a promoted-and-improved copy of `render-spike`'s
  real GPU rendering, and `spartan-languages` in one real file open. Adds viewport
  virtualization — cosmic-text's buffer now only ever sees the visible ~34-60 lines, never the
  whole document — which drops cold-open at 50k lines from `render-spike`'s 897.7-1297.9ms to
  575.5-617.5ms (still ~6x over the <100ms target, not closed) and gets realistic cursor-adjacent
  edit p99 to 3.5-3.9ms, reliably under §39.1's 5ms target where `render-spike` wasn't. Scrolling
  is a new, real, unaddressed cost (p99 19.4-29.2ms) never measured before. 14 headless tests plus
  real screenshot/synthetic-input visual verification — see its own README and §75.5 for the full,
  honest before/after numbers and what's still not built (no auto-scroll-to-cursor, no
  tree-sitter, no real LSP/DAP spawning, no UI chrome).

- **Reference-only interaction design**: [`prototypes/interface-prototype.jsx`](prototypes/interface-prototype.jsx)
  and [`prototypes/signature-features.jsx`](prototypes/signature-features.jsx) — standalone React
  mockups (Tailwind + lucide-react, Prettier-formatted) demonstrating the intended UI down to real
  interactive depth: every settings category, the Playwright live-browser panel, inline chat, the
  full Fleet roster. They demonstrate the interaction design; they are not the app, have no build
  config of their own, and nothing here executes real agent logic, a real LSP/DAP session, or a
  real GPU frame.
- **Preserved for reference**: [`legacy/agent-deck-console/`](legacy/agent-deck-console/) — the
  prior product, a real Electron/Node/Python app that orchestrated third-party AI CLIs with usage
  tracking, auto-failover, and a web cockpit. Kept as the working parity reference until §52–§54
  of the new architecture are actually implemented natively.

Everything else in the spec — the GPU renderer, Leo's full agentic loop, the GUI builder,
Android support, the debugger, the Fleet/Neural Link/Cockpit subsystems — is specified, not
built. (Spartan Mobile IDE is the exception: real, built, in `mobile/` — see above.) See
`CLAUDE.md`'s "Current status" section and spec §35 (prioritized roadmap) and §47/§48/§51
(honest build/verification log) before assuming otherwise. This
project's own history includes real bugs — a UTF-8 char-boundary panic, a cross-adapter DAP
deadlock, an intermittent LSP race — found only by actually running code and trying to break it,
not by reasoning that it should work (§48, §51).

## Repository layout

```
docs/architecture-spec.md   Full technical & design spec (source of truth, 75 sections)
docs/architecture-spec.SNAPSHOT-2026-07-04-pre-implementation.md
                             Frozen copy of the spec taken before implementation began (§75.4)
CLAUDE.md                   Index + behavioral contract for this repo
spikes/                     Real, tested Tier 0 Rust spikes (see spikes/README.md)
crates/                     Real Tier 1 product code (spartan-buffer, spartan-languages,
                             spartan-editor-core — §75, §75.5)
mobile/                     Spartan Mobile IDE — real Expo/React Native companion app (§69.6)
prototypes/                 Reference-only React UI mockups
legacy/agent-deck-console/  Prior product, preserved for feature-parity reference (§55)
.prettierrc.json            Prettier config — applied for real to the .jsx prototypes
Cargo.toml / Cargo.lock     Rust workspace covering the spikes and crates (not mobile/ — separate
                             Node/Expo toolchain, see mobile/README.md)
```

## Getting started

```bash
git clone <this-repo>
cd Spartan_IDE
cargo test --workspace --release    # exercises all six Tier 0 spikes + 3 real crates
```

`dap-spike` needs `lldb-dap` (or `lldb-dap-18`) + `rustc`; `lsp-spike` and
`spartan-editor-core`'s own `lsp_integration.rs` need `rust-analyzer` + `rustc`. All self-skip
with a printed message — not a failure — if their tool isn't found on `$PATH`; see
[`spikes/README.md`](spikes/README.md) for exactly which optional tools unlock which tests, and
why testing against a *second* real adapter (not just the first) is the whole point. Three
suites now spawn real language-server/debug-adapter subprocesses (`dap-spike`, `lsp-spike`,
`spartan-editor-core`'s `lsp_integration.rs`) — under `cargo test`'s default full parallelism
this can occasionally produce a resource-contention flake in one of them; retry with
`-- --test-threads=1` before assuming a real regression (see `CLAUDE.md`).

There is no build system for the `.jsx` prototypes beyond Prettier formatting — don't invent one
without discussing it first (see `CLAUDE.md`).

## Contributing / where to start reading

1. Read [`CLAUDE.md`](CLAUDE.md) in full — it's short, and it's the contract, not a suggestion.
2. Check [`docs/architecture-spec.md` §35](docs/architecture-spec.md) for what's actually next in
   the build order before picking up new scope — Tier 0's remaining spikes and Tier 1's MVP come
   before anything later, regardless of how interesting a Tier 2/3 idea sounds.
3. If you're touching security, sandboxing, or approval flows, read §9 and §36 first — they exist
   because of documented, named failures in comparable tools, not hypothetically.
