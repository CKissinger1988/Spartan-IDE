# @spartan-ide/gui-builder

Real, first-increment implementation of §6.2's "Two-Way Sync Mechanism" (task
#12) — a real npm/TypeScript package, not a mock or a design document. See
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
  structured `CanvasEdit` (`StyleChange` or `PropChange`, matching the shape
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
- `src/*.test.ts` — 21 real tests (Node's built-in `node:test` runner, no
  extra test-framework dependency), including several that run directly
  against this repo's own real `prototypes/*.jsx` files (5,480 real lines
  combined) — not just synthetic snippets.

## What this is not (real, honest scope cuts)

- **No WebView canvas.** §6.1 describes a live WebView surface rendering
  real React output. That needs `spikes/ui-shell-spike`'s already-proven
  wgpu+WebView shell (§47.11) promoted into `spartan-editor-core` first — a
  separate, real, sizable piece of work, not attempted here. This package is
  the AST engine a future canvas would call into, not the canvas itself.
- **No dev-server / HMR.** §6.2 step 1 ("Code → Canvas" live re-render on
  save) needs a running dev server and a live component-tree diff; this
  package only does the one-shot parse.
- **Only `StyleChange` and `PropChange`.** §6.2's own `CanvasEdit` enum also
  names `Reparent` and `ComponentInsert`. Both need a node-identity scheme
  that survives a *structural* edit — this package's plain sequential
  counter cannot do that (inserting or moving a node shifts every later id).
  A real, stable id scheme is separate future work.
- **`PropChange` always sets a plain string literal.** Setting a non-string
  (number/boolean/expression) prop value isn't supported yet.
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
