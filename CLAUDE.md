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
- **Real, working code — Home/End/Ctrl+Arrow navigation, sticky column (§75.22)**: closes the three
  gaps §75.17 named explicitly. Six new `EditorView` methods (`move_to_line_start/end`,
  `move_to_document_start/end`, `move_word_left/right`), all following the existing
  did-anything-change `bool` convention. Word jump has no cheap single-char `Document` accessor to
  build on, so two new private helpers (`char_before`/`char_at`) each fetch one char via
  `text_between(pos..pos+1)` -- O(log n) per call but bounded to a handful of calls per jump, not a
  whole-document scan. Jump logic: skip adjacent whitespace (crossing line boundaries for free via
  `\n`), then consume the contiguous run of same-kind chars, where word chars (alphanumeric/`_`) and
  punctuation are different kinds -- so `foo.bar` stops at the `.` instead of jumping straight from
  `foo` to `bar`. Sticky column (`EditorView::sticky_column`) stores the *desired* column `move_up`/
  `move_down` were asked to reach, not whatever a short intermediate line clamped it to, so a run
  survives multiple short lines and restores the original column on the next long-enough one --
  cleared by every other cursor-moving method, since a "run" is strictly consecutive up/down calls.
  `handle_arrow_key` renamed `handle_navigation_key`, gained a `ctrl: bool` param; §75.18's
  selection-collapse rule for plain Left/Right is preserved (`if !ctrl`-gated), every other
  combination clears the selection and moves, matching how §75.18 already treated Up/Down. Two real
  test-writing mistakes caught only by running the tests: one assumed word-right lands at the start
  of the next token (it actually lands at the end, matching the other passing word-jump tests); one
  assumed moving right on a blank line is a no-op (a blank line still has a real `\n` to move over,
  so it correctly crosses to the next line) -- both fixed by correcting the test, not the code. 15
  new headless tests, full workspace test/clippy/fmt clean. Live, through the real binary: End/Home/
  Ctrl+End/Ctrl+Home all landed correctly on a real fixture (screenshotted); six Ctrl+Right presses
  landed exactly between `foo` and `.bar` in `foo.bar baz qux`, one Ctrl+Left retraced exactly back;
  a dedicated three-line fixture confirmed sticky column end-to-end -- click mid-column, arrow down
  onto a 1-char line (visibly clamped), arrow down again restored the original visual column instead
  of staying at column 1. 50k-line benchmark re-run, no regression (cold-open ~104ms, edit/cursor p99
  ~3.5-4.0ms, scroll p99 ~5.8ms, consistent with the prior baseline).
