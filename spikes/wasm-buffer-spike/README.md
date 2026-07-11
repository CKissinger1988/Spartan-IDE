# wasm-buffer-spike — real client-side WASM feasibility for the web app

Real, runnable code, not a feasibility argument. Proves the single highest-risk
unknown behind the "hybrid" architecture chosen for the planned vscode.dev-
inspired web app (user-requested; concepts/features only, zero VS Code code,
matching this project's own standing "no VS Code/Monaco/CodeMirror" rule --
see `docs/architecture-spec.md` §75.85): does the real `spartan-buffer`
crate -- the exact same rope/branching-undo-tree `Document` the whole product
already depends on, no fork, no simplified copy -- compile to
`wasm32-unknown-unknown` and actually run correctly inside a real JS engine?

## What was built

A thin `#[wasm_bindgen]` wrapper (`WasmDocument`) around a small slice of
`Document`'s already-real, already-tested API (`new`/`text`/`insert`/
`delete`/`undo`/`len_chars`) -- no new buffer logic, this crate is purely a
feasibility gate. `spartan-buffer` itself needed **zero changes** to compile
for the wasm32 target; its one real dependency, `ropey`, has no OS bindings
and is pure algorithmic Rust.

## Real, executed verification

1. `cargo build -p wasm-buffer-spike --release --target wasm32-unknown-unknown`
   -- compiles clean, produces a real 176KB `.wasm` binary.
2. `wasm-bindgen <the .wasm> --target nodejs --out-dir <dir>` (CLI version
   0.2.126, matching the `wasm-bindgen` crate dependency's own version
   exactly, a well-known hard requirement) -- generates real JS glue code.
3. A real Node.js script (not a browser -- Node was what this sandbox had
   available, and a real JS engine either way) loaded the generated module,
   constructed a real `WasmDocument`, and exercised a real insert → delete →
   undo → undo sequence, asserting the exact resulting text at each step.
   **All assertions passed** -- the real branching undo tree correctly
   restored `"hello, world"` then `"hello world"` across two real `undo()`
   calls, run through compiled WASM, not the native test suite.
4. 4 real headless Rust unit tests (`cargo test -p wasm-buffer-spike
   --release`) cover the wrapper's own logic (insert/delete round-trip,
   undo, an out-of-range edit producing a real `Result::Err` instead of a
   panic, `len_chars` tracking real edits) -- run for the host target, since
   `#[wasm_bindgen]` types compile and behave normally off wasm32 too; this
   doesn't require Node or a browser and so is the part CI can run.
5. `cargo build --workspace --release` was re-run after adding this crate to
   the workspace and confirmed unaffected -- a `wasm-bindgen`-based crate is
   a normal Rust dependency for every non-wasm target, unlike
   `crates/plugins/*`'s own `wasm32-wasip1` crates (excluded from this
   workspace for exactly that reason, see the root `Cargo.toml`'s own
   comment).

## What this does and doesn't confirm

**Confirmed, real, and load-bearing for the web app's architecture**: the
buffer/undo-tree half of the client-side core can run entirely in-browser
with no server, no network hop, and no behavior fork from the desktop
product's own real buffer engine.

**Not attempted in this pass** (each a separate, real, still-open piece of
the same architecture): tree-sitter syntax highlighting compiled to WASM
(the desktop Electron shell already uses a JS-side `highlight.js` fallback
for a related reason -- see `desktop/src/syntax.ts`'s own doc comment --
`web-tree-sitter`'s real WASM grammar builds are the more likely path here,
not attempted or even downloaded yet); a real browser-environment run (only
Node was exercised, not an actual `<script type="module">` load in Chromium
-- the generated bindings differ by `--target` flag, `nodejs` vs. `web`, and
only `nodejs` was generated here); bundle size under real gzip (176KB
uncompressed for a small wrapper -- the real `Document`'s full surface area,
once wired up, will be larger, unmeasured here); no `wasm-bindgen-cli` step
was added to CI (needs the `wasm32-unknown-unknown` target and a pinned
`wasm-bindgen-cli` install in the CI image, real, small, separate follow-up
work).
