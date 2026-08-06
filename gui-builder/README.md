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
  `ComponentInsert` (including string props, optional direct text, and dot-separated member-expression tags such as `UI.Button`), `Delete`, `Duplicate`, `TextChange`, and `PropRemove`, matching the shape
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
  preserving value edits.
- `src/bundle.ts` — real esbuild bundling with `data-spartan-id` annotation and
  sandboxed-preview click/drag relays, including an in-memory source path for
  previews of unsaved editor changes.
- `src/*.test.ts` — 113 real tests (Node's built-in `node:test` runner, no
  extra test-framework dependency), including several that run directly
  against this repo's own real `prototypes/*.jsx` files (5,480 real lines
  combined) — not just synthetic snippets.

## What this is not (real, honest scope cuts)

- **No HMR or per-keystroke rebuild.** The desktop Design screen refreshes the
- **No HMR or per-keystroke rebuild.** The desktop Design screen refreshes the
  bundle on file activation and after a real Canvas → Code edit, using the
  current in-memory source for correctness. A future incremental preview can
  add HMR after measuring rebuild cost.
- **No font manager or code-authored component state machine runtime yet.** Font assets can
  be discovered and copied, and CSS token definitions can be edited from the
  desktop suite; the desktop now provides reusable per-file interaction-state
  presets for the live preview, while code-authored state-machine generation
  remains future work.
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
