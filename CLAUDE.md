# Spartan IDE

From-scratch, agent-first desktop IDE. No VS Code/Monaco/CodeMirror — this is a locked
architectural decision, not an open question. Custom Rust rope + wgpu rendering engine,
`ropey` as the buffer foundation, tree-sitter for syntax, in-house LSP/DAP clients.

**Source of truth:** `docs/architecture-spec.md` (75 sections). This file is an index and a
behavioral contract, not a summary — read the relevant section before implementing anything,
don't guess from the section title.

This repository previously shipped a different product — an Electron-based "Agent Deck Console"
terminal launcher for third-party AI CLIs. That product was replaced by this from-scratch
architecture. Its real, working code is preserved at `legacy/agent-deck-console/` for reference
and is not on the build path for the new architecture; §55 is the traceability matrix mapping
its features into the new design. Don't delete `legacy/agent-deck-console/` without checking §55
first — it's the parity reference until each row there is actually reimplemented natively.

## Where things live

| Need to know about... | Read |
|---|---|
| Core engine (rope, renderer, LSP/DAP) | §2, §20 |
| Leo agent core, ModelProvider, LiteLLM, Ollama/HF | §3, §4, §41, §44 |
| Android, ADB, debugging | §21, §32, §33 |
| GUI builder (Design View, Open Design integration) | §6, §34, §38 |
| Engineering studio views (Test/Ops/Data/Manage) | §22–§26, §30 (Project Graph) |
| Security/trust hardening — READ BEFORE TOUCHING AUTH, SANDBOXING, OR APPROVAL FLOWS | §9, §36 |
| Settings taxonomy | §42 |
| Vibe Mode, CLI | §45, §46 |
| Prioritized roadmap (what's actually Tier 1 vs. later) | §35 |
| What's real vs. reference-only right now | §47, §48, §51 |
| External CLI fleet orchestration (replaces legacy Agent Deck console) | §52 |
| Neural Link workspace analysis bridge | §53 |
| Ops Cockpit web dashboard companion | §54 |
| Legacy feature parity matrix | §55 |
| Git & GitHub integration — Source Control panel | §56 |
| LM Studio — second local runtime | §57 |
| API Keys & Credentials settings | §58 |
| Terminal panel (fills a gap left open since §14) | §59 |
| Developer Mode — READ BEFORE ASSUMING IT DISABLES ANY §9/§36 INVARIANT | §60 |
| WSL & WSA integration | §61 |
| Slash commands, panel visibility | §62 |
| Skills — lightweight agent capability packages | §63 |
| MCP Server Management Panel | §64 |
| Playwright — live testing & visual debugging | §65 |
| CPU render fallback (no-GPU path) | §66 |
| High-contrast Antigravity-aesthetic theme, Antigravity 2.0 feature parity matrix | §50.3, §67 |
| Antigravity/VS Code extension manifest import (not a VS Code fork — READ §68 BEFORE ASSUMING EXTENSION COMPATIBILITY) | §68 |
| Spartan Mobile IDE — design (§69.1–§69.5) plus real implementation in `mobile/`, this repo (§69.6) | §69 |
| Import & Migration — projects/preferences from Cursor, Windsurf, Copilot, etc. | §70 |
| Leo chat — Antigravity parity (inline chat, Walkthrough artifact) | §71 |
| IoT & embedded development (boards, serial monitor, flashing, MQTT) | §72 |
| Security & Exploit Auditor — READ §73.2 BEFORE ASSUMING IT CAN TARGET ANYTHING BUT THE OPEN PROJECT | §73 |
| Open source decompilers (Ghidra, radare2, JADX, ILSpy, CFR/Fernflower) — untrusted-content posture in §74.7 | §74 |
| Real Tier 1 implementation begun (core buffer + language registry), what it does/doesn't mean | §75 |

## Current status (check this before assuming anything is built)

- **Real, working code — Tier 0 spikes**: `spikes/rope-spike`, `spikes/fallback-parser-spike`,
  `spikes/dap-spike`, `spikes/lsp-spike`, `spikes/render-spike`, `spikes/ui-shell-spike` — Rust,
  tested, run repeatedly on real (if modest) hardware. `dap-spike` drives real `lldb-dap` and
  `debugpy` subprocesses; `lsp-spike` drives real `rust-analyzer` and `pyright-langserver`
  subprocesses — both crates now proven against two independent adapters/languages each, not just
  one, closing the "registry replication" risk §47.6 flagged as open (fixing one real deadlock
  found only by testing the second adapter — see §47.7). No mocked adapter/server anywhere in
  either crate. Together they are the full DAP+LSP execution of Tier 0 Spike 0.2 (§39.2),
  documented in §47.5–§47.7. `render-spike` drives a real `wgpu`/`winit`/`glyphon` pipeline
  against real GPU hardware — see the GPU-half update below and §47.9–§47.10. `ui-shell-spike`
  drives a real `wgpu` shell plus a real embedded `wry`/WebView2 control in the same window — see
  the Spike 0.4 update below and §47.11. `fallback-parser-spike` now also drives a real local
  Ollama instance (`tests/real_ollama_fidelity.rs`, self-skips if Ollama isn't running) — see the
  Spike 0.3 update below and §47.12. Not the actual product, just Tier 0 risk-gate spikes.
  Also `legacy/agent-deck-console/` — a real, previously-shipped Electron/Node app, kept as a
  parity reference (§55), not part of the new architecture's build.
- **Real, working code — Tier 1 implementation begun (§75)**: `crates/spartan-buffer` (the real
  §2.1 document/buffer model — branching undo tree, bounded checkpoint ring, char-indexed edits
  safe against the multi-byte-boundary bug class §48 found once already) and
  `crates/spartan-languages` (the real §20.1 `LanguageProfile` registry — curated
  `languages.toml` seeded with exactly Tier 1's six languages per §35.4, marker-file project
  detection, extension-glob file matching). 15 and 10 tests respectively, all passing, clippy
  and fmt clean — real product code in `crates/`, deliberately not under `spikes/`, since these
  aren't go/no-go risk-gate experiments. Two real bugs were found and fixed only by running the
  tests, not by inspection (§75.2). This is the start of Tier 1's core-engine/language-registry
  work, not the IDE — no GPU rendering, no LSP/DAP wiring into the registry, no tree-sitter, no
  Leo, no UI. See §75.3 before assuming more is done than is.
- **Reference only, not implemented**: everything else. `prototypes/*.jsx` are React mockups of
  the intended UI — they demonstrate the interaction design, they are not the app. §52–§54 are
  design-only amendments written to fold the legacy console's features into this architecture;
  they have not been implemented against real third-party CLIs.
- **Spartan Mobile IDE (§69) is real, built in `mobile/`, this repo, this branch** — an
  Expo/React Native TypeScript app (five screens, §69.1's full v1 list and §69.5's full v1+v2
  list built, at three explicitly different confidence levels — see §69.6 and `mobile/README.md`
  for exactly what's Expo-Go-verified vs. custom-dev-client-only vs. a deliberate stub). It
  briefly lived in its own local repository before an explicit decision moved it into this one as
  a `mobile/` subdirectory via `git subtree`, preserving its real commit history rather than
  squashing it. No backend exists yet; never run on a device/emulator/simulator (none reachable
  in this environment). See §69.6 for the full history of this decision.
- **The "no GPU/display in this environment" assumption below was wrong for a later session's
  machine — a real GPU was reachable.** A standalone `wgpu` adapter/device probe succeeded
  against `Intel(R) UHD Graphics 620` (Vulkan, `IntegratedGpu`), and `spikes/render-spike`
  (§47.9–§47.10) now runs a real window/renderer/latency-benchmark against it, repeatedly. This
  is a **first increment, later deepened once, still not a closed spike** — a damage-region CPU
  shaping pass (§47.10) cut p50 latency from 169ms to ~3ms and p99 from 224ms to 6-12ms at 50k
  lines (down from missing the <5ms p99 target by ~45x to ~2x), but cold-open (~900-1300ms vs.
  <100ms) is untouched and GPU-upload cost remains real and unaddressed, which
  `spikes/render-spike/README.md` reports honestly rather than rounding away. **Spike 0.4 has
  also now run for the first time** (`spikes/ui-shell-spike`, §47.11) — a real `wgpu` shell plus
  a real embedded WebView2 control, with a real measured IPC round-trip (p50=2.3ms, well under
  the <50ms target) and a real ~180ms mode-switch fade. Two real integration gaps were found and
  fixed: a `WebView2Loader.dll` deployment gap specific to this project's GNU toolchain (fixed via
  `build.rs`), and a genuine keyboard-focus ownership conflict between the native shell and the
  WebView (fixed with a direct Win32 `SetFocus` call) — the latter a concrete instance of the
  exact "does this feel like one app" risk §39.4 exists to test. Don't assume either spike is
  closed just because both have now run — see their own READMEs for exactly what's still
  unconfirmed. **Spike 0.3 has also now gotten its first real local-model data** (§47.12): Ollama
  turned out to already be genuinely installed and running; a real `llama3.2:1b` model (1.2B
  params — smaller than §39.3's actual "~7B/13B class" targets, since disk space, ~11-12GB free,
  couldn't safely fit a 13B model) was pulled and driven against the real, already-tested
  `FallbackParser` (`spikes/fallback-parser-spike/tests/real_ollama_fidelity.rs`, self-skips if
  Ollama/the model aren't present). The result is real but not flattering: only 2/3 real tool-call
  attempts were even syntactically valid JSON, and **0/3 chose the semantically correct tool**
  (wrong tool name spelling once, wrong tool entirely once) — a small, largely negative data point
  at this model size, not a verdict on the 7B/13B class the spec actually targets. The parser
  itself had no bugs surfaced: real invalid JSON was correctly caught and surfaced, never dropped.
  Spike 0.2 (both halves), spike 0.1's CPU/data-structure half, spike 0.1's GPU half (partially),
  spike 0.4 (partially), and now spike 0.3 (partially, and mostly a negative result at this model
  size) are the spikes with real execution behind them. See §39 for what the remaining spikes
  need, §47.5–§47.6 for 0.2, §47.9–§47.10 for 0.1's GPU half, §47.11 for 0.4, §47.12 for 0.3.

## Build & test

```bash
cargo test --workspace --release   # 6 spikes + 2 real crates (spartan-buffer, spartan-languages)
# dap-spike needs `lldb-dap` (or `lldb-dap-18`) + `rustc`; lsp-spike needs `rust-analyzer` + `rustc`.
# Both self-skip with a printed message if their tool isn't found on $PATH.
# render-spike needs a real GPU + display to `cargo run`; its own headless unit tests (Document
# <-> render-input mapping) run fine under `cargo test` with neither.
# ui-shell-spike needs a real GPU + display + the WebView2 Runtime to `cargo run`; it has no
# headless tests of its own (everything it does is GPU/WebView-facing by nature).
cargo build --release --workspace
```

No other build system exists yet. If you're about to run `npm`/`pnpm`/anything else against
the `.jsx` prototypes, stop — they're standalone React artifacts (Tailwind + lucide-react),
not a project with its own build config. Don't invent one without discussing it first.

## Rules, not suggestions

- **Never claim something works without running it.** This project's own history includes real
  bugs caught only by actually executing code and adversarial-testing it (§48) — including two
  cases where a "fix" silently deleted content and looked correct until re-verified (§51.1).
  Match that discipline: run it, don't reason your way to "should work."
- **Don't fabricate benchmark numbers.** If you can't run something in this environment (no GPU,
  no display, no live model backend), say so explicitly instead of estimating and presenting it
  as measured. §47 is the template for how to report partial/blocked verification honestly.
- **Security hardening in §36 and §9 is not optional scope** — path-jailing, untrusted-repo
  quarantine, the Single Writer Invariant, External Content Fetch Gating (§50.2). These exist
  because of documented failures in comparable tools (Cursor, Windsurf, Antigravity — §36.2).
  Don't simplify them away for convenience during implementation.
- **The Security & Exploit Auditor (§73) only ever targets the currently open, user-owned
  project** — never a third-party host, never a URL outside that project's own configured
  local/staging environment. This is a structural refusal (§73.2), not a warning dialog. Every
  active-verification run needs its own explicit approval, even under Autonomous/Vibe autonomy
  (§45) — the same category of standing exception Untrusted-Repo Quarantine (§36.4.2) already
  carves out for a different risk. If a task description implies scanning or exploiting anything
  other than the open project, stop and re-read §73.2 before writing code.
- **"Developer Mode" (§60) never disables destructive-action approval or plugin sandboxing** —
  those stay hard invariants at every revision. Path-jailing has exactly **one** documented
  exception, made deliberately with the tradeoff surfaced and a specific scope chosen (§60.2.1,
  amending §36.4.6) — it is not a template for other invariants to grow exceptions. If a task
  description says "full system access" or similar, re-read §60 in full (especially §60.2.1)
  before writing code; don't reason from the feature name or from this bullet's summary alone.
- **§35 is the actual build order.** Don't jump ahead to Tier 2/3 features (full multi-language
  support, enterprise SSO, the plugin marketplace) before Tier 0's remaining spikes and Tier 1's
  MVP scope are real. If asked to build something, check §35 for what tier it's in and say so
  if it's premature.
- Before writing code for a new subsystem, check whether a Tier 0 spike already exists for the
  risky part of it (§39). Don't re-litigate an already-validated design decision from scratch.
- **Visual simplification/decluttering never removes the terminal, inline diagnostics, or
  direct in-place code editing** (§36.4.10) — a real Antigravity 2.0 minimalism regression
  reportedly did exactly that and alienated users who wanted an IDE, not a chat window (§36.2).
  Decluttering targets secondary chrome (redundant borders, boxed badges, duplicate labels)
  and generous whitespace, never a core editing/debugging surface. This is permanent, not a
  one-time judgment call for whichever pass introduced it.

## What NOT to do

- Don't fork or vendor any VS Code/Monaco/CodeMirror code, ever, for any reason.
- Don't add a new cloud model provider as a bespoke adapter — it goes through LiteLLM (§44)
  unless there's a specific reason (like Claude's prompt caching) to hand-roll it, as already
  decided for `ClaudeProvider`/`OllamaProvider`.
- Don't touch `docs/architecture-spec.md`'s section numbering without re-running the structural
  check §51.1 used (sequential 1–N, no duplicate headers, cross-references resolve) — that
  exact bug happened once already in this project's own history.
