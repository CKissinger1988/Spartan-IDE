# render-spike — Spike 0.1 GPU-Half, First Increment

Real, runnable code — not pseudocode. Companion to `spikes/rope-spike` (the
CPU-only half of the same spike, §47.1): where `rope-spike` measured the rope
data structure alone with no renderer at all, this spike adds the previously
unexecuted other half — a real `wgpu` surface, real shaped/rasterized text
from the real `spartan-buffer::Document`, real keyboard-driven edits, a real
cursor, and a first real (not estimated) input-to-photon latency number.

This closes the historical blocker recorded in `CLAUDE.md` and §39/§47 —
"Spike 0.1's GPU half... has never run — no display/GPU in either
environment this was built in." That was true of every earlier session. It
is not true of this machine: a real `wgpu 0.19` adapter/device probe
succeeded here (Intel UHD Graphics 620, Vulkan backend, `IntegratedGpu`), so
this increment runs on real hardware, not a software/CPU fallback.

**This spike is not closed.** §39.1's full scope — a persistent B-tree rope
(rope-spike already covers a rope's raw insert/clone cost, but this spike's
`Document` isn't wired to real damage-region-aware rendering), an SDF glyph
atlas, damage-region re-rasterization, a full keystroke-trace benchmark
corpus (steady typing, held-key repeat, large paste, rapid undo/redo,
scroll-while-typing) replayed against defined reference hardware, and a
formal go/no-go recommendation — is a 3-4 week, 2-engineer effort per its own
spec table. What follows is a real, honestly-scoped first slice of it.

## What was built

- `gpu.rs` — real `wgpu` instance/adapter/device/surface setup and resize
  handling. No mocked adapter; `AdapterInfo` is printed and checked at
  startup on every run.
- `text.rs` — real text shaping and GPU rasterization via `glyphon`
  (wrapping `cosmic-text` → `rustybuzz` for shaping, `swash` for
  rasterization onto a coverage-mask GPU atlas). This is a real but
  *different* technique from a literal per-glyph signed-distance field —
  named explicitly here, not glossed over (see "What this does not confirm").
- `lib.rs` / `editor_view.rs` — a small library crate wrapping the real,
  already-tested `spartan_buffer::Document` (the same buffer model the rest
  of the project uses, not a throwaway `String`), plus the one thing the
  buffer itself doesn't own: cursor position. Split into a library so
  `tests/editor_view_maps_document_state.rs` can exercise real
  `Document`-to-render-input mapping headlessly — no GPU, no window, no
  display required for these 10 tests.
- `input.rs` — real `winit` `KeyEvent`s mapped to real `Document::insert`/
  `delete` calls.
- `cursor.rs` / `cursor.wgsl` — a real second render pipeline drawing a
  filled-quad caret, positioned every frame from the cursor's char index via
  `cosmic-text`'s own `layout_runs()` (not a naive `column * char_width`
  guess).
- `latency.rs` — real `Instant`-based input-to-photon timestamping (from a
  `KeyboardInput` that changes the `Document` to the frame that presents the
  result), a bounded ring buffer, and `percentiles()` ported **verbatim**
  from `rope-spike`'s (identical format string) so the two reports are
  directly comparable.
- `fixture.rs` — `synthetic_file()`, also ported **verbatim** from
  `rope-spike`'s generator (byte-for-byte identical function body, only the
  `pub` differs), so both spikes measure the same corpus shape.
- A `--synthetic:<lines>` CLI mode generates that corpus in-memory, and an
  optional second CLI argument runs N **internally-scripted** random-position
  inserts (driving `Document`/`TextState` directly on the same thread) and
  then exits with a final report — stated plainly: this is not real
  OS-level synthetic input. Real OS-level input (via a separate `enigo`-based
  tool driving actual `WM_KEYDOWN`/`WM_KEYUP` through the real window) *was*
  used to verify Steps 4 and 5 functionally, by hand, with screenshots — see
  "What was verified by hand" below — but the latency numbers in this report
  come from the internal scripted driver, since it's the only way to get a
  clean, repeatable 2000-sample run.

## Two real bugs found by running this, not by inspection

1. **sRGB gamma mismatch on the clear color.** `wgpu::Color` clear values are
   linear-space, but the chosen surface format is sRGB — an intended dark
   `(0.08, 0.08, 0.09)` background rendered as a visibly lighter gray.
   Diagnosed by first clearing to an unmistakable pure red to confirm the
   pipeline worked at all, then applying a real sRGB EOTF conversion
   (`s/12.92` below the 0.04045 knee, `((s+0.055)/1.055)^2.4` above it) to
   both the clear color and the cursor's fragment-shader output color.
