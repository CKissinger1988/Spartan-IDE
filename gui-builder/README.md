# @spartan-ide/gui-builder

Real implementation of §6.2's "Two-Way Sync Mechanism" (task #12) — a real
npm/TypeScript package, not a mock or a design document. It is now wired into
the desktop GUI Builder Design screen. See
`docs/architecture-spec.md` §6 and §75.38 for the full write-up this README
summarizes.

## What this is

- `src/parse.ts` (`parseComponent`) — real "Code → Canvas": parses JSX/TSX
  source with `@babel/parser` (via `recast`'s own parser adapter,
  `src/parserAdapter.ts`) into a `ComponentNode[]` tree — tag names, a
  per-prop summary (`string` / `style` / `expression`), text content, and
  nested children — the shape a real canvas UI would render and let a user
  click on.
- `src/edit.ts` (`applyCanvasEdit`) — real "Canvas → Code": takes a
  structured `CanvasEdit` (`StyleChange`, `StyleRemove`, `StyleClear`, `StyleClearMany`, `PropChange`, `TagChangeMany`, `TextChangeMany`, `Reparent`, `DeleteMany`, `DuplicateMany`, `ReorderMany`, or
  `ComponentInsert`/`ComponentInsertMany` (including string and typed number/boolean/expression props, optional direct text, dot-separated member-expression tags such as `UI.Button`, and atomic child/sibling placement across selections), `SubtreeInsert` for guarded same-file JSX subtree paste, `Delete`, `Duplicate`, `TextChange`, and `PropRemove`, matching the shape
  §6.2 already sketched in Rust) and mutates the *real AST node* it targets
  directly, then regenerates source via `recast.print`, which reuses the
  original source text for every node the edit didn't touch. This is the
  real mechanism behind §6.2's "preserves formatting, comments, and existing
  code structure" requirement — not string templating, not a full rewrite.
- `src/tree.ts` — the one canonical depth-first traversal both of the above
  are built on, so their node-id numbering can never drift apart from each
  other by construction. Ids are a pure function of tree structure ("n0",
  "n1", ... in document order over `JSXElement` nodes only), recomputed
  fresh on every parse — not a persistent identity.
- `src/components.ts` — real project component discovery with safe
  depth-bounded scanning and import-specifier generation for the component
  palette.
- `src/assets.ts` — real depth-bounded image/font asset discovery with
  project-relative paths and JSX references computed from the open component file.
- `src/tokens.ts` — real CSS custom-property discovery for applying existing
  design tokens as `var(--token)` values in inline styles, plus safe source-
  preserving definition creation, value edits, and safe removal, including
  unsaved source buffer discovery.
- `src/bundle.ts` — real esbuild bundling with `data-spartan-id` annotation and
  sandboxed-preview click/drag relays, including an in-memory source path for
  previews of unsaved editor changes.
- The desktop preview uses generation-guarded parse/bundle refreshes, so a
  slower response for an older unsaved buffer cannot overwrite the newest tree
  or iframe bundle.
- Preview variants can be copied as versioned `spartan.gui-builder.variant`
  JSON to the system clipboard and imported into another open component file;
  when clipboard permission is unavailable, the Design screen keeps a
  session-local fallback. Imported snapshots retain their source path so the
  UI can warn when relative imports may need adjustment.
- The desktop font palette can request a stylesheet-relative font snippet for
  the selected open CSS file and append it through the normal editor history
  path, refusing an existing matching family or URL.
- The sandbox inspection relay includes viewport-relative `x`, `y`, `right`,
  and `bottom` bounds alongside width and height, so the desktop inspector and
  copied handoff snapshots describe the element's actual rendered position.
- The Design screen can paste a copied subtree as either a child or the next
  sibling of one or many selected elements; root sibling targets are refused
  safely and multi-target insertion is atomic.
- The Design inspector includes an explicitly labelled estimated screen-reader
  announcement showing the selected element's inferred role, accessible name,
  heading level, and parsed ARIA state. It is included in copied accessibility
  reports and design handoffs; dynamic labelled-by resolution remains clearly
  identified as an estimate.
- The Design screen supports dedicated subtree clipboard chords:
  `Ctrl/Cmd+Alt+B` copies the selected subtree and
  `Ctrl/Cmd+Alt+P` pastes it using the selected child/sibling placement;
  the established style and prop clipboard chords remain unchanged.
- The inspector can also download the selected live inspection as a portable
  Markdown design spec containing source JSX, rendered styles/bounds, audit
  findings, and the estimated screen-reader announcement.
- Design mode can generate a typed TSX component scaffold from a name, default
  variant, and line-based prop schema (`string`, `number`, `boolean`, `enum`, or
  `slot`); the desktop shell creates the file only inside the active project and
  opens it in the Editor.
- `src/*.test.ts` — 181 real tests (Node's built-in `node:test` runner, no
  extra test-framework dependency), including several that run directly
  against this repo's own real `prototypes/*.jsx` files (5,480 real lines
  combined) — not just synthetic snippets.

## What this is not (real, honest scope cuts)

- **No HMR or per-keystroke rebuild.** The desktop Design screen refreshes the
  bundle after a debounced edit or a real Canvas → Code edit, using the current
  in-memory source for correctness. A future incremental preview can add HMR
  after measuring rebuild cost.
- **No full font manager or code-authored component state machine runtime yet.** Font assets can
  be discovered, applied to selected elements, and added to an open stylesheet
  with a generated `@font-face` declaration; CSS token definitions can be
  edited from the desktop suite. The desktop also provides reusable per-file
  interaction-state presets for the live preview, while code-authored
  state-machine generation remains future work.
- **`PropChange` supports string, number, boolean, and parsed expression
  values.** Expressions are parsed as JavaScript/JSX and never injected as
  raw source fragments.
- **No Figma import, no screenshot-to-component.** Separate §6.4 items,
  unrelated to two-way sync itself.

## A known, real limitation — not fixed, named on purpose

Once an edit forces `recast` to fully reprint a `JSXElement` (rather than
patch just the attribute that changed), that reprint regenerates the
element's *children* using JSX's own pretty-printer, which normalizes
`JSXText` whitespace the same way React's JSX runtime does at render time. A
leading newline+indentation directly after a `{expression}` sibling can
collapse to nothing. The **rendered** output is unchanged (React already
treats `{a}\n  text` and `{a}text` identically) but the **source formatting**
is not always byte-for-byte preserved in this specific shape (a
`JSXExpressionContainer` immediately followed by mixed text+expression
content on the next source line).

This was found live against this repo's own `prototypes/interface-prototype.jsx`
(a real ~5,000-line fixture), isolated to a minimal repro, and confirmed to
be an upstream `recast`/Babel-JSX-generator behavior, not a bug in this
package's own mutation logic — reproduces identically even when mutating an
existing AST node's value *in place* (same object reference, no new node
created). `src/edit.test.ts` has a dedicated test
(`KNOWN LIMITATION: ...`) that asserts this exact behavior, specifically so
a future `recast` upgrade or printer-option fix that closes the gap fails
that test loudly instead of the fix going unnoticed.

## Build & test

```bash
cd gui-builder
npm install
npm test    # node --import tsx --test src/**/*.test.ts
npm run build
```
