/**
 * Real §6.2 two-way sync data model (task #12). `ComponentNode` is the
 * "Code -> Canvas" side (a simplified component tree a real canvas UI
 * would render and let a user click on); `CanvasEdit` is the "Canvas ->
 * Code" side (a structured edit event, matching the shape already
 * sketched in `docs/architecture-spec.md` §6.2's own Rust enum, ported to
 * TypeScript since this package -- not the Rust IDE crate -- is where the
 * real JS/JSX AST work actually happens, per §6.1's own "lightweight
 * dev-server bridge" description).
 *
 * Deliberately a real, honest v1 subset of the full spec: only
 * `StyleChange` and `PropChange` are implemented by `applyCanvasEdit`
 * (see edit.ts's own doc comment for why `Reparent`/`ComponentInsert`
 * are named, not attempted, in this pass).
 */

/** A single style object entry's value -- a real literal (safely editable
 * without losing information) or a real, verbatim expression (a design
 * token reference like `color: C.text`, a template literal, a function
 * call, ...). Real fixtures in this repo's own `prototypes/*.jsx` mix
 * both kinds freely within one style object (numeric literals alongside
 * token-variable colors), so this is a per-key distinction, not an
 * all-or-nothing one for the whole style prop. */
export type StyleEntryValue = { kind: "literal"; value: string } | { kind: "expression"; source: string };

export type PropSummary =
  | { kind: "string"; value: string }
  | { kind: "style"; entries: Record<string, StyleEntryValue> }
  /** A prop whose value isn't a plain string or a plain-object style
   * value at all -- `{ count }`, `{() => onClick()}`, `style={spread}`,
   * etc. Recorded as the real, verbatim source text of the expression
   * rather than evaluated (this package never executes user code) or
   * dropped silently. */
  | { kind: "expression"; source: string };

export interface ComponentNode {
  /** Stable *within one parse*, assigned by a deterministic depth-first
   * pre-order traversal counting `JSXElement` nodes only (see ids.ts).
   * Not a persistent identity across structural edits -- see ids.ts's own
   * doc comment for the real, named limitation this implies. */
  id: string;
  tagName: string;
  props: Record<string, PropSummary>;
  children: ComponentNode[];
  /** Direct text children only, concatenated; `null` if this element has
   * no direct text children (only element/expression children, or none
   * at all). A real, deliberate simplification -- text interleaved with
   * element children (`<p>Hello <b>world</b></p>`) is not reconstructed
   * as a single ordered sequence in this v1 tree. */
  textContent: string | null;
}

export type CanvasEdit =
  | { kind: "StyleChange"; nodeId: string; property: string; value: string }
  | { kind: "PropChange"; nodeId: string; prop: string; value: string };