2. **`Document` (ropey) and `cosmic-text`'s `Buffer` disagree about how many
   lines a file ending in `"\n"` has.** Ropey treats the position after a
   final newline as the start of one more, empty line (`char_to_line` at
   end-of-document returns that phantom line's index). `cosmic-text` never
   synthesizes a `BufferLine` past the last line terminator, so that phantom
   line has no `layout_runs()` entry at all — a naive cursor-position lookup
   silently found nothing and drew no caret whenever the cursor sat at true
   end-of-file on such a document. Fixed in `TextState::cursor_pixel_pos` by
   detecting exactly that case (one line past the last laid-out line, at
   column 0) and positioning the caret one row below the last real line.
   Locked in as a regression test
   (`cursor_line_col_on_the_phantom_trailing_line_after_a_final_newline`).

Neither of these is a bug in `ropey`, `cosmic-text`, or `wgpu` — they're real
seams between independently-correct libraries with different conventions,
the exact kind of thing a spike like this exists to surface before the real
Tier 1 editor is built on top of the same assumption.

## What was verified by hand (Steps 1-5)

Each of Steps 1-5 was checked by actually running the program, not read off
the code:

- **Step 1**: window opens, resizes without crashing, and prints the real
  `AdapterInfo` (`Intel(R) UHD Graphics 620 | backend=Vulkan |
  device_type=IntegratedGpu`) — not a software/CPU fallback.
- **Step 2-3**: screenshots confirmed legible, correctly-shaped glyphs
  rendering the real byte-for-byte content of `crates/spartan-buffer/src/lib.rs`.
- **Step 4**: a standalone `enigo`-based tool sent real OS-level virtual-key
  events (`WM_KEYDOWN`/`WM_KEYUP`, not Unicode/`WM_CHAR` injection, which
  `winit`'s keyboard pipeline doesn't consume) into the real window; a
  screenshot showed the typed/backspaced text land correctly in the
  document's rendered content.
- **Step 5**: the same tool, against a small fixture so the whole file fit
  on screen, typed `xyz123` then two backspaces; a screenshot showed the
  caret (the theme's accent rust/terracotta color, §50.3) sitting exactly
  after the resulting `xyz1`, confirming it tracks real typing and
  backspacing, not just a fixed position.

## Latency results

Environment: Intel(R) UHD Graphics 620, Vulkan backend, `IntegratedGpu`,
Windows, GNU (`stable-x86_64-pc-windows-gnu`) toolchain, release build.
Corpus: `fixture::synthetic_file(50_000)` — 50,000 lines, 3,527,780 bytes —
byte-for-byte the same generator as `rope-spike`'s (§47.1).

Reproduction:

```
cargo run --release -p render-spike -- --synthetic:50000 2000
```

Primary run (2000 internally-scripted random-position inserts):

```
=== render-spike -- Spike 0.1 GPU-half, first increment ===
Loaded --synthetic:50000 -- 3527780 chars, 50000 lines
Scripted latency benchmark: 2000 internally-driven random-position inserts
Adapter: Intel(R) UHD Graphics 620 | backend=Vulkan | device_type=IntegratedGpu
Cold-open: process start -> first presented frame = 897.67ms
  input-to-photon              p50=190.8706ms  p95=217.5321ms  p99=239.9208ms  max=274.7382ms  n=200
  input-to-photon              p50=185.4860ms  p95=216.3035ms  p99=238.9643ms  max=274.7382ms  n=400
  input-to-photon              p50=176.6991ms  p95=214.5598ms  p99=236.4617ms  max=274.7382ms  n=600
  input-to-photon              p50=172.8520ms  p95=209.9455ms  p99=238.9643ms  max=308.7432ms  n=800
  input-to-photon              p50=170.8020ms  p95=205.9404ms  p99=233.6879ms  max=308.7432ms  n=1000
  input-to-photon              p50=170.5588ms  p95=202.9564ms  p99=233.6879ms  max=308.7432ms  n=1200
  input-to-photon              p50=169.6722ms  p95=200.8906ms  p99=232.7924ms  max=308.7432ms  n=1400
  input-to-photon              p50=169.5093ms  p95=199.0865ms  p99=230.1791ms  max=308.7432ms  n=1600
  input-to-photon              p50=169.3801ms  p95=197.4690ms  p99=226.4592ms  max=308.7432ms  n=1800
  input-to-photon              p50=169.2963ms  p95=195.8225ms  p99=223.9138ms  max=308.7432ms  n=2000

=== Scripted benchmark complete (2000 inserts) ===
  input-to-photon (scripted, final) p50=169.2963ms  p95=195.8225ms  p99=223.9138ms  max=308.7432ms  n=2000
```

Independent cross-check run (500 iterations, same fixture, run separately
immediately afterward — not the same process, not reused numbers):

```
Cold-open: process start -> first presented frame = 899.67ms
  input-to-photon (scripted, final) p50=185.1179ms  p95=223.2851ms  p99=266.0589ms  max=297.3507ms  n=500
```

The two runs land in the same ballpark (p50 169-185ms, cold-open 897-900ms)
without matching to the decimal — real measurement variance, not a
fabricated or copy-pasted number.

For scale/contrast, the same 500-iteration scripted benchmark against much
smaller files, run earlier in the same session on the same machine:

```
target/tiny_fixture.txt (3 lines, 34 bytes):
  input-to-photon (scripted, final) p50=  1.2086ms  p95=  2.0673ms  p99=  2.4430ms  max=  8.2983ms  n=500

crates/spartan-buffer/src/lib.rs (521 lines, 20,570 bytes):
  input-to-photon (scripted, final) p50=  9.1959ms  p95= 10.6763ms  p99= 15.3142ms  max= 19.5761ms  n=500
```

## Against §39.1's actual success criteria

| Criterion | Target | This increment | Verdict |
|---|---|---|---|
| p99 input-to-photon | <5ms | 223.9ms (50k lines) | **Fails, by ~45x** |
| Cold file open to first paint | <100ms | 897.7ms (50k lines) | **Fails, by ~9x** |
| Rope memory overhead vs. flat buffer | <20% | Not measured this pass | Not evaluated |

This is not a subtle miss — it's the expected, honest consequence of a named
and deliberate shortcut: `TextState::set_text` re-shapes the *entire*
document on every single edit (see its own doc comment), with no damage-region
tracking of any kind. At 50,000 lines that full reshape dominates the
input-to-photon budget completely; the latency numbers scale visibly with
document size (34 bytes: ~1.2ms p50; 20.5KB: ~9.2ms p50; 3.5MB: ~169ms p50),
which is itself evidence the instrumentation measures something real rather
than returning a constant. Meeting §39.1's <5ms target on a 50k-line file
requires the damage-region re-rasterization this increment explicitly
skipped — that is the next real increment, not a tuning pass on this one.

## What this confirms

- The GPU half of Spike 0.1 is no longer simply "never run" — it has now
  executed, repeatedly, on real hardware (Intel UHD 620, Vulkan), producing
  real, reproducible (not fabricated or hand-estimated) latency numbers.
- A real `wgpu`/`winit`/`glyphon` pipeline can render the project's actual
  `spartan-buffer::Document` content, accept real keyboard input, and drive
  real edits through the same `Document` API the rest of the project already
  depends on and tests.
- The latency instrumentation itself is trustworthy: numbers are non-zero,
  internally consistent across repeated runs (169-190ms p50 range across two
  independent 50k-line runs — real measurement variance, not suspiciously
  identical decimals), and respond predictably to a real load variable
  (document size).
- Two genuine cross-library seams (sRGB clear-color gamma; ropey vs.
  cosmic-text's differing trailing-newline line-count semantics) were found
  and fixed only by actually running the code, consistent with this
  project's established discipline (§48, §51.1).

## What this does not confirm

- **§39.1's <5ms p99 / <100ms cold-open success criteria are not met** at
  50k lines with this increment's full-reshape-every-edit approach — see the
  table above. Closing that gap needs real damage-region re-rasterization,
  not implemented here.
- **No true damage-region rendering of any kind.** Every edit re-shapes and
  re-uploads the entire visible buffer content; this is explicitly the
  simplest-possible-first-slice choice, named in `text.rs`'s own doc comment,
  not an oversight.
- **`glyphon`'s coverage-mask atlas, not a literal per-glyph SDF field.**
  Functionally similar (both are texture-atlas-based GPU text rendering) but
  a different technique from what §2.2/§39.1 literally specify. Hand-rolling
  a real SDF atlas (font parsing, rect packing, SDF generation, a custom
  alpha-thresholded shader) remains unattempted.
- **No damage-region-aware rope**, no branching undo tree wired into this
  spike (`spartan-buffer::Document` has one — see `crates/spartan-buffer` —
  but this spike's benchmark never exercises undo/redo, snapshotting, or
  the rope's own clone cost; that's `rope-spike`'s territory, §47.1).
- **No keystroke-trace corpus.** §39.1 calls for replaying steady typing,
  held-key repeat, large-block paste, rapid undo/redo, and scroll-while-typing
  as a full trace suite against defined reference hardware. This report
  covers exactly one pattern — single-character random-position inserts —
  run internally-scripted, not as literal OS-level synthetic input (real
  OS-level input was used only for Steps 4-5's functional, not latency,
  verification — see above).
- **One machine, one integrated GPU, one operating system.** No discrete GPU,
  no macOS/Linux, no other display backend was available to test against in
  this environment.
- **No formal go/no-go recommendation.** That's explicitly this spike's
  eventual exit artifact per §39.1, and isn't warranted yet given the gaps
  above.

**This spike is not closed. Only this first, honestly-scoped slice of its
GPU half is.**

## Reproducing this report

```bash
# Build
cargo build --release -p render-spike

# Headless unit tests (no GPU required)
cargo test -p render-spike --release

# Interactive mode — opens a real window, accepts real keyboard input
cargo run --release -p render-spike -- crates/spartan-buffer/src/lib.rs

# Scripted latency benchmark against the real 50k-line corpus (§47.1's shape)
cargo run --release -p render-spike -- --synthetic:50000 2000
```
