/**
 * Real §6.2 "Code -> Canvas" half (task #12): a single, canonical
 * depth-first pre-order traversal (`buildComponentTree`) that both
 * produces the `ComponentNode` tree a real canvas UI would render *and*
 * populates an id -> raw-AST-node lookup map `applyCanvasEdit` (edit.ts)
 * uses to find the exact node a `CanvasEdit` targets -- one traversal,
 * shared by both directions of the sync, so their id numbering can never
 * drift apart from each other by construction, not by coincidence between
 * two separately-written traversals.
 */
import type { ComponentNode, PropSummary, SourceLocation, StyleEntryValue } from "./types.js";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type AnyNode = any;

export interface BuiltTree {
  roots: ComponentNode[];
  /** Real AST node objects (recast/Babel shape), keyed by the same ids
   * `roots` uses -- `applyCanvasEdit` mutates these directly rather than
   * re-parsing or string-templating. */
  nodesById: Map<string, AnyNode>;
  /** The real parent `JSXElement` AST node for each id, or `null` for a
   * top-level root (its "parent" is a function body/module scope, not
   * another `JSXElement`'s own `.children` array). `Reparent` uses this
   * to splice a node's real AST node out of its current parent's real
   * `.children` array -- the exact array `recast.print` will re-emit
   * from, not a separately reconstructed one. */
  parentOf: Map<string, AnyNode | null>;
}

function isJsxElement(node: AnyNode): boolean {
  return !!node && typeof node === "object" && node.type === "JSXElement";
}

function tagNameOfNamePart(name: AnyNode): string {
  return name?.name ?? "unknown";
}

function tagNameOf(node: AnyNode): string {
  const name = node.openingElement?.name;
  if (!name) return "unknown";
  if (name.type === "JSXIdentifier") return name.name;
  if (name.type === "JSXMemberExpression") {
    return `${tagNameOfNamePart(name.object)}.${tagNameOfNamePart(name.property)}`;
  }
  return "unknown";
}

/** A JSX attribute value counts as a real, summarizable "style object" if
 * it's a plain object expression with no spread elements -- a spread
 * (`{...base, color: "red"}`) makes per-key merge order matter in a way
 * this v1's flat `entries` map can't safely represent, so the *whole*
 * style prop falls back to the honest `"expression"` kind in that case.
 * Individual properties within a real (non-spread) style object are
 * summarized per-key instead (see `styleEntriesOf`) -- real fixtures in
 * this repo's own `prototypes/*.jsx` mix literal values (`fontSize: 12`)
 * with design-token variable references (`color: C.text`) freely within
 * one object, so an all-or-nothing literal-only check would reject
 * almost every real style prop in this codebase's own prototypes. */
function isSummarizableStyleObject(expr: AnyNode): boolean {
  if (!expr || expr.type !== "ObjectExpression") return false;
  return expr.properties.every((prop: AnyNode) => prop.type === "ObjectProperty" && !prop.computed);
}

function sourceSlice(source: string, node: AnyNode): string {
  if (typeof node.start !== "number" || typeof node.end !== "number") {
    return "<unknown>";
  }
  return source.slice(node.start, node.end);
}

function styleEntriesOf(source: string, expr: AnyNode): Record<string, StyleEntryValue> {
  const entries: Record<string, StyleEntryValue> = {};
  for (const prop of expr.properties) {
    const key = prop.key.type === "Identifier" ? prop.key.name : prop.key.value;
    if (prop.value.type === "StringLiteral") {
      entries[key] = { kind: "literal", value: prop.value.value };
    } else if (prop.value.type === "NumericLiteral") {
      entries[key] = { kind: "literal", value: String(prop.value.value) };
    } else {
      entries[key] = { kind: "expression", source: sourceSlice(source, prop.value) };
    }
  }
  return entries;
}

function summarizeAttrValue(source: string, value: AnyNode): PropSummary {
  if (value === null) {
    // Boolean-shorthand JSX attribute, e.g. `<input disabled />`.
    return { kind: "string", value: "true" };
  }
  if (value.type === "StringLiteral") {
    return { kind: "string", value: value.value };
  }
  if (value.type === "JSXExpressionContainer") {
    const expr = value.expression;
    if (isSummarizableStyleObject(expr)) {
      return { kind: "style", entries: styleEntriesOf(source, expr) };
    }
    return { kind: "expression", source: sourceSlice(source, expr) };
  }
  return { kind: "expression", source: sourceSlice(source, value) };
}

