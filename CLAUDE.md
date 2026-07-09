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
  tests, not by inspection (§75.2). This was the start of Tier 1's core-engine/language-registry
  work, not the IDE — no GPU rendering, no LSP/DAP wiring into the registry, no tree-sitter, no
  Leo, no UI, as of that pass. See §75.3 for what was true then; §75.5 (below) is what's changed
  since.
- **Real, working code — `crates/spartan-editor-core` (§75.5)**: the first real crate combining
  `spartan-buffer`, a promoted-and-improved copy of `render-spike`'s GPU rendering, and
  `spartan-languages` in one real file open. Adds **viewport virtualization** (`Viewport` +
  `windowed_text()`) — cosmic-text's `Buffer` now only ever sees the visible ~34-60 lines, never
  the whole document — the literal fix for the cold-open gap `render-spike`'s own report named as
  untouched. Real, run-not-estimated numbers against the same 50k-line fixture and hardware
  `render-spike` used: cold-open dropped from render-spike's 897.7-1297.9ms to 575.5-617.5ms
  (~1.6-2.2x faster, but still ~6x over the <100ms target — not closed); a new cursor-adjacent
  typing benchmark (added because the random-position one mostly measures a near-zero off-window
  no-op at this document/viewport ratio, an honest caveat, not glossed over) shows edit p99 at
  3.5-3.9ms, reliably under §39.1's 5ms p99 target where render-spike's own report said that
  target was "not reliably met." Scrolling is a new, real, unaddressed cost never measured before
  (p99 19.4-29.2ms). 14 new headless tests, real visual verification via screenshot +
  `enigo`-synthetic keyboard input (real typing, real caret position, real scroll and scroll-back
  confirmed on screen), clippy/fmt clean. No auto-scroll-to-cursor, no tree-sitter, no real
  LSP/DAP spawning, no Leo, no UI chrome — see §75.5 for the full list of what this still isn't.
