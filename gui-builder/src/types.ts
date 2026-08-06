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
 * All six members of the full spec's `CanvasEdit` union are now real and
 * implemented by `applyCanvasEdit`: `StyleChange`/`PropChange` (original
 * v1), and `Reparent`/`ComponentInsert` (closing the gap this file's own
 * doc comment used to name as unattempted -- see edit.ts's doc comment
 * for how the earlier "the id scheme can't survive a structural edit"
 * concern was resolved: both new edits resolve every id they reference
 * against one single fresh parse, the same guarantee `StyleChange`/
 * `PropChange` already relied on, so no cross-parse id stability is
 * actually needed).
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
  | {
      kind: "PropChange";
      nodeId: string;
      prop: string;
      value: string;
      valueType?: "string" | "number" | "boolean" | "expression";
    }
  /** Replaces the direct JSX text content of an element. If the element has
   * no direct text child, a new text child is appended. */
  | { kind: "TextChange"; nodeId: string; text: string }
  /** Removes an existing non-root element and its complete JSX subtree. */
  | { kind: "Delete"; nodeId: string }
  /** Clones an existing non-root element, including its JSX subtree, and
   * inserts the clone immediately after the original among its siblings. */
  | { kind: "Duplicate"; nodeId: string }
  /** Moves an existing element to become a child of a different (or the
   * same, for reordering) parent element. `index` is the position within
   * the *target* parent's children array after the move (default:
   * append at the end) -- plain `Array.splice` semantics, evaluated
   * against the target parent's children array state at insert time
   * (i.e. after the node has already been detached from its old
   * parent). Refused, with a real descriptive error rather than
   * producing a broken tree, for: reparenting a top-level root (it has
   * no parent `JSXElement` to detach from), reparenting an element into
   * itself, and reparenting an element into one of its own descendants
   * (a real cycle). */
  | { kind: "Reparent"; nodeId: string; newParentId: string; index?: number }
  /** Creates a brand-new element and inserts it as a child of `parentId`
   * at `index` (default: append). `tagName` must be a single valid JSX
   * identifier (e.g. `"div"`, `"Card"`) -- member-expression tag names
   * (`Foo.Bar`) are a real, deliberate v1 scope cut, refused with a
   * clear error rather than silently mishandled. `props`, if given, are
   * always inserted as plain string-literal JSX attributes (the same
   * real limitation `PropChange` already has). The new element is
   * always self-closing (`<Tag />`); giving it real children is a
   * separate, unstarted future increment. */
  | {
      kind: "ComponentInsert";
      parentId: string;
      tagName: string;
      index?: number;
      props?: Record<string, string>;
      /** Module specifier to import `tagName` from, when the component
       * being inserted lives in another file (task #278's real
       * component-library browser supplies this). Omitted for a plain
       * DOM tag or a component already declared in the same file --
       * inserting `<Card />` without its import would otherwise
       * regenerate source referencing an undefined binding, breaking the
       * live preview on the very next bundle. `ensureImport` adds as
       * little as possible: nothing when the name is already imported,
       * a merged specifier when an import from the same module exists,
       * and only otherwise a whole new statement. */
      importFrom?: string;
      /** Whether `tagName` is that module's default export -- decides
       * `import Card from "..."` vs. `import { Card } from "..."`. */
      importIsDefault?: boolean;
    };