function propsOf(source: string, node: AnyNode): Record<string, PropSummary> {
  const props: Record<string, PropSummary> = {};
  for (const attr of node.openingElement?.attributes ?? []) {
    if (attr.type !== "JSXAttribute" || attr.name.type !== "JSXIdentifier") {
      continue; // Real, named skip: JSXSpreadAttribute (`{...rest}`) has
      // no single prop name to key a `PropSummary` by, so it isn't
      // represented in this v1 tree at all.
    }
    props[attr.name.name] = summarizeAttrValue(source, attr.value);
  }
  return props;
}

function textContentOf(node: AnyNode): string | null {
  const parts = (node.children ?? [])
    .filter((child: AnyNode) => child.type === "JSXText")
    .map((child: AnyNode) => child.value.trim())
    .filter((text: string) => text.length > 0);
  return parts.length > 0 ? parts.join(" ") : null;
}

function sourceLocationOf(node: AnyNode): SourceLocation | undefined {
  const loc = node.loc;
  if (!loc?.start || !loc?.end) return undefined;
  return {
    startLine: loc.start.line,
    startColumn: loc.start.column,
    endLine: loc.end.line,
    endColumn: loc.end.column,
  };
}

function walk(
  node: AnyNode,
  ctx: { source: string; counter: { next: number }; nodesById: Map<string, AnyNode>; parentOf: Map<string, AnyNode | null> },
  seen: WeakSet<object>,
  parent: AnyNode | null,
): ComponentNode[] {
  if (!node || typeof node !== "object") return [];
  if (seen.has(node)) return [];
  seen.add(node);

  if (Array.isArray(node)) {
    return node.flatMap((child) => walk(child, ctx, seen, parent));
  }

  if (isJsxElement(node)) {
    const id = `n${ctx.counter.next++}`;
    ctx.nodesById.set(id, node);
    ctx.parentOf.set(id, parent);
    // Real parent propagation: only a JSXElement's own `.children` array
    // establishes a new "parent" for Reparent/ComponentInsert purposes --
    // an element nested inside an *expression* (`{cond && <div/>}`) still
    // gets `node` as its real parent here, since that's the actual
    // `JSXElement` whose own `.children` array will contain the
    // expression container this element lives inside of. Reparenting out
    // of an expression-wrapped position isn't attempted by this v1 (see
    // edit.ts), but the parent tracking itself stays correct regardless.
    const children = (node.children ?? []).flatMap((child: AnyNode) => walk(child, ctx, seen, node));
    return [
      {
        id,
        tagName: tagNameOf(node),
        sourceLocation: sourceLocationOf(node),
        props: propsOf(ctx.source, node),
        children,
        textContent: textContentOf(node),
      },
    ];
  }

  // Generic descent so JSX nested inside expressions (`{cond && <div/>}`,
  // `{items.map(i => <li key={i}/>)}`) is still discovered, without a
  // full `@babel/traverse` scope-aware visitor -- a real, deliberately
  // simple v1 choice, matching this workspace's own established
  // preference for the smallest real mechanism that actually works
  // (e.g. `spartan-editor-core`'s own windowed-only tree-sitter pass).
  const found: ComponentNode[] = [];
  for (const key of Object.keys(node)) {
    if (key === "loc" || key === "start" || key === "end" || key === "range") continue;
    found.push(...walk(node[key], ctx, seen, parent));
  }
  return found;
}

/** Builds the real component tree for every top-level JSX root reachable
 * from `ast` (a full recast/Babel `File` AST) -- "top-level" meaning not
 * itself nested inside another `JSXElement`'s own `children` (a JSX root
 * returned by one function among several in the same file is its own
 * top-level entry, exactly the "many components share one prototype
 * file" shape this workspace's own `prototypes/*.jsx` fixtures have). */
export function buildComponentTree(ast: AnyNode, source: string): BuiltTree {
  const nodesById = new Map<string, AnyNode>();
  const parentOf = new Map<string, AnyNode | null>();
  const roots = walk(ast, { source, counter: { next: 0 }, nodesById, parentOf }, new WeakSet(), null);
  return { roots, nodesById, parentOf };
}