- **Real, working code — real LSP wiring in `crates/spartan-editor-core` (§75.6)**: a real, live
  `rust-analyzer` session for the open file, promoted verbatim from `spikes/lsp-spike`'s already-
  proven `LspClient`/`DidChangeDebouncer` (`src/lsp.rs`), orchestrated by a genuinely new
  `LspSession` (`src/lsp_session.rs`) that runs the entire session — spawn, initialize/didOpen,
  every didChange dispatch — on its own OS thread so `LspClient`'s up-to-90s blocking calls never
  freeze the render loop; the 150ms debounce timer itself stays on the render thread, handing the
  background thread only a single-slot mailbox snapshot (`Mutex`+`Condvar`, not a channel) so a
  burst of debounce firings during a long indexing wait can never queue a stale backlog. Diagnostics
  are printed to stdout (no UI exists yet, same pattern as detected-language printing). A real,
  load-bearing fix beyond the spike's own test coverage: `wait_real_diagnostics` only ever resolves
  on a *non-empty* array by design, so live edits use `wait_notification` directly instead —
  otherwise a fixed error could never be reported as fixed. Real, executed verification: a new
  self-skipping integration test spawns real `rust-analyzer` against a generated fixture with a
  deliberate `E0308` error, confirms the real diagnostic, then confirms it really clears to empty
  after a corrected snapshot (2.49s wall-clock, first real exercise of `did_change_full`, which the
  spike's own tests never called) — plus a live binary run (screenshot + `enigo` input) confirming
  the same end-to-end on screen. The 50k-line `--synthetic:` benchmark was re-run afterward and
  showed no regression (LSP never spawns for synthetic fixtures). DAP wiring, hover/completion,
  tree-sitter, Leo, and any diagnostics UI remain unbuilt — see §75.6 for the full list, including
  the single-file-mode fallback and the ~7s worst-case shutdown-close freeze, both named rather than
  silently absorbed.
- **Real, working code — auto-scroll-to-cursor + resize-aware viewport in `crates/spartan-editor-core`
  (§75.7)**: closes two limitations §75.5 named explicitly. `Viewport::ensure_visible()` scrolls
  minimally to keep the cursor on screen after an edit; `WindowEvent::Resized` now recomputes
  `visible_lines` from the new window height (previously fixed at startup only) and reshapes.
  Real visual verification: 45 scripted `Enter` keypresses confirmed via screenshot to keep the
  caret visible at the bottom of the window instead of scrolling off-screen; a real Win32
  `SetWindowPos` resize confirmed via screenshot to reflow content with the caret still correctly
  positioned. The 50k-line benchmark was re-run and shows no regression (these fixes deliberately
  don't touch the benchmark's scripted edit paths). One test-writing mistake caught by actually
  running it, not by inspection: an early version of a clamp test assumed a scenario the clamp
  logic can't actually reach for any valid input, worked out by hand and fixed — see §75.7.
- **Real, working code — real DAP wiring in `crates/spartan-editor-core` (§75.8)**: real
  breakpoints, a real hit, real continue/step commands, real stack/variable inspection, promoted
  from `spikes/dap-spike`'s already-proven `DapClient` (`src/dap.rs`, plus two new methods
  `step_over`/`step_into`), orchestrated by a genuinely new `DapSession` (`src/dap_session.rs`)
  that deliberately does NOT reuse `LspSession`'s mailbox — every debug command is discrete and
  ordered, none may be dropped, so it uses a plain `mpsc::channel` instead. Two real, deliberate
  scope cuts named up front rather than discovered later: a pre-built `--debug-binary:<path>` CLI
  arg instead of real build-system integration (`dap_command` only names the adapter, not how to
  build a debuggable binary — that's unmodeled in the registry entirely), and line-number
  breakpoints instead of rope-anchored persistence (the exact §39.2-sanctioned v1 fallback, since
  wiring the proven rope-anchoring would need byte-level edit details this crate's public API
  doesn't expose yet). A real environment constraint hit during verification: `lldb-dap` isn't
  installed on this machine, so a second real test against `debugpy` (mirroring `dap-spike`'s own
  cross-language check) verifies the identical code path instead — real breakpoint hit, real
  `StepOver` landing on the correct line, real `Continue` to a real exit, all genuinely executed.
  A second real, unplanned finding from trying this: `spartan-languages`'s own Python
  `dap_command` (`"debugpy"`) isn't directly invocable without a wrapper script `main.rs`'s
  registry-driven dispatch doesn't generate — found by testing, documented, not fixed in this
  pass. Live verification confirmed `F9`/`F5` work correctly through the real binary, including
  graceful (non-crashing) error reporting when no adapter can be spawned. The 50k-line benchmark
  was re-run and shows no regression. DAP UI, build integration, rope-anchored breakpoints, and
  `step_into` test coverage remain open — see §75.8 for the complete list.
- **Real, working code — cold-open investigation in `crates/spartan-editor-core` (§75.9)**: a
  real, permanent per-step timing breakdown (added to `main.rs`, not a one-off) found
  `GpuState::new()`'s wgpu instance/adapter/device setup, not text shaping, to be cold-open's
  single largest cost (~220-433ms). A hypothesis-driven fix — restricting `wgpu::Instance::new()`
  to `Backends::VULKAN` only, since every run of this project on every machine so far has used
  Vulkan — was implemented, measured across 5 runs, found to make no real difference, and
  reverted rather than kept as unproven complexity: a real negative result, reported honestly, not
  hidden. A second fix did help for real: `FontSystem::new()` (cosmic-text's font scan, ~93-97ms)
  now runs on its own thread concurrently with `GpuState::new()`'s async setup, since it has no
  actual GPU dependency — `TextState::new()` dropped to ~2-2.5ms and cold-open dropped to a
  467-715ms range from ~570-810ms. A real methodological finding along the way: an apparent
  regression during verification (cursor-adjacent p99 jumped to 4.65-4.73ms, scroll to 41ms) was
  investigated via a controlled A/B/A/B comparison against the prior committed binary rather than
  assumed either way, and turned out to be transient system noise from rapid rebuild-and-run
  cycles, not a real code effect. §39.1's <100ms cold-open target remains far from met
  (~4.7-7.2x over) — `GpuState::new()` is now the clear, dominant, unaddressed remaining cost.
- **Real, working code — real DAP build-system integration in `crates/spartan-editor-core`
  (§75.10)**: closes §75.8's named gap for Cargo. `src/build.rs`'s `build_debug_binary()` runs a
  real `cargo build --message-format=json` (its exact JSON shape confirmed by running a real
  build, both success and a real compile error, before writing any parsing code) and returns the
  real resulting binary path or real rendered diagnostics. `F5` now runs this on its own thread
  whenever no explicit `--debug-binary:` was given but a real Cargo project is discoverable, never
  blocking the render loop. A small bundled fix: `DapSession::is_finished()` lets `F5` tell a
  genuinely-over session apart from a live one, so pressing it again after a session ends
  correctly starts fresh instead of silently failing to `Continue` a dead session. Two new tests
  run a real `cargo build` against generated fixtures (real successful build reports a real,
  existing executable path; a real type error reports the real `E0308` diagnostic). Live, through
  the actual binary: `F9`/`F5` triggered a real `cargo build` that succeeded, found the real
  executable (confirmed on disk), and attempted a real DAP launch with it — failing gracefully
  with the same honest error as §75.8, since `lldb-dap` still isn't installed here. The 50k-line
  benchmark was re-run and shows no regression. Only Cargo is wired; other build systems still
  need an explicit `--debug-binary:<path>`.
- **Real, working code — real tree-sitter syntax highlighting in `crates/spartan-editor-core`
  (§75.11)**: closes the standing, named "tree-sitter stays unwired" gap from §75.5-§75.10, for
  Rust first. Deliberately windowed, not whole-document — reading `tree_sitter_highlight::
  Highlighter::highlight()`'s real installed source showed its public API always scans its entire
  input with no cheap sub-range option, so `src/highlight.rs` only ever parses the same ~34-60
  line visible slice everything else in this crate uses; a real, named consequence is that a
  multi-line construct starting above the window is misinterpreted within it. The per-line fast
  path is bypassed entirely for highlighted files (a line can't be correctly re-highlighted in
  isolation), routing every edit through a full windowed re-parse instead. A real bug was caught
  by the live screenshot itself: numeric literals first rendered uncolored because tree-sitter-
  rust's own bundled query captures them as `@constant.builtin`, never `@number` — found by
  reading the real `.scm` file, fixed by renaming the configured highlight name to `"constant"`,
  locked in with a new test. Real, honestly-measured cost of the fast-path bypass on the same
  899-line real file, with vs. without highlighting (same content, unrecognized extension so no
  `Highlighter` attaches): highlighted p50 5.2-5.9ms / p99 28.7-34.3ms vs. unhighlighted p50
  2.8-3.1ms / p99 5.8ms — a real ~1.9x/~5x cost, reported plainly. The 50k-line benchmark was
  re-run and, after an A/B against the prior committed binary showed the day's own noise baseline
  had simply drifted (matching §75.9's own methodology for telling drift from a real regression),
  shows no regression attributable to this pass. A second real bug was found in the benchmark
  harness itself (not shipped product code): passing literal `0` for a scripted phase meant to be
  skipped makes it report complete and exit immediately instead, silently cutting off any
  later-ordered phase — skipping a phase requires omitting its argument entirely, not passing `0`.
  Only Rust is wired, only six capture names are themed, and no incremental re-parsing exists yet.
- **Real, working code — first Linux-container verification pass, a real cross-platform regression
  found and fixed (§75.12)**: every prior §75.x pass ran on Windows with a real GPU; this pass ran
  in a Linux container with no GPU/display at all (confirmed via `/dev/dri`, not assumed), so it's
  a pure `cargo test --workspace --release` + clippy/fmt verification pass, not a feature increment.
  Two real, environment-specific (not code) build gaps were found and fixed by installing system
  packages (`libgtk-3-dev`, `libwebkit2gtk-4.1-dev`) before the workspace would even compile — no
  prior Windows session had ever surfaced them. Once compiling, the full suite passed for real,
  including — new territory for this project — a machine with a real, installed `lldb-dap-18`.
  Installing `debugpy` too (to get both real DAP adapters into one session for the first time)
  surfaced a real regression: `spartan-editor-core`'s `dap_python_cross_language.rs` had silently
  dropped `dap-spike`'s own `#[cfg(not(windows))]` wrapper-script branch when §75.8 ported it,
  leaving only the Windows `.cmd` version — which has no executable bit and no interpreter on
  Linux, so the test panicked for real instead of exercising breakpoint/step/continue. Invisible
  until now because no prior session had both a non-Windows environment and a real `debugpy`
  install at once. Fixed by restoring `dap-spike`'s original platform split verbatim; re-ran clean,
  including the `StepOver`/`Continue`-to-exit assertions the test already had but had never
  actually exercised end-to-end. clippy and fmt both re-confirmed clean after the fix.
- **Real, working code — first live-GUI verification of `spartan-editor-core` on Linux, a real
  X11 keyboard-focus finding, a second real cross-platform build bug found and fixed (§75.13)**:
  §75.12 assumed no GPU meant no GUI verification was possible; this pass found and installed a
  real software Vulkan device (Mesa lavapipe, `PHYSICAL_DEVICE_TYPE_CPU`, confirmed via
  `vulkaninfo`) that's enough to actually launch the real product binary end-to-end for the first
  time outside a Windows/real-GPU machine. Three more real environment gaps were found and fixed
  (`libxkbcommon-x11-0`, `mesa-vulkan-drivers`, `Xvfb` for a display) before the window would even
  open. A real X11/ICCCM finding, confirmed with temporary debug instrumentation added and then
  fully reverted before committing anything: without a window manager, winit's X11 backend never
  reports `Focused(true)` and never receives a single `KeyboardInput` event, even though mouse
  events and direct `XSetInputFocus` calls both work fine — not a product bug, fixed by running a
  minimal window manager (`fluxbox`), after which real keypresses flowed correctly end-to-end. A
  second real cross-platform build bug was found by actually running `cargo build --release
  --workspace` (not `cargo test`, which never linked the offending code path at all):
  `ui-shell-spike`'s real Win32 `SetFocus` focus-stealing fix (§47.11) had no `#[cfg(windows)]`
  guard, so the full workspace build failed to link on Linux with `undefined symbol: SetFocus`.
  Fixed by gating the `windows`/`raw_window_handle` imports and the call site behind
  `#[cfg(windows)]`, no Windows behavior change, re-confirmed clippy/fmt/full-test-suite clean
  afterward. Live, through the real product binary: real language detection, LSP/DAP
  startup, and tree-sitter highlighting all initialized correctly; a real cold-open of 116-118ms
  (this software-rendering stack only — explicitly not compared to the GPU-hardware cold-open
  numbers elsewhere in this file, different backend entirely); a screenshot confirmed real,
  correct rendering, and a second screenshot after sending real `Space`/`a`/`b` keypresses through
  the X server confirmed the literal characters `ab` genuinely inserted with the cursor positioned
  correctly after them — the first live, keyboard-driven edit of this crate's real binary outside
  Windows. Cold-open/edit/scroll latency were not benchmarked here (functional verification only);
  Backspace/Enter/arrows/scroll were not separately exercised live; whether `wry`'s Linux WebKitGTK
  backend has an analogous focus-stealing bug to the Windows one is unexplored.
- **Real, working code — real mouse input, click-to-position-cursor (§75.14)**: first increment of
  a real push through §35.4's remaining Tier 1 gaps (UI shell, multi-file editing, Leo, Android,
  GUI Builder — all still reference-only as of this bullet). Picked first because `spartan-editor-
  core` had zero mouse handling at all before this — a hard dependency for any further clickable
  UI. Real `TextState::hit_test` (cosmic-text's own `Buffer::hit`, confirmed against installed
  source), `viewport::to_doc_line`, and `EditorView::set_cursor_to_line_col` (clamped) chain
  together in `main.rs`'s new `CursorMoved`/`MouseInput` handling. 7 new headless tests for the
  clamped setter and the line-translation inverse; `hit_test` itself needs a real GPU device so
  can only be verified live — confirmed via two screenshotted clicks at different coordinates each
  landing the caret exactly where clicked, plus a follow-up Backspace deleting at the new position,
  proving the click and keyboard paths share one real cursor. No text selection yet (no selection
  concept exists in `EditorView` at all -- scoped out as its own increment), no double/triple-click,
  no context menu, no scrollbar-drag.
- **Real, working code — real multi-file editing, keyboard-driven switching (§75.15)**: second
  increment of the Tier 1 push. `main.rs`'s flat single-file locals became a real `OpenFile` struct
  + `Vec<OpenFile>` indexed by `active`; `TextState` stays one shared GPU-backed instance across all
  open files (reshaped on switch, not duplicated). Additional files open via repeated
  `--open:<path>` CLI args (no file-tree/dialog UI yet -- task #16), switched with Ctrl+Tab /
  Ctrl+Shift+Tab. Each LSP-capable open file spawns its own `rust-analyzer` process (named cost --
  `LspSession` is hardcoded to one file's lifecycle, multiplexing is separate future work). DAP
  breakpoints moved from a flat `Vec<i64>` to per-file storage, fixing a real latent bug (no file
  association once >1 file could be open). A real bug was caught only by actually closing the live
  app: draining `files` on `CloseRequested` left it empty, but winit delivers at least one more
  event afterward that still indexes `files[active]`, causing a real panic -- fixed by `take()`-ing
  each file's LSP session in place instead of draining the `Vec`. Live verification: two real files
  opened, switched between (confirmed via screenshots showing genuinely different content), typed
  into one, switched away and back (confirmed the edit was per-file, not shared), and closed cleanly
  with both LSP sessions shutting down, no panic. 50k-line benchmark re-run and still completes
  correctly. No visual file-tree/tab bar, no shared LSP session across files, no unified multi-file
  breakpoint set, no open-file dialog. Also surfaced a bigger, real, separate gap: **no file in this
  crate has ever been saved to disk** -- there is no save functionality at all yet, for any file.
- **Real, working code — real save-to-disk, Ctrl+S (§75.16)**: closes the gap §75.15 named. Real
  `std::fs::write` on Ctrl+S (matched via `physical_key`, not `logical_key`, since Ctrl-held letters
  don't reliably carry `text` on every platform), guarded against `--synthetic:` fixtures (no real
  path to write to -- prints a clear refusal instead). `OpenFile.dirty` tracks unsaved changes; since
  no UI chrome exists anywhere in this crate, the window title itself is the first real dirty
  indicator (`window.set_title` appends `*`). Live verification against a real scratch file (not a
  tracked repo file): typed an edit, confirmed the title gained `*` via `xdotool getwindowname`
  (not just a screenshot -- the real title was wider than the visible titlebar), pressed Ctrl+S,
  confirmed the `*` cleared, confirmed `"Saved: <path>"` printed, and confirmed by reading the file
  directly off disk that its actual bytes matched the edit. No prompt/confirmation on closing or
  switching away from a dirty file -- both currently discard unsaved changes silently, a real,
  named data-loss risk, not a hidden one. No save-as, no external-change detection, no auto-save.
- **Real, working code — real arrow-key cursor navigation (§75.17)**: found while scoping text
  selection -- no arrow-key handling existed anywhere in this crate before this pass; the cursor
  could only move via mouse click or as a side effect of editing. `EditorView::move_left/right/up/
  down` (clamped, `move_up`/`move_down` reuse the existing `set_cursor_to_line_col` clamp) wired to
  `main.rs`'s `ArrowLeft/Right/Up/Down` handling, following the same `ensure_visible`+reshape
  pattern every other cursor-moving key uses. No "sticky column" across multi-line up/down runs
  (a real, named, minor UX simplification). 8 new headless tests, all passed first run except one
  caught before running: an early test assumed line 0 was a single-line fixture's last line without
  accounting for `Document`'s own documented ropey phantom-trailing-line behavior, fixed by deriving
  the real last line instead of assuming. Live verification: 3xDown+5xRight from document start
  landed exactly on line 3 col 5 (screenshot-confirmed against the real, distinguishable text
  there), then Up+Left landed on line 2 col 4. Shift+Arrow does not extend a selection yet -- no
  selection concept exists (task #15 remains, now correctly blocked only on selection itself). No
  Home/End, Ctrl+Arrow word jumps, or Ctrl+Home/End yet either.
- **Real, working code — real text selection, click-drag/shift-click/shift-arrow/type-over-replace
  (§75.18)**: closes task #15. `EditorView` gained a real `selection_anchor` + `selection_range()`
  (normalized, click-with-no-drag isn't a real selection); `insert_at_cursor`/`backspace` now
  replace/delete an active selection instead of operating alongside it. `viewport::
  selection_line_spans` is new, pure, headlessly-tested logic turning a selection range into
  per-line column spans; `main.rs` turns those into real pixel rects via the same
  `cursor_pixel_pos` lookup the caret uses, rendered by a new `SelectionRenderer`
  (`selection.rs`/`.wgsl`) -- its own type, not a generalized `CursorRenderer`, since selection
  needs a variable count of semi-transparent quads rendered *before* the glyph pass, the opposite
  of the caret's single opaque quad on top. Mouse drag extends from press to release; Shift+click
  extends from the existing anchor; a new `handle_arrow_key` makes Shift+Arrow extend and a plain
  arrow with a selection active collapse it (Left/Right to the exact start/end, Up/Down clear-and-
  still-move) instead of moving further; Escape clears a selection. 13 new headless tests -- one
  real test-writing mistake caught by running it (miscounted which chars a range covered), fixed by
  recounting, not by changing the correct implementation. Live verification: a real drag produced a
  genuine multi-line highlight (screenshotted); typing over it replaced the range (screenshotted,
  also incidentally surfacing that Ctrl+Z inserts a literal "z" rather than undoing -- no undo
  keybinding exists, tracked separately); shift-click extend, plain-Left collapse, shift-arrow
  re-extend, and Escape-clear were each screenshotted in sequence. Full test/clippy/fmt suite clean;
  50k-line benchmark re-run, no regression. No clipboard (copy/cut/paste) yet -- deferred as its own
  complete feature, needs a real clipboard crate dependency. No double/triple-click. An empty
  selected line renders with zero width (invisible) -- a real, named, minor gap.
- **Real, working code — real undo/redo, Ctrl+Z/Ctrl+Y (§75.19)**: found live while testing
  selection -- Ctrl+Z was inserting a literal "z" instead of undoing anything. Investigation
  surfaced that `spartan-buffer::Document` has had a complete, tested branching undo tree since
  §75.2 (`undo()`, `jump_to_checkpoint()`) that nothing in `spartan-editor-core` had ever called.
  Since a branching tree has no single well-defined "redo," that conventional behavior is built one
  layer up: `EditorView` gained a `redo_stack` (pushed by `undo()`, popped by `redo()`, cleared by
  any real edit), treating an already-evicted checkpoint (a real possibility given the bounded ring)
  as "skip it" rather than an error. `main.rs` wires Ctrl+Z to undo, both Ctrl+Y and Ctrl+Shift+Z to
  redo, matched via `physical_key` like Ctrl+S. Always a full reshape, never the cheap per-line path
  -- undo/redo can change an unbounded amount of content at once. 7 new headless tests, all passed
  first run. Live verification: typed 10 characters, one Ctrl+Z removed exactly one (confirming this
  crate commits one checkpoint per keystroke, not per logical edit), 9 more fully restored the
  original content, 10 Ctrl+Y presses then restored the typed text exactly -- each step
  screenshot-confirmed. No undo coalescing (ten keystrokes need ten undos, tracked as a follow-up),
  no undo/redo UI, no explicit LSP re-notification path beyond the normal debounced one.
- **Real, working code — real clipboard integration, Ctrl+C/X/V, and a real live selection bug
  found and fixed (§75.20)**: closes task #22 (deferred out of §75.18). New `arboard` dependency for
  real OS clipboard access; new `Document::text_between()` in `spartan-buffer` itself (no substring
  accessor existed before -- only whole-document `text()` and per-line `line()`). Ctrl+C/X/V copy,
  cut, and paste the active selection's real text, matched via `physical_key` like Ctrl+S/Z/Y.
  **A real, live bug was found only by testing paste-after-a-plain-click** (never exercised by
  §75.18's own live testing, which always drag-selected, shift-clicked, or stayed keyboard-only):
  a plain click armed `selection_anchor` unconditionally "in case of a drag," but since
  `selection_range()` only checks whether anchor and cursor differ, *any* later cursor movement --
  typing, pasting, undo/redo, not just an actual drag -- silently became a visible selection.
  Reproduced live (screenshotted: pasted text incorrectly shown highlighted) and fixed by no longer
  arming the anchor on press at all -- a new `drag_anchor_pos` local remembers only where the button
  was pressed, and `CursorMoved` arms the real anchor lazily, only once real movement is observed.
  Re-verified live after the fix: identical paste sequence now shows no stray highlight
  (screenshotted), plus a full cut-then-paste-back round trip (screenshotted at each step). 5 new
  tests (3 for `text_between`, 2 for `selected_text`), full test/clippy/fmt suite clean before and
  after the fix, 50k-line benchmark re-run with no regression. No rich-text/image clipboard formats,
  no X11 PRIMARY-selection middle-click paste, no clipboard history.
- **Real, working code — real visual tab bar, click to switch, click to close (§75.21)**: closes
  the visual half of task #16 (file tree sidebar split to task #24 -- different scope). Reading
  `glyphon-0.5.0`'s actual `TextRenderer::prepare()` source showed it already accepts *multiple*
  `TextArea`s sharing one `FontSystem`/`TextAtlas` -- so `TextState` gained a second
  `tab_bar_buffer`, no parallel rendering pipeline needed. `TEXT_ORIGIN_Y` redefined as
  `8.0 + TAB_BAR_HEIGHT`; every existing call site already used the symbolic constant, so the whole
  editor shifted down for free. New, pure, headlessly-tested `tab_bar.rs` builds the tab row's
  display string and each tab's real char-range (plus its `×` close button's narrower range); clicks
  resolve via the *same* real cosmic-text hit-testing `hit_test` already uses, not a pixel-guessing
  geometry model. Active-tab highlight reuses `SelectionRenderer` as a second instance rather than a
  new type. `close_file()` shuts down the closed file's LSP session and keeps `active` pointing at a
  valid file, refusing to close the last remaining tab (no "empty editor" state exists to fall back
  to). 8 new headless tests, all passed first run. Live, with three real files open: tab bar
  rendered correctly (screenshotted), click-to-switch worked (highlight moved, title/content
  updated), click-×-to-close worked (LSP shutdown confirmed in the log), closing down to one tab and
  then attempting to close it printed the refusal and left it untouched -- each screenshotted. A
  real `clippy::reversed_empty_ranges` deny-level lint surfaced on a §75.20 test that hadn't been
  re-linted since; fixed by matching this same file's own pre-existing precedent for that exact
  situation. Full test/clippy/fmt suite clean, 50k-line benchmark re-run with no regression. No file
  tree sidebar yet (files still only open via `--open:` CLI args), no tab reorder, no overflow
  handling, no Ctrl+W.
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
cargo test --workspace --release   # 103 tests: 6 spikes + 3 real crates (spartan-buffer,
                                    # spartan-languages, spartan-editor-core)
# dap-spike and spartan-editor-core's own dap_integration.rs need `lldb-dap` (or `lldb-dap-18`) +
# `rustc`; lsp-spike and spartan-editor-core's own lsp_integration.rs need `rust-analyzer` +
# `rustc`; spartan-editor-core's dap_python_cross_language.rs needs python + the debugpy package.
# All self-skip with a printed message if their tool isn't found on $PATH -- and do differ by
# machine: this project's own history includes sessions where lldb-dap was installed and one
# (§75.8) where it wasn't but debugpy was, and one (§75.12, a Linux container) where lldb-dap was
# installed but debugpy had to be installed manually to get real dual-adapter coverage -- so
# don't assume either is universally present.
# Real subprocess-spawning suites now spawn language-server/debug-adapter processes under real
# timing (dap-spike, lsp-spike, and spartan-editor-core's own lsp_integration.rs/dap_integration.rs/
# dap_python_cross_language.rs) -- under `cargo test`'s default full parallelism this occasionally
# produces a resource-contention flake (a different one of these suites' tests timing out each run,
# not a real functional bug -- confirmed by re-running the exact same binary in isolation, where it
# passes). If a real-subprocess test fails only inside a full `--workspace` run, first retry with
# `cargo test --workspace --release -- --test-threads=1` before assuming it's a real regression.
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
