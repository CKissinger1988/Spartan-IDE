# spartan-editor-core

Real (non-spike) Tier 1 core-engine code — not a Tier 0 risk-gate experiment
with a go/no-go verdict, and not the IDE itself. This crate is the first
place `spartan-buffer` (§2.1, the real document/rope model), a real GPU
text-rendering pipeline, `spartan-languages` (§20.1, the real language
registry), and now real live LSP and DAP sessions are combined and driven
from one real file open. See `docs/architecture-spec.md` §75.5 (buffer +
renderer + language registry, viewport virtualization), §75.6 (real LSP
wiring), §75.7 (auto-scroll/resize), §75.8 (real DAP wiring), §75.9
(cold-open investigation), and §75.10 (real DAP build-system integration)
for the full write-ups this README summarizes.

## What this is

- `src/editor_view.rs` — owns a real `spartan_buffer::Document` plus the
  cursor position, classifying every edit as `EditEffect::None` /
  `Line(usize)` / `Structural` so the renderer knows whether a cheap
  per-line reshape suffices or a full reshape is needed. Promoted from
  `spikes/render-spike` (§47.9-§47.10) with one deliberate change: the
  cursor starts at document position 0 (top of file), not end-of-file —
  correct for opening a real file, where `render-spike`'s original choice
  was just its own demo's convenience.
- `src/viewport.rs` — the one genuinely new piece of engineering in this
  crate: a `Viewport { scroll_line, visible_lines }` struct and
  `windowed_text()`, which extracts only the currently-visible slice of the
  document. This is what makes cosmic-text's `Buffer` see ~34-60 lines
  regardless of whether the document is 3 lines or 50,000 — the literal
  reading of §2.2's "only re-rasterize the visible viewport" requirement,
  and the fix for the cold-open gap `render-spike`'s own exit report named
  as untouched by its damage-region increment.
- `src/language.rs` — `detect_language_for_file()`, wiring the real
  `spartan_languages::LanguageRegistry` in for the first time alongside
  real rendering. Also `find_project_root()`, which walks up a file's
  ancestor directories looking for the detected profile's own
  `marker_files` (e.g. `Cargo.toml`) — the project root a real LSP server
  needs. Tree-sitter is **not** wired up; LSP and DAP now are (§75.6, §75.8).
- `src/lsp.rs` — `LspClient`/`DidChangeDebouncer`, promoted verbatim from
  `spikes/lsp-spike` (already proven against real `rust-analyzer` and
  `pyright-langserver`).
- `src/lsp_session.rs` — the genuinely new engineering in §75.6: a real,
  live `LspSession` running an entire language-server session (spawn,
  initialize/didOpen, every subsequent didChange dispatch) on its own OS
  thread, since `LspClient`'s calls block for up to 90s and would freeze
  the render loop otherwise. Uses a single-slot mailbox (`Mutex`+`Condvar`,
  not a channel) so a burst of debounce firings during a long indexing
  wait can never pile up a stale backlog — only the most recent edit ever
  actually gets dispatched.
- `src/dap.rs` — `DapClient`, promoted verbatim from `spikes/dap-spike`
  (proven against real `lldb-dap` and `debugpy`), plus two small new
  methods (`step_over`, `step_into`) matching the already-promoted
  `continue_()`'s exact shape.
- `src/dap_session.rs` — the genuinely new engineering in §75.8: a real,
  live `DapSession`. Deliberately **not** a mailbox like `LspSession` —
  every debug command (continue, step-over, step-into) is discrete and
  must execute in order, none dropped, so it uses a plain ordered
  `mpsc::channel` instead.
- `src/gpu.rs`, `src/text.rs`, `src/cursor.rs`, `src/cursor.wgsl`,
  `src/latency.rs`, `src/input.rs`, `src/fixture.rs` — promoted from
  `spikes/render-spike` essentially verbatim; already proven on this
  project's real Intel UHD 620 / Vulkan / Windows-GNU setup.
- `src/main.rs` — the real binary: opens a file (or `--synthetic:<lines>`
  for the benchmark fixture), prints the detected language, starts a real
  LSP session for real (non-synthetic) files whose language has an
  `lsp_command`, opens a wgpu/winit window seeded with only the initial
  viewport's text, and wires keyboard input, PageUp/PageDown scrolling,
  live LSP diagnostics printing, `F9`/`F5`/`F10`/`F11` debug controls
  (toggle breakpoint / launch-or-continue / step-over / step-into, given a
  `--debug-binary:<path>`), and an optional three-phase scripted latency
  benchmark together.

## Real measured results (50,000-line synthetic fixture, same Intel UHD
Graphics 620 / Vulkan / IntegratedGpu hardware `render-spike` used)