- **Real, working code — unsaved-changes confirmation modal, closing task #18 (§75.23)**: closing a
  dirty tab or exiting the app previously discarded content immediately, with no confirmation.
  Deliberately does NOT cover switching the active file (Ctrl+Tab, clicking a different tab) --
  tracing the actual code showed nothing is ever lost by switching away from a dirty file, only by
  closing a tab or the whole process, so the scope narrowed to match the real risk rather than
  building an unnecessary switch-time prompt. `SelectionRenderer`/`selection.wgsl` (§75.18, reused
  for the tab highlight in §75.21) had its color hardcoded in the shader -- genericized to a real
  per-vertex `color` attribute now that a third caller (the modal's dim overlay) wants a different
  one, rather than a third near-duplicate pipeline; the two existing callers now pass the extracted
  `selection::ACCENT_HIGHLIGHT` constant explicitly. `TextState` gained a third glyphon `TextArea`
  (`modal_buffer`), same "empty text draws nothing, no separate on/off flag" pattern the tab bar
  already established, roughly vertically centered using the real current window height. Keyboard-
  only confirm/cancel (Enter/Escape) is a real, named v1 scope decision -- no clickable buttons yet.
  A new `PendingClose` enum + a dedicated `KeyboardInput` match arm inserted *before* every other
  keyboard arm intercepts all input while a modal is up (match arms are tried in order); both mouse-
  press arms are additionally gated on `pending_close.is_none()` so clicks don't leak through either.
  A real edge case was found and fixed before it could ship: closing a dirty *sole remaining* tab
  would have raised the modal, then had Enter's `close_file` call silently refuse (its own pre-
  existing last-tab guard) -- fixed by adding the same `files.len() > 1` condition to the dirty
  check up front, so a dirty last tab's `×` stays the same harmless no-op it already was. Live
  verification hit two real environment snags along the way, both test-harness mistakes rather than
  product bugs: an `--open:<path>` arg landed in the wrong positional slot and was silently dropped
  (fixed by padding the benchmark-arg positions); and keyboard input stopped reaching one long-lived
  window instance after extended debugging (mouse clicks kept working) but was fine on a fresh
  instance, consistent with §75.13's already-documented X11/WM focus fragility, not a code change in
  this pass. With both resolved: real dirty-tab-close raised the modal (dim overlay + correct file
  name and instructions, screenshotted); Escape cancelled it (screenshotted, typing resumed
  immediately after); Enter really closed the tab; dirtying the last file and clicking the real OS
  window-close button raised the real `CloseRequested`-driven app-exit modal; Enter there really
  exited the process (confirmed via `ps`) with both LSP sessions shut down and the final latency
  report printed. A dedicated check confirmed both a click and a typed marker string were fully
  swallowed while the modal was up, leaving the underlying dirty content unchanged. No new headless
  tests needed (purely GPU/rendering/input-facing, like §75.14/§75.21's own work); full test/clippy/
  fmt clean; 50k-line benchmark re-run, no regression. No Save option in the modal itself (discard-
  or-cancel only, a deliberate v1 cut), no button hit-testing, no confirmation for any other future
  content-loss path.
- **Real, working code — Ctrl+W tab close, the keyboard half of task #25 (§75.24)**: splits task #25
  ("Ctrl+W close, overflow handling, reorder") the same way §75.21 split task #16 -- Ctrl+W reuses
  §75.23's exact two-part rule (dirty + more than one file open -> raise the modal; otherwise close
  immediately, `close_file`'s own last-tab guard already covers the rest) from a second call site,
  no new logic; overflow handling and drag-to-reorder are separate, larger, still-open scope tracked
  under the same task. `close_file(&mut files, &mut active, active)` doesn't compile -- evaluating
  `active` by value while `&mut active` is already borrowed is a real, immediate aliasing conflict,
  not a design issue -- fixed by copying into a local `closing` binding first. No new headless tests
  (thin wrapper around already-tested logic, same reasoning §75.23 itself used). Live, through the
  real binary: Ctrl+W on a clean tab closed it immediately and switched to the remaining tab
  (screenshotted); Ctrl+W on the resulting sole tab printed the pre-existing last-tab guard message
  and left it open, no modal (screenshotted); dirtying that same sole tab and pressing Ctrl+W again
  produced the identical correct no-op rather than a modal promising a close that could never happen
  -- confirming the mouse-path edge-case fix from §75.23 also holds for this new keyboard path, since
  both share the same `files.len() > 1` condition. Full test/clippy/fmt clean; 50k-line benchmark
  re-run, no regression. Tab overflow (many tabs exceeding window width) and drag-to-reorder remain
  entirely unimplemented.
- **Real, working code — undo coalescing, task #23 (§75.25)**: before this, every keystroke created
  its own real `spartan-buffer` checkpoint, so undoing a 5-character word took 5 separate Ctrl+Z
  presses. Coalescing lives at the `EditorView` layer (not `Document`'s -- its "one checkpoint per
  edit" contract is load-bearing elsewhere, already tested, out of scope to change) via a new
  `typing_run: Option<(start_cursor, checkpoints_since_start)>` field following §75.22's
  `sticky_column` precedent exactly: extended by a plain insert, reset by literally everything else
  (moves, jumps, mouse clicks, selection delete, backspace, undo, redo). `undo()` now loops
  `Document::undo()` up to the run's length in one call, stopping early (not panicking, not
  over-undoing) if the run partially aged out of `Document`'s bounded ring, falling back to the same
  clamp §75.19 already used in that case. A real, small correctness fix fell out of this for free:
  `redo_stack` grew from `Vec<CheckpointId>` to `Vec<(CheckpointId, usize)>` (storing the pre-undo
  cursor too), so both undo and redo now restore the cursor to its *exact* pre-edit/pre-undo
  position instead of only clamping it into bounds -- which was subtly wrong for any edit not at the
  very end of the document even before coalescing existed. A deliberate, named scope cut: backspace
  runs do NOT coalesce (a different, related, not-yet-scheduled gap, not folded into this task). One
  pre-existing test encoded the old, now-intentionally-changed one-undo-per-keystroke behavior --
  not a bug, a real premise this pass was built to invalidate -- rewritten with cursor moves between
  edits so it still tests what it always meant to (distinct edits, not adjacent typing). 7 new tests
  (coalescing, run-breaking, precise cursor restoration on both undo and redo, selection-replace not
  coalescing, backspace ending a run, and a 510-char run's ring-eviction fallback) all passed on the
  first run -- no bug found this time, stated plainly rather than manufactured. Full test/clippy/fmt
  clean (75 tests in this crate's own suite). Live, through the real binary: typing "hello world" and
  pressing Ctrl+Z once removed the whole string in one step; Ctrl+Y once restored it, cursor at the
  end; typing "!" after an intervening Left-then-Right arrow correctly started a new run -- one more
  Ctrl+Z removed only the "!", leaving "hello world" intact. 50k-line benchmark re-run, no regression
  (this benchmark never calls undo/redo, so only `insert_at_cursor`'s marginally larger per-call
  bookkeeping was exercised). No idle-timeout run termination, no backspace coalescing.
- **Real, working code — file tree sidebar, task #24 (§75.26)**: closes the file-tree half of task
  #16 split off back in §75.21. Before this, files could only be opened via `--open:<path>` CLI args
  at startup -- no in-app browse-and-open existed. New, pure, no-GPU `file_tree.rs` (same split as
  `tab_bar.rs`) owns `FileTree` (root + a `BTreeSet` of expanded dirs) and two pure functions:
  `visible_rows()` (real, recursive `std::fs::read_dir` through expanded dirs only, no caching, a
  named v1 cost) and `build_tree_text()` (ASCII `"> "`/`"v "`/`"  "` markers, not Unicode triangles,
  so no font-coverage dependency). Hit-testing turned out *simpler* than the tab bar's own: since the
  sidebar is genuinely multi-line (one row per line), the real cosmic-text `Buffer::hit`'s own
  `Cursor::line` *is* the row index directly -- no char-range list needed the way the tab bar's
  single-line-many-tabs layout requires. The layout shift reused §75.21's own proven trick instead of
  touching call sites by hand: `TEXT_ORIGIN_X` is now `SIDEBAR_WIDTH + 8.0`, and since the main editor
  and tab bar's rendering *and* hit-testing already routed through that one symbolic constant, both
  shifted right automatically. `sidebar_root()` reuses the exact same `language::find_project_root`
  call `open_file()` already makes for LSP root detection -- the sidebar shows the same root the
  language server is actually analyzing. Clicking an already-open file switches to it instead of
  duplicating it (`find_open_file_index`, canonicalizing both sides before comparing, falling back to
  raw comparison if that fails). 8 new headless tests, all passed first run. Full test/clippy/fmt
  clean (95 tests across this crate's lib+integration suites). Live, against a real 3-level fixture
  project: root listing rendered correctly sorted (dirs first); clicking a dir expanded it to depth 1;
  clicking a file opened a real new tab with real content; clicking that same file again in the
  sidebar switched back instead of duplicating (tab count unchanged); clicking a nested dir revealed
  its child at depth 2; the main editor's click-to-position and keyboard input both still worked
  correctly through the shifted layout. 50k-line benchmark re-run (no real path, so `file_tree` is
  `None` and the sidebar renders nothing), no regression. No caching/filesystem watching, no keyboard
  tree navigation (mouse-only), no delete/rename/create, no git-status decoration or icons, no
  sidebar toggle (always shown once a root is known, fixed 200px regardless of window width).
- **Real, working code — tab drag-to-reorder, closing task #25 (§75.27)**: closes the reorder half
  of task #25 (§75.24 already closed the Ctrl+W half; tab overflow handling is the one piece still
  open under the same task). A real, named v1 cut: teleport-on-release, not a live "ghost tab" --
  `tab_drag_start` just remembers which file a press landed on; the release position, hit-tested like
  a fresh click, decides whether/where to reorder. A drag is distinguished from a plain click by
  `to != from` at release time, not a pixel-distance threshold -- a click starts and ends on the same
  tab, so it can never accidentally reorder. `reorder_file()` is `Vec::remove`+`Vec::insert` (no
  `Clone` needed) plus index-remapping for `active`, hand-verified for both drag directions across
  all three cases (moved-file, strictly-between, unaffected) before any live testing. A real finding
  from a deliberately harder 3-tab test, not from inspection: the tab-press handler already sets
  `active = file_index` immediately on press (unchanged, pre-existing behavior, the same path a plain
  click uses) -- so by the time `reorder_file` runs on release, `active` is already forced to `from`,
  meaning the "strictly between" branch, though hand-verified correct, is currently unreachable via
  this UI. Named explicitly rather than silently deleting real, correct-but-unreached logic or hiding
  the gap. No new headless test (binary-private helper, same established live-verification-only
  pattern as `close_file`/`window_title`/`modal_message`). Full test/clippy/fmt clean. Live: a 2-tab
  drag reordered and correctly kept the dragged file active (screenshotted before/after); a plain
  click on the same tab correctly didn't reorder (screenshotted); a 3-tab test (2nd tab made active,
  1st dragged onto 3rd) produced the exact hand-verified position result `[lib.rs, nested.rs,
  main.rs]` and surfaced the live-unreachability finding above. 50k-line benchmark re-run, no
  regression. Tab overflow handling remains the one open piece of task #25.
- **Real, working code — tab bar overflow handling, closing task #25 (§75.28)**: closes the last
  open piece of task #25 (Ctrl+W and reorder shipped in §75.24/§75.27). Overflowing tabs used to
  just be clipped off-screen with no way to reach them. `TextState` gained a real pixel-space
  `tab_bar_scroll` field + `ensure_tab_visible()`, the horizontal analogue of `Viewport::
  ensure_visible`, run every frame right after the tab bar's text is rebuilt. A real bug was found
  only by testing overflow with enough tabs open, not by inspection: cosmic-text's default
  `Wrap::Word` had silently been in effect on the tab bar buffer since §75.21 -- harmless until real
  scrolling started depending on `tab_bar_pixel_pos`'s "only ever reads the first layout run"
  assumption, which a word-wrapped second run broke, silently under-scrolling and never reaching the
  real last tab (confirmed live: opening tab 12 of 12 only scrolled far enough to reveal tab 9).
  Fixed with one line, `set_wrap(&mut font_system, Wrap::None)`, right after the buffer is built --
  the tab bar is conceptually always exactly one real line no matter how wide it gets. Every existing
  tab-bar hit-testing and rendering call site needed the matching one-line scroll adjustment (the
  active-tab rect subtracts `tab_bar_scroll()`, both click-resolution sites add it back), and the tab
  bar's clip bounds were tightened to `SIDEBAR_WIDTH` on the left so a scrolled-off tab can never
  bleed into the sidebar. No new headless tests (GPU/rendering/input-facing, same category as
  §75.14/§75.21/§75.27). Live, with 12 real files open in a 1000px window: raw overflow confirmed
  (file12.rs's tab entirely invisible); the wrap bug caught mid-fix (only scrolled to tab 9); after
  the fix, opening tab 12 scrolled all the way to reveal it, fully visible and highlighted; scrolling
  back to tab 1 worked symmetrically; a direct click on a visible tab while scrolled resolved
  correctly (not just via the sidebar); closing a tab while scrolled also resolved correctly. Full
  test/clippy/fmt clean, 50k-line benchmark re-run with normal p50/p95/p99 (two isolated `max`
  outliers matched this project's own previously-documented rebuild-cycle noise, not a regression).
  No mouse-wheel tab bar scrolling (this crate has no `MouseWheel` handling anywhere yet), no visual
  overflow indicator, not stress-tested past 12 tabs.
- **Real, working code — tree-sitter syntax highlighting for TypeScript/JavaScript, Python, Java,
  and Go, closing most of task #8 (§75.29)**: extends §75.11's Rust-only wiring to four more of
  Tier 1's six languages. `tree-sitter-typescript`/`-python`/`-java`/`-go` all confirmed
  tree-sitter-0.25-compatible via a real `cargo build`; `tree-sitter-kotlin-ng` was added,
  investigated, and removed again -- a real, named crate-ecosystem gap, not attempted this pass:
  it ships no bundled highlights query at all, and the one alternate crate that does
  (`tree-sitter-kotlin` 0.3.8) pins `tree-sitter = "0.22"`, hard-incompatible with this workspace.
  Kotlin's detection/LSP/DAP wiring from earlier passes is untouched; only highlighting is missing,
  and the fallback message now says exactly why. A real, load-bearing finding: TypeScript's own
  bundled query alone has no captures for strings/comments/numbers/functions (only its *additions*
  over JS) -- reading the actual installed source showed it's designed to be layered on
  `tree-sitter-javascript`'s own comprehensive query, so `Highlighter::typescript()` now parses
  with `LANGUAGE_TYPESCRIPT` and queries with both concatenated, verified by a real test checking
  a base-JS string *and* a TS-only `interface` keyword both highlight correctly. A second real
  finding, caught only by running the new tests: Python and Go capture numeric literals as plain
  `@number`, never Rust's `@constant.builtin` -- two tests failed on the first run, traced by
  grepping the real installed query files, fixed by adding a new `"number"` `HIGHLIGHT_NAMES` entry
  rather than loosening the tests. 4 new headless tests (8 total in `highlight.rs`), full
  test/clippy/fmt clean across the whole workspace. Live, through the real binary, with real
  `.ts`/`.py`/`.java`/`.go` fixtures open as four real tabs in a real X session: each language's
  keywords/types/strings/numbers/comments rendered in visibly correct, distinct colors, screenshot-
  confirmed for all four. A real environment false alarm was caught and resolved during this
  verification (not shipped as a bug): a too-small first Xvfb screen left no room for fluxbox's
  title bar, and the resulting window-shrink made the tab bar appear missing from the screenshot
  capture -- a larger Xvfb screen reproduced a full, correctly captured window with the tab bar
  fully visible, confirming this was a test-harness capture artifact, not a rendering regression.
  50k-line benchmark re-run, no regression (synthetic fixtures attach no language profile, so no
  highlighting code runs on that path at all). No JSX-aware parsing for `.tsx`/`.jsx` (uses the
  plain TypeScript grammar), no injections for any of the five wired languages, same windowed-only
  and no-incremental-reparse limitations §75.11 already named for Rust apply identically here.
- **Real, working code — local Source Control panel, closing §56.1, task #7 (§75.30)**: the first
  real git integration anywhere in this workspace. New `crates/spartan-git` (real `git2`, vendored
  `libgit2`, no system git binary or network access needed) provides real repo discovery, status
  (independent staged/unstaged halves per file, matching git's own real semantics), stage/unstage,
  and commit -- 10 new headless tests against a real temp git repository, all passing first run.
  New `git_panel.rs` (pure sidebar-row layout, mirroring `file_tree.rs`/`tab_bar.rs`'s own split)
  builds "Staged Changes"/"Changes" sections with real status glyphs -- 5 more tests. UI wiring in
  `main.rs`: Ctrl+G toggles the left sidebar between the file tree and this panel (sharing one
  region, not a second pane -- a real multi-pane left rail is task #3's three-column shell, not yet
  built); clicking a file row stages or unstages it; Ctrl+Shift+C opens a real commit-message modal
  (refused up front if nothing is staged or no repo was found) that reuses the *existing*
  unsaved-changes modal's rendering infrastructure rather than adding a fourth glyphon `TextArea`,
  extended to accept real typed text and Backspace. A real compiler warning
  (`unused_assignments`, "did you mean to capture by reference instead?") was caught and fixed, not
  suppressed: an initial version captured git status into an outer closure-persistent variable the
  same way `sidebar_rows`/`git_rows` are, but unlike `git_rows` it was never actually read again
  across separate closure invocations -- fixed by making it a local `let` inside the match arm
  instead, both correct and simpler. Full test/clippy/fmt clean across the whole workspace. Live,
  through the real binary, against a real git repository fixture (real `git init`, a real initial
  commit, then real modified/staged/untracked files via actual `git` CLI commands): the panel
  showed the exact real `git status` split (screenshotted); clicking staged/unstaged rows moved
  files between sections in both directions (screenshotted); the commit modal opened, accepted real
  typed text, and Enter produced a real commit -- confirmed not just by the panel clearing but by
  reading the fixture repo's own `git log`/`git show` off disk afterward, showing the exact real
  commit hash, author, message, and diff; a second commit attempt with nothing staged correctly
  printed a refusal instead of opening an empty modal. 50k-line benchmark re-run, no regression (a
  `--synthetic:` fixture has no real path, so no `GitRepo` is ever discovered on that path). No diff
  view (no Diff Card component exists yet -- that's Agent-view artifact-card territory, task #3/#5),
  no per-hunk staging, no branch switcher, no stash, no merge-conflict resolution UI, no GitHub
  layer at all (§56.2-56.4, a separate larger increment), no multi-line commit body. A real, named,
  minor gap: `CloseRequested` doesn't guard against an open commit modal, so closing the window
  while it's up (with no dirty file) silently discards an in-progress, uncommitted message.
- **Real, working code — secrets detection, the redaction half of §9, task #6 (§75.31)**: the first
  real security-baseline code in this workspace. New `crates/spartan-security` implements `scan()`
  (regex-based detection of 7 real credential shapes: AWS access key IDs, GitHub/Slack/Stripe/Google
  tokens, PEM private-key blocks, and a lower-confidence generic `api_key`/`secret`/`token`/
  `password`-assignment catch-all) and `redact()` (replaces each finding's span with
  `[REDACTED:<KIND>]`), with most-specific-first overlap dedup so a token embedded in a generic
  assignment is reported once, as its specific kind. 14 new headless tests against synthetic,
  clearly-fake credential fixtures, all passing first run. Wired into `spartan-editor-core`'s
  `open_file()`: every opened file is scanned in full (deliberately whole-file, unlike this crate's
  windowed-only tree-sitter passes) and prints a `WARNING: ... appears to contain N possible
  secret(s)` line if anything is found -- a real, agent-independent, immediately useful call site,
  since no Leo/`ModelProvider` tool-execution layer exists yet to hook `redact()` into before a
  cloud call (deliberately deferred, not built prematurely with no real caller -- same reasoning
  applied to skip tool-execution approval gating and path-jailing, §36.4.6, in this same pass: both
  need a real agent tool layer to gate, which doesn't exist in this workspace). Full test/clippy/fmt
  clean across the whole workspace. Live, through the real binary: a fixture with a synthetic
  `AKIA...`-shaped string printed the exact expected warning; a clean fixture printed none (no false
  positive). The 50k-line benchmark was re-run and shows a real, small, honestly-reported cost: the
  cold-open "arg parsing/fixture load" bucket moved from this session's own ~12ms baseline to a
  consistent ~18ms across two repeated runs (~6ms from scanning the full 3.5MB synthetic fixture
  with all 7 patterns) -- a real regression, not noise, reported plainly rather than rounded away;
  overall cold-open and edit/scroll latencies are otherwise unaffected. No tool-execution approval
  gating or path-jailing (both need a real tool layer that doesn't exist), `redact()` has no caller
  yet, no debouncing/incremental re-scan (runs once per file open), and the generic-assignment
  pattern is untuned against any real-world corpus, only this crate's own synthetic fixtures.
- **Real, working code — local-first crash reporter, closing §18, task #13 (§75.32)**: the first
  real crash-reporting code in this workspace. New `crates/spartan-crash`: `CrashReport::
  from_panic_info` builds a report from a real `std::panic::PanicHookInfo`; `format_report` runs
  both the panic message and its `file:line:column` location through `spartan_security::redact`
  (§75.31) before serializing to JSON, same "redact before anything leaves local disk" discipline
  §9 calls for; `write_report` writes `~/.spartan/crashes/crash-<timestamp>.json`; `install_hook`
  chains onto the real existing default panic hook (captured via `take_hook()`) rather than
  replacing it, so normal terminal panic output is unchanged. Deliberately local-only this pass --
  no upload path exists at all yet, so "never auto-uploads" is true by construction, not a
  disable-able gate. 6 new headless tests (JSON shape, redaction of a secret in the message, in the
  location, real file creation/naming, clean-message non-redaction), all passing first run. Wired
  into `spartan-editor-core`'s `main()` as the literal first statement, before anything else can
  panic. Full test/clippy/fmt clean across the whole workspace. Live, through the real binary:
  triggering a real, pre-existing panic path (`--synthetic:not-a-number`) with `$HOME` pointed at a
  scratch directory produced a real crash report file on disk with the exact real panic message and
  source location, confirmed by reading the file directly; normal terminal panic output (thread
  name, message, backtrace) still printed unchanged, confirming the default hook is genuinely
  chained, not replaced. 50k-line benchmark re-run, no additional regression beyond §75.31's own
  already-reported scan cost. No upload path of any kind yet (an explicit user-initiated upload is
  real future work), no UI for browsing/deleting past reports (unbounded local accumulation, a real
  named gap), no OS-level segfault/native-crash capture (Rust panics only).
- **Real, working code — WASM Component Model plugin host + two real reference plugins, closing
  most of §5, task #10 (§75.33)**: the first real WASM plugin infrastructure in this workspace. New
  `crates/spartan-plugin-host` (real `wasmtime::component`, a real WIT world at `wit/plugin.wit`)
  loads a real compiled `.wasm` component and calls its real exports. `wasm-tools`/`cargo-component`
  were installed from crates.io this session (no prebuilt binary reachable through this session's
  egress policy). A real, load-bearing finding refined the original plan mid-implementation: every
  real compiled plugin unconditionally imports WASI CLI/filesystem interfaces (confirmed via
  `wasm-tools component wit`, not assumed) purely because Rust's `wasm32-wasip1` std runtime links
  against them regardless of what a plugin calls -- so §5.2's "undeclared capability, no import"
  holds strictly only for this crate's own custom `host.log` interface (gated at the real
  `Linker`-binding level, confirmed by a real test that an undeclared capability makes real
  instantiation fail); the built-in WASI capabilities are gated one layer down via
  `wasmtime-wasi`'s own deny-by-default `WasiCtx` instead (confirmed via its installed source), so
  the mandatory imports resolve but every filesystem/network op stays inert. Two real reference
  plugins, `crates/plugins/linter-bridge` (real TODO/FIXME + line-length linting, uses the `log`
  capability) and `crates/plugins/theme-pack` (contributes a real JSON theme, declares zero
  capabilities, and its compiled binary provably imports no `spartan:plugin/host` at all) -- each
  its own real compiled `.wasm` component, deliberately in a *separate* cargo workspace
  (`crates/plugins/Cargo.toml`) after `cargo component new` was caught auto-registering the first
  one into the main workspace (which would have broken `cargo build --workspace` for the native
  target) and fixed before it shipped. A real test-writing mistake was caught by running the test,
  not by inspection (a fixture line didn't match the plugin's own rule); fixed by making the
  plugin's linting rule more realistic (substring match, not exact-line match), not by weakening
  the test. 9 new tests (3 manifest unit tests + 6 real subprocess-building end-to-end tests that
  each run a real `cargo component build`, self-skipping if the tool isn't on `PATH`), all passing.
  Full `cargo build --release --workspace` (226 tests total workspace-wide, 0 failures),
  clippy/fmt clean natively and in the separate plugins workspace. No third reference plugin
  (`agent_api`/"custom Leo tool" needs a real Leo tool-execution loop that doesn't exist yet --
  deferred for the same reason as task #6's approval gating), no UI wiring into
  `spartan-editor-core` (no plugin discovery/enable panel, no real editor-loop call site for
  `get_diagnostics`), no per-plugin resource budget (§36.4.9), no marketplace/signing (§5.4), no
  real manifest-to-`WasiCtx` capability grant mapping for filesystem/network.
- **Real, working code — accessibility tree via AccessKit + AT-SPI, closing most of §16.3, task #9
  (§75.34)**: the first real accessibility code in this workspace. A real dependency investigation
  came first: `accesskit_winit`'s latest releases need `winit ^0.30`, but this workspace is pinned
  to `winit = "0.29"`; checking the real crates.io sparse index (the web API is blocked by policy)
  found `accesskit_winit` 0.16.0-0.19.0 all genuinely depend on `winit ^0.29` -- confirmed by
  re-reading `Cargo.lock` after adding `accesskit_winit = "0.19"` + `accesskit = "0.13"` and seeing
  a single, unsplit `winit` entry, avoiding a much bigger winit 0.29→0.30 upgrade the feature
  didn't actually need. New `accessibility.rs` (pure, headlessly-tested, mirroring `tab_bar.rs`'s
  own split) builds a real `accesskit::TreeUpdate` every frame: a `Role::Window` root, a
  `Role::TabList` of real per-file `Role::Tab` nodes (selection state, dirty-marked names), and a
  `Role::Document` node holding the real, full (not windowed) active-file text. 5 new headless
  tests, all passing first run. Wired into `main.rs`'s winit event loop following
  `accesskit_winit`'s own reference example: hidden window → adapter attached → window shown;
  `process_event` called on every real `WindowEvent`; tree rebuilt every `RedrawRequested`. A real,
  load-bearing finding worked out from the actual installed `accesskit_unix` source: AT-SPI
  registration is lazy, gated behind the OS `org.a11y.Status.IsEnabled` property that's normally
  only true while a real screen reader is running -- a first live-verification attempt found
  nothing registered on the real AT-SPI bus, which read like a bug until the real source showed why
  and manually flipping that property (via `busctl`, simulating what a screen reader's own startup
  does) produced full, correct registration and a live, walkable tree. A second real, upstream,
  version-independent finding: neither the pinned `accesskit_atspi_common` 0.3.0 nor the latest
  available 0.19.0 implements the AT-SPI Text interface at all, so the real document text this pass
  already populates is currently inert from a screen reader's perspective -- present in the tree,
  not reachable through any interface a real assistive technology would query, at any version this
  workspace could adopt without the winit 0.30 upgrade named above (which wouldn't have fixed this
  particular gap anyway). Full test/clippy/fmt clean across the whole workspace. Live, through the
  real binary, in a real `dbus-run-session` + `at-spi-bus-launcher` + `python3-pyatspi` client
  (installed this pass; a real, unrelated Python ABI mismatch between the default `python3` (3.11)
  and its `python3-gi` install (built for 3.12) was found and worked around by using `python3.12`
  directly): a real single-file launch showed the exact expected tree structure with real file
  paths and window title; a real two-file launch showed both real tabs correctly listed. 50k-line
  benchmark re-run twice, no regression (one isolated scroll `max` outlier in the first run didn't
  reproduce in the second, consistent with this project's own already-documented rebuild-cycle
  noise). No real screen-reader read-aloud of content (the Text-interface gap above), no
  `ActionRequest` handling (matched and explicitly left a no-op, not silently dropped), no file-tree
  or Source-Control-panel accessibility nodes, no high-contrast/reduce-motion settings (§16.3's
  other two bullets, out of this pass's scope), no Windows/macOS backend testing (Linux/AT-SPI only,
  the only platform available in this environment).
- **Real, working code — Linux packaging pipeline, task #14 (§75.35)**: the first real
  build-automation code in this workspace, via the standard Rust "xtask" convention (`cargo run -p
  xtask -- package`). Deliberately Linux-only -- the one platform this environment can both build
  and actually *run* the resulting package on to verify it, unlike Windows/macOS packaging this
  environment has no way to build or test. `xtask/src/package.rs` separates pure, headlessly-tested
  content generation (a real freedesktop.org `.desktop` entry, a real XDG-conventions `install.sh`,
  a real end-user README -- 4 new tests, all passing first run) from real I/O (a genuine `cargo
  build --release`, a genuine `tar` subprocess call). No `LICENSE` file exists in this repo yet, so
  none is bundled or invented -- a real, named, out-of-scope gap, not fabricated. Real,
  end-to-end-executed verification, not just "it compiles": ran the real pipeline, inspected the
  real resulting `.tar.gz`'s contents, extracted it to a real scratch directory, ran `install.sh`
  with `$HOME` pointed at a separate scratch home, confirmed the real installed binary and desktop
  entry (with its `Exec=` line correctly rewritten to the real absolute path) -- and then actually
  **ran the installed binary from its new location** under the same Xvfb pipeline this session has
  used throughout, confirmed via a real screenshot showing correct rendering and syntax
  highlighting, proving the packaged artifact is a genuine working copy, not just a same-named
  file. `/dist/` added to `.gitignore`. Full test/clippy/fmt clean across the whole workspace. No
  Windows/macOS packaging, no code signing, no CI/release-automation wiring, no `LICENSE` file, no
  version auto-detection from `Cargo.toml` (hardcoded `"0.1.0"`, matching every crate's current
  version but not read from one source of truth yet) -- all real, named gaps.
- **Real, working code — Agent/Editor/Design mode toggle, the first piece of task #3 (§75.36)**:
  the first real piece of the three-mode UI shell (§8, §16.1). New, pure `mode_toggle.rs`
  (`AppMode` enum, real toggle-text layout, real click hit-testing, mirroring `tab_bar.rs`'s own
  split) plus a 5th glyphon `TextArea` (`mode_toggle_buffer`) rendered top-right of the tab bar,
  proactively `Wrap::None`'d from the start to avoid re-discovering §75.28's own hit-testing bug.
  Ctrl+1/2/3 and clicking a label switch modes; a dedicated keyboard arm swallows all other input
  while a non-Editor mode is active (a named blanket v1 choice over per-arm gating); `Agent` and
  `Design` show a real, specific placeholder message explaining exactly what's missing (no
  `ModelProvider`/Leo, no GUI Builder/WebView bridge) rather than simulated content, reusing the
  existing modal/dim-overlay infrastructure instead of a fourth near-duplicate overlay type. 5 new
  headless tests (39 total in this crate's `--lib` suite), full workspace test/clippy/fmt clean.
  Live, through the real binary: toggle strip rendered correctly with the active label accented
  (screenshotted); clicking and Ctrl+1/2/3 both switch modes correctly through all three states
  (screenshotted at each step); typing while in Agent/Design mode is fully swallowed, confirmed by
  returning to Editor mode and finding the document exactly unchanged (screenshotted); ordinary
  Editor-mode click-to-position and typing confirmed unaffected (screenshotted). A real
  methodological check: elevated cursor-adjacent p99 (12.9-17.0ms) on the 50k-line benchmark was
  investigated via a real A/B against a `git worktree` build of the immediately prior commit,
  which showed the same noisy spread (p99 10.4-20.8ms) with an identical stable p50 -- confirming
  session-level noise, not a real regression from this change, per §75.9's/§75.29's own established
  methodology. No command palette (§16.1's other half of task #3, closed by §75.37 below), no
  per-arm input isolation (blanket swallow only), no mode-switch transition animation, no
  persisted last-active mode across restarts.
- **Real, working code — real command palette, closing task #3 (§75.37)**: closes the gap §75.36
  named. New, pure `command_palette.rs` (`CommandId` enum of 8 real actions -- Save/Undo/Redo/
  Close Tab/Toggle Sidebar/the three mode switches, every one an already-real keybinding elsewhere
  in this crate), a real depth-bounded recursive file listing under the project root (skips
  hidden/`target`/`node_modules`), and a real case-insensitive subsequence fuzzy matcher/ranker.
  Deliberately not §16.1's full vision -- no natural-language Leo routing (no `ModelProvider`
  exists), no radial quick-actions, no minimap fusion. Ctrl+P opens it (reachable from any mode,
  not just Editor); keyboard-only selection (Up/Down/Enter/Escape/typed query) reuses the existing
  modal/dim-overlay rendering, no new hit-testing needed. Selecting any entry forces `mode =
  Editor` first so its effect is actually visible, then the three mode-switch commands set their
  own choice afterward. Five existing mouse-click arms (file tree, source control, tab bar, mode
  toggle, main editor) gained a `command_palette_state.is_none()` guard; while doing this, a
  pre-existing gap was found and fixed as an incidental correctness improvement -- the tab bar's
  own click arm was missing a `commit_message.is_none()` guard entirely, predating this pass. 9 new
  headless tests (48 total in this crate's `--lib` suite), full workspace test/clippy/fmt clean.
  Live, through the real binary against a real 3-file scratch project: Ctrl+P opened the palette
  showing all 8 commands plus 3 real files (screenshotted); fuzzy query `"utl"` correctly filtered
  to `utils.rs` plus one real, hand-verified non-obvious subsequence match (screenshotted); Enter
  opened the selected file as a real new tab (screenshotted); Up/Down navigation, Escape-cancel
  with the document provably unchanged, and executing "Undo" via fuzzy search (correctly undoing a
  whole coalesced typing run in one step) were each screenshotted; executing "Switch to Agent
  Mode" via the palette worked, and Ctrl+P was confirmed to still open the palette from within
  Agent mode itself (screenshotted), returning to Editor via a second palette selection; a direct
  click on the tab bar while the palette was open was confirmed to do nothing (screenshotted),
  proving the new mouse guards actually work. The 50k-line benchmark (never exercises this code
  path) was re-run twice and showed the same noisy-tail pattern already established as this
  session's baseline, no attributable regression. No mouse-click selection within the palette's
  own list (keyboard-only), no `.gitignore`-aware filtering, no live re-scan while open (list is
  captured once at open time, a deliberate choice), no frecency ranking. Task #3 is now fully
  closed.
- **Real, working code — GUI Builder two-way AST sync engine, first increment of task #12
  (§75.38)**: the first real Node/TypeScript project in this workspace's history (every prior
  increment has been Rust) -- new `gui-builder/`, its own real npm package, deliberately outside
  the Cargo workspace (§6.2 itself names Babel/SWC, not Rust, as the intended AST layer).
  `parseComponent()` parses real JSX/TSX into a `ComponentNode` tree (tags, per-prop summaries,
  text, nested children) via `@babel/parser`; `applyCanvasEdit()` takes a structured `CanvasEdit`
  (`StyleChange`/`PropChange`, matching §6.2's own Rust enum sketch) and mutates the real AST node
  directly, regenerating source via `recast.print`, which preserves the original formatting of
  every untouched node -- the real mechanism behind §6.2's "preserves formatting" requirement, not
  string templating. Both directions share one canonical traversal so their sequential node ids
  can't drift apart by construction. Real, honest v1 scope: only `StyleChange`/`PropChange` are
  implemented (`Reparent`/`ComponentInsert` need a node-identity scheme that survives structural
  edits, not attempted); `PropChange` always sets a string literal. A real finding while building
  the style summarizer: real design-token usage (`color: C.text`) mixes literals with variable
  references within one style object, so summarization is per-key (`literal`/`expression`), not
  all-or-nothing. A second, more significant real finding, investigated and isolated rather than
  hidden: editing one attribute can force `recast` to fully reprint the enclosing `JSXElement`,
  which normalizes `JSXText` whitespace in its children the same way React's JSX runtime does at
  render time -- a leading newline after a `{expression}` sibling can collapse (rendered output
  unchanged, source formatting not always byte-identical in this shape). Confirmed as a real
  upstream `recast`/Babel-JSX-generator behavior via a minimal repro (reproduces even mutating an
  existing node's value in place), documented in `gui-builder/README.md`, and locked in with a
  dedicated test that will fail loudly if a future `recast` upgrade changes this. 21 tests (Node's
  built-in `node:test`, no added test-framework dependency) all pass, including two run directly
  against this repo's own real `prototypes/*.jsx` files (5,480 real combined lines) -- the first
  real functional exercise of those files' actual syntax. `npm run build` (tsc) compiles clean; the
  Rust workspace was rebuilt afterward and confirmed unaffected. No WebView canvas (§6.1, needs
  `ui-shell-spike`'s wgpu+WebView shell promoted into `spartan-editor-core` first), no dev-server/
  HMR, no Figma import, no screenshot-to-component, no design-token file I/O. Task #12 remains
  `in_progress` -- a real first increment, not the full MVP.
- **Real, working code — real embedded WebView bridge for Design mode, second increment of task
  #12 (§75.39)**: promotes `spikes/ui-shell-spike`'s already-proven wgpu+WebView shell (§47.11)
  into the real product for the first time. Design mode now shows a real, live, bidirectional
  `wry` WebView (lazily created on first use, hidden via a zero-size `Rect` when leaving Design
  mode rather than destroyed) instead of static placeholder text -- real active-file path, a real
  component-file check, and a real IPC self-check ("connected, real round-trip confirmed"), still
  honestly not a live React/JSX canvas (needs §75.38's dev-server bridge, not yet wired to this
  WebView at all). Three real, environment-specific bugs found only by running this live, not by
  inspection: (1) `wry` needs `gtk::init()` called and `gtk::main_iteration_do` pumped every frame
  on Linux/BSD (winit doesn't drive GTK's own loop) -- fixed per wry's own documented recipe,
  gated to the exact platform set wry's own `Cargo.toml` uses, plus a promoted `build.rs` for the
  matching Windows `WebView2Loader.dll` deployment gap; (2) this specific container hung
  indefinitely in `gtk::init()` itself, isolated (via a minimal Python GTK3 repro, independent of
  any Rust code) to GTK's GSettings/dconf backend blocking on a D-Bus session this minimal
  container doesn't run -- `GSETTINGS_BACKEND=memory` fixes it, documented as a verification-
  environment-only recipe note, not baked into the shipped binary (a real desktop always has a
  working D-Bus session); (3) a real click landing *inside* the WebView's own content silently
  stops native keyboard shortcuts from reaching winit afterward -- `ui-shell-spike`'s own README
  had already found and fixed this on Windows/WebView2 but named the Linux/WebKitGTK case
  "unexplored"; no longer unexplored, fixed with `window.focus_window()` (winit's cross-platform
  method) on every native mouse press, alongside the existing Windows-specific raw `SetFocus` call
  each platform actually needs. A real, named residual: a click *inside* the WebView still needs a
  follow-up *native* click (any sidebar/tab bar/mode-toggle click, including "Editor" itself) to
  restore keyboard shortcuts -- confirmed live to reliably work as the escape hatch. Full
  workspace build/clippy/fmt/test clean; no new headless tests (GPU/WebView-facing, matching
  `mode_toggle.rs`'s own precedent), but its existing placeholder test was updated since Design
  mode's `placeholder_message()` now correctly returns `None`. Live, through the real binary:
  Design mode's real WebView content screenshotted; Ctrl+2 round-trips (both immediate and after a
  longer settle) confirmed reliable; the WebView-click focus-steal and its "Editor" label
  escape-hatch fix were both confirmed live; ordinary Editor-mode editing confirmed unaffected. The
  50k-line benchmark (never enters Design mode) was re-run and shows no attributable regression.
  Still no real React/JSX rendering, no `gui-builder` AST engine wired to this WebView at all, no
  macOS testing (none available in this project's history), CPU cost while Design mode is open not
  precisely benchmarked. Task #12 remains `in_progress` -- connecting `gui-builder`'s real AST
  engine to this real WebView shell via a real dev-server bridge is the natural next increment.
- **Real, working code — real Windows cross-compilation and execution verification, a real
  cross-platform link bug found and fixed (§75.40)**: this project has always named Windows and
  Linux as both required desktop targets, but no prior session had ever actually cross-checked the
  *other* platform. This one does: installs a real `x86_64-pc-windows-gnu` Rust target + real
  `mingw-w64`, cross-compiles the *entire* workspace for Windows, and installs real Wine to
  actually *run* the resulting `.exe` test binaries, not just link them. A real bug was found and
  fixed, not just discovered: `libgit2-sys` 0.17.0's own Windows build script never linked
  `advapi32` (confirmed by reading its actual source), causing real `undefined reference to
  __imp_GetLengthSid`/`__imp_RegOpenKeyExW`/`__imp_CryptAcquireContextA` link failures for
  `spartan-git` (and transitively `spartan-editor-core`) on the GNU toolchain -- invisible from
  this project's entire prior history since no Windows build had ever been attempted post-§75.30.
  Fixed by bumping `git2` `"0.19"` -> `"0.20"` (resolves to `libgit2-sys` 0.18.5, confirmed by
  reading its build script to add exactly the missing `advapi32` link line) -- re-verified clean on
  *both* platforms afterward (full native Linux build/clippy/test, and the Windows cross-target).
  Real, executed compilation verification: `cargo check`/`clippy --all-targets`/`test --no-run` for
  the Windows target, both for `spartan-editor-core` alone and the full workspace (every crate,
  every spike, `xtask`, including `spartan-plugin-host`'s real `wasmtime`), all clean. A second real
  confirmation §75.39's `build.rs` genuinely works: the Windows cross-build's own output showed the
  real "copied WebView2Loader.dll" success message for the first time ever (every prior Linux
  session only exercised the harmless no-op branch). Real, executed *execution* verification via
  Wine -- a real step beyond linking, and the first real Windows-binary execution performed inside
  the same session that built the binaries (`ui-shell-spike`'s own Windows verification, §47.11,
  was real but from an earlier, separate session on a real machine this project no longer has):
  every pure-logic crate's real test suite ran and passed unmodified under Wine --
  `spartan-buffer` (22/22), `spartan-security` (14/14), `spartan-crash` (6/6, including a real
  Windows-filesystem-emulated write+read-back), `spartan-languages` (17/17), `xtask` (4/4),
  `spartan-plugin-host` manifest tests (3/3), and `spartan-editor-core`'s own `--lib` (48/48) and
  `viewport_and_language.rs` (75/75) suites. Most pointedly, `spartan-git`'s own suite (10/10) --
  real repo discovery/stage/unstage/commit -- passed under Wine, the real, positive, run-not-assumed
  proof the `advapi32` fix is correct, not merely link-clean, since these operations call exactly
  the previously-unlinked APIs. `lsp_integration.rs`/`dap_integration.rs` correctly self-skipped
  (no Windows `rust-analyzer.exe`/`lldb-dap.exe` in this bare Wine sandbox) -- a real, correct
  outcome, not a false pass. `build_integration.rs` genuinely failed (2/2) for a real, understood,
  *environment* reason: it deliberately doesn't self-skip (assumes a real dev machine has
  `cargo`/`rustc`), and this Wine sandbox has no Windows Rust toolchain installed inside it -- named
  honestly, not papered over. A real, honest environment hiccup along the way, distinguished from
  the actual bug: a `--workspace --no-run` build hit "No space left on device" partway through
  (`target/debug` had grown to 16GB across this session's own many rebuilds) -- fixed by clearing
  it (safe, regenerable, nothing tracked), not by weakening the check. No GPU/GUI/WebView execution
  under Wine was attempted (needs `wine32`/a real X setup this sandbox lacks) -- the real product
  binary's own GPU-facing paths remain unverified from this session specifically, resting on
  cross-compilation success plus `ui-shell-spike`'s own separate real-Windows verification. No MSVC
  target checked (GNU only, what a Linux mingw-w64 cross-setup can produce) -- a real dev machine's
  default `rustup` target is usually MSVC; the `advapi32` fix is almost certainly relevant there too
  but wasn't independently confirmed against MSVC's own default library search. No macOS
  verification exists anywhere in this project's history.
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
cargo test --workspace --release   # 249 tests: 6 spikes + 7 real crates + xtask (spartan-buffer,
                                    # spartan-languages, spartan-git, spartan-security,
                                    # spartan-crash, spartan-plugin-host, spartan-editor-core, xtask)
# `cargo run -p xtask -- package` (§75.35) builds a real Linux .tar.gz release package into
# dist/ (gitignored) -- see §75.35 for the real install.sh verification recipe.
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
# spartan-plugin-host's own integration tests (linter_bridge_integration.rs,
# theme_pack_integration.rs) need `cargo-component` on $PATH (real subprocess `cargo component
# build` calls against crates/plugins/*) -- self-skip with a printed message if it isn't installed.
# crates/plugins/* (the real reference WASM plugins) are their own separate cargo workspace
# (crates/plugins/Cargo.toml), excluded from the main workspace on purpose -- `cargo build
# --workspace`/`cargo test --workspace` from the repo root never touch them; build them with
# `cargo component build` from inside crates/plugins/<name> instead.
# gui-builder/ (task #12, §75.38) is a real, separate npm/TypeScript project, not part of the
# Cargo workspace at all -- `cd gui-builder && npm install && npm test` (21 tests, Node's built-in
# `node:test` runner).
# spartan-editor-core's Design mode now embeds a real wry WebView (§6.1, §75.39) -- on Linux, live
# `cargo run`/manual testing in a minimal/headless environment (no real desktop D-Bus session, e.g.
# this project's own Xvfb+fluxbox verification setup) needs `GSETTINGS_BACKEND=memory` set or
# `gtk::init()` hangs indefinitely trying to reach a dconf service that isn't there -- a real
# environment-only requirement, not something the binary sets itself (a real desktop always has a
# working D-Bus session). See §75.39 for the full diagnosis.
# Real Windows cross-compilation/execution verification (§75.40): `rustup target add
# x86_64-pc-windows-gnu` + `apt-get install mingw-w64` gets a real Windows GNU toolchain on Linux;
# `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc cargo check/clippy/test --no-run
# --target x86_64-pc-windows-gnu --workspace` cross-compiles everything. `apt-get install wine64`
# (Wine 9.0) then actually *runs* the resulting `.exe` test binaries -- e.g. `wine
# target/x86_64-pc-windows-gnu/debug/deps/spartan_buffer-<hash>.exe`. GPU/GUI/WebView binaries were
# never run this way (no `wine32`/X setup for that); only headless test suites were. See §75.40 for
# the real `advapi32`/`libgit2-sys` link bug this process found and fixed.
# spartan-editor-core's real accessibility tree (§16.3, §75.34) only has headless tests for its
# own pure tree-building logic (accessibility.rs) -- live AT-SPI registration needs a real Linux
# desktop accessibility stack (at-spi2-core's `at-spi-bus-launcher`, a real D-Bus session, and
# `org.a11y.Status.IsEnabled` set true, normally done by a running screen reader) to actually
# verify, not `cargo test`. See §75.34 for the exact `dbus-run-session` + `busctl` + `pyatspi`
# recipe used to confirm it live.
cargo build --release --workspace
```

No other Rust build system exists. `gui-builder/` (task #12, §75.38) is a real, separate npm/
TypeScript project — see its own README.md and `cd gui-builder && npm install && npm test`. It
parses/edits `prototypes/*.jsx` as real AST data (proven against both real prototype files); it
does **not** build or render them as a running app — there is still no dev server, no bundler
config, no way to actually view either `.jsx` file in a browser. Don't add one without discussing
it first; that's a separate, larger piece of §6.1's own "Canvas Engine" work, not yet started.

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
