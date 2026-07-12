# tree-sitter-wasm-spike — real syntax-parsing feasibility for the web app

Real, runnable code, not a feasibility argument. Second real preparation step
for the planned vscode.dev-inspired web app (§75.85's hybrid architecture) --
proves the client-side syntax-parsing half works: can `web-tree-sitter`
(the standard WASM build of tree-sitter for browser/JS use) actually parse
real source and run real queries against it, in a real JS engine?

## What was tested

Two real, prebuilt grammars from `tree-sitter-wasms` (Rust and Python,
covering two of Spartan's 7 Tier 1 languages -- the package bundles
prebuilt WASM for all 7, including C#, Java, Kotlin, TypeScript, and
JavaScript, though only two were exercised this pass). Real, executed
verification via `node --test`, 6 tests, all passing:

- The real grammar parses valid source with zero syntax errors, and
  correctly reports a genuine syntax error on invalid source (not silently
  accepting garbage).
- A real field lookup (`childForFieldName("name")`) resolves a function's
  actual name node -- confirms the parse tree's structure, not just that
  parsing "completed."
- A real `Query` compiled from a real `.scm` query file, run against a real
  parsed tree, produces real captures with the correct names and the
  correct underlying node text (e.g. the captured `function` node's text is
  literally `"add"` for the Rust fixture, `"greet"` for the Python one).

One real test-writing mistake was caught only by running the suite, not by
inspection: the first version of `fixtures/sample.rs` had no integer
literals at all (`a + b` are variables, not literals), so the `@number`
capture assertion correctly failed -- fixed by adding a real literal to the
fixture, not by weakening the assertion.

## A real version-compatibility finding, not assumed compatible

The first attempt used the latest `web-tree-sitter` (0.26.10) against
`tree-sitter-wasms`' prebuilt grammars and failed immediately inside
`Language.load()` with a low-level WASM "dylink" module-format error.
Traced by reading `web-tree-sitter`'s own bundled source: 0.26.x's
`Language.load()` now requires grammars built as Emscripten dynamic-link
("side module") WASM binaries, a newer build convention. `tree-sitter-wasms`
own `package.json` pins `tree-sitter-cli: ^0.20.8` (a much older CLI
generation) as its build tool, predating that convention -- a real,
confirmed ecosystem version gap between the (frequently updated)
`web-tree-sitter` client library and the (less frequently updated)
community prebuilt-grammar package, not a bug in either project
individually.

**Fixed by using `web-tree-sitter@0.20.8`** -- the same era as the grammars
were actually built for (`Parser.Language.load()`, a nested class, is that
version's real API shape; 0.26.x moved `Language` to a top-level export,
among other API changes). This is the version pinned in `package.json`.

A second, smaller real finding from the same investigation: the *query
text* also needs to match the grammar's own generation, separately from the
WASM loading format above. The real, current `tree-sitter-rust` crate's own
bundled `highlights.scm` (crates.io 0.24.2, the exact query
`crates/spartan-editor-core`'s own `highlight.rs` uses on the Rust side)
references a `doc_comment` node type that doesn't exist in the older
grammar generation bundled by `tree-sitter-wasms` -- confirmed via a real
`RangeError: Bad node name 'doc_comment'` thrown by `Language.query()`. Not
worked around by degrading the assertion; the two `queries/*.scm` files in
this spike are deliberately minimal, hand-written, version-safe subsets
(comment/string/function-name/number captures only), not the full
production queries -- reusing the real production highlights queries is
real, separate follow-up work that needs either a newer, dylink-format
grammar source or a query written against the older grammar generation.

## What this does and doesn't confirm

**Confirmed, real, and load-bearing**: tree-sitter parsing and querying,
the same real mechanism `spartan-editor-core`'s own Rust-side highlighting
already uses, genuinely works through a WASM build in a real JS engine --
for two different languages, ruling out "one language happens to work" as a
fluke (matching the general lesson `spikes/README.md`'s own §47.7 section
states about not generalizing from a single example, even though that
lesson was originally about DAP/LSP adapter divergence, not WASM grammars).

**Not attempted in this pass**, each a real, separate, still-open piece:
reusing the exact production highlight queries the desktop shells already
use (blocked on the grammar-generation mismatch above -- needs either
building fresher WASM grammars via `tree-sitter-cli build-wasm` against
this repo's own already-vendored grammar crates, likely requiring Docker or
Emscripten, or hand-porting each language's query to the older grammar
generation); a real browser-environment run (only Node was exercised);
incremental re-parsing (`Tree#edit` + old-tree reuse, real and supported by
`web-tree-sitter`, just not exercised here); the other 5 bundled Tier 1
grammars (TS/JS, Kotlin, Java, C#, Go -- all confirmed *present* in
`tree-sitter-wasms`' file listing, so no further grammar-sourcing work is
expected, but none has actually been loaded/parsed/queried yet, so "present
in the package" is not the same claim as "confirmed working" the way Rust
and Python now are); real bundle-size measurement of the WASM runtime +
grammar files together (the Rust grammar alone is ~800KB, each additional
language grammar is a separate download, real cost not yet measured for a
realistic language set).