| Metric | render-spike (post damage-region) | This crate (+ virtualization) |
|---|---|---|
| Cold-open | 897.7-1297.9ms | 575.5-617.5ms at first (3 runs); 467-715ms after §75.9's `FontSystem` parallelization (5 runs) |
| Edit p99, random-position across whole doc | 6.0-25.1ms | 2.5-3.1ms (see caveat below) |
| Edit p99, realistic cursor-adjacent typing | 6.0-25.1ms (no other kind measured) | 3.5-3.9ms (2 runs, n=500) |
| Scroll re-shape | not measured (never scrolled before) | p50 16.2-16.4ms, p99 19.4-29.2ms (3 runs, n=100) |

**Caveat on the random-position row, stated plainly rather than rounded
away**: with a ~34-60 line viewport against 50,000 lines, a uniformly
random edit position lands inside the visible window only ~0.07-0.1% of
the time. Across three 2000-iteration runs, 0-1 edits actually landed
in-window — so that number mostly measures "how cheap is a genuine
no-op," not real reshape cost. The dedicated **cursor-adjacent** benchmark
(sequential typing at the cursor, which never leaves the visible window
during that phase) is the honest answer to "does virtualization help
realistic typing" — and it does, landing reliably under §39.1's <5ms p99
target, which `render-spike`'s own report said was "not reliably met."

Reproduce with:

```bash
cargo build -p spartan-editor-core --release
./target/release/spartan-editor-core "--synthetic:50000" 2000 500 100
# args: fixture (or --synthetic:<lines>), random-edit iters, cursor-typing iters, scroll iters
```

## Cold-open investigation (§75.9)

A real, permanent per-step timing breakdown (printed alongside the
existing cold-open number) found `GpuState::new()` (wgpu instance/adapter/
device/surface) to be the single largest cost (~220-433ms), not text
shaping. A hypothesis-driven fix — restricting `wgpu::Instance::new()` to
`Backends::VULKAN` only, skipping the DX12/DX11/GL probing `Backends::all()`
otherwise does — was implemented and measured across 5 runs, found to make
no real difference, and reverted rather than kept as unproven complexity.
A second fix did help for real: `FontSystem::new()` (cosmic-text's system
font scan, ~93-97ms) has no actual dependency on the GPU device, so it now
runs on its own thread concurrently with `GpuState::new()`'s async setup —
`TextState::new()`'s own cost dropped to ~2-2.5ms, and cold-open dropped to
a 467-715ms range from ~570-810ms. An apparent regression surfaced during
verification of this change (cursor-adjacent p99 jumped to 4.65-4.73ms,
scroll p99 to 41ms) turned out, after a controlled A/B/A/B comparison
against the prior committed binary, to be transient system noise from
repeated rapid rebuild-and-run cycles, not a real effect — documented as
a real methodological finding, not glossed over.

## Real visual verification

Screenshot + `enigo`-based synthetic OS input (the same methodology already
established for `render-spike`/`ui-shell-spike`): opened a real file
(`crates/spartan-buffer/src/lib.rs`), confirmed the real detected language
printed to stdout, confirmed real file content on screen, typed two real
lines via OS-level synthetic keyboard input, confirmed the caret rendered
at the correct position, scrolled forward two pages and confirmed the
content changed, scrolled back and confirmed the original content —
including the injected text exactly where it was typed — matched the
pre-interaction screenshot.

## Real LSP verification (§75.6)

A self-skipping integration test (`tests/lsp_integration.rs`, skips if
`rust-analyzer` isn't on `$PATH`) spawns a real `LspSession` against a real
generated Cargo fixture with a deliberate `E0308` type error, confirms a
real non-empty diagnostic, then sends corrected text via `notify_edit` and
confirms diagnostics really update to empty — the first real exercise of
`did_change_full`, which the spike's own tests never called. A live binary
run against a real fixture (screenshot + `enigo` synthetic input) confirmed
the same end-to-end, on screen: real diagnostics printed after real
indexing, then — after typing `//` at the cursor to comment out the file's
one line (the only edit reachable given this crate's cursor always starts
at position 0 with no navigation beyond PageUp/PageDown) — real diagnostics
updated to "0 diagnostics — clean" within one debounce cycle. The 50k-line
`--synthetic:` benchmark was re-run afterward and showed no measurable
change (LSP never spawns for synthetic fixtures, which have no real
project root).

## Auto-scroll and resize (§75.7)

Two named limitations from earlier passes are now fixed: `Viewport::ensure_visible()`
scrolls minimally to keep the cursor on screen (typing enough newlines near
the bottom edge used to move the caret off-screen with no follow), and
`WindowEvent::Resized` now recomputes `visible_lines` from the new window
height and reshapes accordingly (previously fixed at startup only). Both
verified for real: 45 synthetic `Enter` keypresses confirmed via screenshot
to keep the caret visible at the bottom of the window, and a real Win32
`SetWindowPos` resize confirmed via screenshot to reflow content and keep
the caret correctly positioned. The 50k-line benchmark was re-run and shows
no regression (the benchmark's scripted edit paths deliberately don't
exercise the new auto-scroll code, by design).

## Real DAP verification (§75.8)

`lldb-dap`/`lldb-dap-18` weren't installed on the machine this pass ran on
(a real, confirmed environment difference from an earlier session), so a
second real test (`tests/dap_python_cross_language.rs`, mirroring
`dap-spike`'s own cross-language test) verifies the identical
`DapSession`/`DapClient` path against real `debugpy` instead: a real
breakpoint hit with the correct variable value, a real `StepOver` landing
on the correct next line, and a real `Continue` running to a real exit —
all genuinely executed, not asserted. The primary `tests/dap_integration.rs`
(Rust/`lldb-dap`) correctly self-skips here. Live, through the actual
product binary: `F9` correctly toggles and prints a breakpoint; `F5`
correctly attempts a launch and, since no adapter could be spawned in this
environment for either language (see below), reports a clean error rather
than hanging — confirmed via log and a follow-up screenshot showing the
editor still fully responsive. A real, unplanned finding from trying this:
`spartan-languages`'s own Python `dap_command` (`program = "debugpy"`)
isn't directly invocable — `debugpy` needs `<python> -m debugpy.adapter`,
which the test worked around with a generated wrapper script, but
`main.rs`'s registry-driven dispatch has no such step. The 50k-line
benchmark was re-run and shows no regression.

## Real DAP build-system integration (§75.10)

Closes §75.8's named gap for Cargo: `src/build.rs`'s `build_debug_binary()`
runs a real `cargo build --message-format=json` (its exact JSON shape
confirmed by running a real build, success and failure, before writing any
parsing code) and returns the real resulting binary path or real rendered
compiler diagnostics. `F5` now runs this on its own thread (never blocking
the render loop) whenever no explicit `--debug-binary:` was given but a
real Cargo project is discoverable. `DapSession::is_finished()` also fixes
a small real gap: `F5` after a session ended used to silently fail to
`Continue` a dead session; now it correctly starts fresh. Two new tests
run a real `cargo build` against generated fixtures (success reports a
real, existing executable path; a real type error reports the real
`E0308` diagnostic). Live, through the actual binary: `F9`/`F5` triggered
a real `cargo build` that succeeded, found the real executable (confirmed
to exist on disk), and attempted a real DAP launch with it — failing
gracefully with the same honest error as §75.8, since `lldb-dap` still
isn't installed here. The 50k-line benchmark was re-run and shows no
regression.

## What is explicitly not done here

- §39.1's <100ms cold-open target — still not met (467-715ms after §75.9's
  fix, ~4.7-7.2x over), though meaningfully closer than `render-spike`'s
  897-1298ms. `GpuState::new()`'s ~220-433ms wgpu instance/adapter/device
  setup is now the clear, dominant, unaddressed remaining cost — a
  Vulkan-only-backend optimization was tried for this specifically and
  found not to help (§75.9).
- Scroll cost (19-29ms p99) is a new, real, unaddressed cost against the
  same latency target edits are measured by.
- Auto-scroll snaps the cursor to the window's very edge, with no
  surrounding context margin (a real editor convention this pass didn't
  attempt). No horizontal scrolling/wrapping for long lines.
- No SDF glyph atlas, no layered compositing, no tree-sitter, no Leo, no
  UI chrome (scrollbar, tabs, panels, a diagnostics/problems panel, a
  breakpoint gutter, a variables panel). Diagnostics/stops are printed to
  stdout only.
- No hover or completion wiring — the promoted `LspClient` supports both,
  but there's no UI trigger (hover-position detection, a completion popup)
  to hang them off yet.
- A file with no discoverable marker file in any ancestor directory falls
  back to single-file mode (`find_project_root` returns `None`) — real,
  but meaningfully worse diagnostics than a real project root gives.
- `LspSession::shutdown()`'s bounded waits (~7s worst case) will visibly
  delay window close if triggered mid-request — mitigated with a printed
  status line, not eliminated.
- No crashed/hung language server detection or restart.
- Build-system integration only covers Cargo (§75.10) — a language whose
  `build_systems` names something else (`npm`, `poetry`, `gradle`, ...)
  still requires an explicit `--debug-binary:<path>`.
- No incremental-build awareness beyond what `cargo build` itself already
  does, and no build cancellation if `F5` is pressed again mid-build (the
  in-progress build simply finishes; the second press is rejected with a
  printed message, not queued or merged).
- DAP breakpoints are plain line numbers, not rope-anchored — the
  §39.2-sanctioned v1 fallback, not an oversight, but a real limitation:
  an edit that shifts lines above a breakpoint will silently point it at
  the wrong line. No live breakpoint changes once a session is running.
- `step_into` compiles and is wired to `F11` but isn't exercised by any
  test or live verification yet, unlike `continue_`/`step_over`.
- The Python registry entry's non-invocable `dap_command` (see above) is a
  real, pre-existing gap this pass found but didn't fix.
