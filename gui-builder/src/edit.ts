/**
 * Real §6.2 "Canvas -> Code" half (task #12): applies a structured
 * `CanvasEdit` directly onto the real recast/Babel AST (never
 * string-templating) and regenerates source via `recast.print`, which
 * preserves the original formatting/comments/structure of every node this
 * edit doesn't touch -- the real mechanism behind
 * `docs/architecture-spec.md` §6.2's own "preserves formatting, comments,
 * and existing code structure the user wrote by hand" requirement.
 *
 * All four members of the `CanvasEdit` union are real and implemented:
 * `StyleChange`/`PropChange` (mutate an existing element in place) and
 * `Reparent`/`ComponentInsert` (structural edits, added after an earlier
 * pass's own doc comment here named a concern that turned out not to be a
 * real blocker on closer inspection -- every id a `CanvasEdit` references
 * is resolved against the *one single fresh parse* `applyCanvasEdit`
 * itself performs, the same guarantee `StyleChange`/`PropChange` already
 * relied on. A plain sequential counter genuinely cannot survive
 * *across* two separate parses (ids shift after a structural edit,
 * exactly as the earlier concern said) -- but nothing here needs that:
 * the client always re-fetches a fresh tree with fresh ids after every
 * applied edit (§75.42's own already-established pattern), so no id is
 * ever reused across a structural change. `Reparent`'s cycle guard
 * (`isDescendant`, below) is a real, separate, newly-added safety check
 * a plain in-place style/prop edit never needed. */
import * as recast from "recast";
import { parserAdapter } from "./parserAdapter.js";
import { buildComponentTree } from "./tree.js";
import type { CanvasEdit } from "./types.js";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type AnyNode = any;

const b = recast.types.builders;

function isValidIdentifierName(name: string): boolean {
  return /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(name);
}

function propertyKeyName(prop: AnyNode): string {
  return prop.key.type === "Identifier" ? prop.key.name : prop.key.value;
}

function makeKey(name: string): AnyNode {
  return isValidIdentifierName(name) ? b.identifier(name) : b.stringLiteral(name);
}

function findAttribute(element: AnyNode, name: string): AnyNode | undefined {
  return (element.openingElement.attributes as AnyNode[]).find(
    (attr) => attr.type === "JSXAttribute" && attr.name.type === "JSXIdentifier" && attr.name.name === name,
  );
}

function applyStyleChange(element: AnyNode, property: string, value: string): void {
  let styleAttr = findAttribute(element, "style");

  if (!styleAttr) {
    const objectExpr = b.objectExpression([b.objectProperty(makeKey(property), b.stringLiteral(value))]);
    styleAttr = b.jsxAttribute(b.jsxIdentifier("style"), b.jsxExpressionContainer(objectExpr));
    element.openingElement.attributes.push(styleAttr);
    return;
  }

  const container = styleAttr.value;
  if (!container || container.type !== "JSXExpressionContainer" || container.expression.type !== "ObjectExpression") {
    throw new Error(
      `StyleChange targets an element whose "style" attribute isn't a plain object expression this package can safely edit -- refusing to overwrite it rather than silently discarding a variable/expression reference.`,
    );
  }

  const objectExpr = container.expression;
  const existing = (objectExpr.properties as AnyNode[]).find(
    (prop) => prop.type === "ObjectProperty" && !prop.computed && propertyKeyName(prop) === property,
  );
  if (existing) {
    existing.value = b.stringLiteral(value);
  } else {
    objectExpr.properties.push(b.objectProperty(makeKey(property), b.stringLiteral(value)));
  }
}

function applyPropChange(element: AnyNode, prop: string, value: string): void {
  const existing = findAttribute(element, prop);
  if (existing) {
    existing.value = b.stringLiteral(value);
  } else {
    element.openingElement.attributes.push(b.jsxAttribute(b.jsxIdentifier(prop), b.stringLiteral(value)));
  }
}

/** True if `target` is `ancestor` itself or lives anywhere in its real
 * `JSXElement` children subtree -- `Reparent`'s cycle guard walks *down*
 * from the node being moved rather than *up* from the target parent,
 * since every child link here is a real, directly-followable AST array,
 * with no need to consult `parentOf` (or any reverse lookup) at all. */
function isDescendant(ancestor: AnyNode, target: AnyNode): boolean {
  if (ancestor === target) return true;
  for (const child of ancestor.children ?? []) {
    if (child && typeof child === "object" && child.type === "JSXElement" && isDescendant(child, target)) {
      return true;
    }
  }
  return false;
}

function spliceOut(parentChildren: AnyNode[], node: AnyNode): void {
  const idx = parentChildren.indexOf(node);
  if (idx === -1) {
    throw new Error("Internal error: a tracked child was not found in its own tracked parent's children array.");
  }
  parentChildren.splice(idx, 1);
}

/** A real, load-bearing fix found only by running the tests, not by
 * inspection: recast/Babel's own printer decides whether to emit a
 * self-closing tag (`<div />`) purely from `openingElement.selfClosing`
 * -- it does NOT infer that from whether `.children` is non-empty. A
 * previously-childless element (`selfClosing: true`, `closingElement:
 * null`, `.children: []`) silently drops anything pushed into its own
 * real `.children` array at print time unless `selfClosing` is
 * explicitly cleared and a real `closingElement` is given, confirmed by
 * a minimal repro before this fix existed. */
function ensureOpenForChildren(parent: AnyNode): void {
  if (parent.openingElement.selfClosing) {
    parent.openingElement.selfClosing = false;
    parent.closingElement = b.jsxClosingElement(parent.openingElement.name);
  }
}

function spliceIn(parent: AnyNode, node: AnyNode, index: number | undefined): void {
  ensureOpenForChildren(parent);
  const parentChildren = parent.children as AnyNode[];
  if (index === undefined || index >= parentChildren.length) {
    parentChildren.push(node);
  } else {
    parentChildren.splice(Math.max(0, index), 0, node);
  }
}

function applyReparent(
  nodesById: Map<string, AnyNode>,
  parentOf: Map<string, AnyNode | null>,
  edit: Extract<CanvasEdit, { kind: "Reparent" }>,
): void {
  const node = nodesById.get(edit.nodeId);
  if (!node) {
    throw new Error(`No element with id "${edit.nodeId}" found in the current source.`);
  }
  const newParent = nodesById.get(edit.newParentId);
  if (!newParent) {
    throw new Error(`No element with id "${edit.newParentId}" found in the current source.`);
  }
  if (isDescendant(node, newParent)) {
    throw new Error(
      edit.nodeId === edit.newParentId
        ? "Cannot reparent an element to be its own child."
        : "Cannot reparent an element into one of its own descendants -- this would create a cycle.",
    );
  }
  const currentParent = parentOf.get(edit.nodeId);
  if (currentParent === undefined) {
    throw new Error(`Internal error: no parent entry tracked for id "${edit.nodeId}".`);
  }
  if (currentParent === null) {
    throw new Error(
      `Element "${edit.nodeId}" is a top-level component root -- it has no parent JSXElement to detach it from.`,
    );
  }
  spliceOut(currentParent.children as AnyNode[], node);
  spliceIn(newParent, node, edit.index);
}

function applyComponentInsert(nodesById: Map<string, AnyNode>, edit: Extract<CanvasEdit, { kind: "ComponentInsert" }>): void {
  const parent = nodesById.get(edit.parentId);
  if (!parent) {
    throw new Error(`No element with id "${edit.parentId}" found in the current source.`);
  }
  if (!isValidIdentifierName(edit.tagName)) {
    throw new Error(
      `"${edit.tagName}" is not a supported JSX tag name for ComponentInsert -- must be a single valid identifier (member-expression tags like "Foo.Bar" are a real, deliberate v1 scope cut).`,
    );
  }
  const attributes = Object.entries(edit.props ?? {}).map(([name, value]) =>
    b.jsxAttribute(b.jsxIdentifier(name), b.stringLiteral(value)),
  );
  const opening = b.jsxOpeningElement(b.jsxIdentifier(edit.tagName), attributes, true);
  const newElement = b.jsxElement(opening, null, []);
  spliceIn(parent, newElement, edit.index);
}

/** Parses `source`, locates the element(s) `edit` refers to (using the
 * exact same id-assignment traversal `parseComponent` uses, run fresh
 * against this parse -- see tree.ts), mutates the AST in place, and
 * returns the regenerated source. Throws a real, descriptive error
 * (rather than silently no-op'ing) if a referenced id can't be found --
 * e.g. because the source changed structurally since the id was last
 * computed. */
export function applyCanvasEdit(source: string, edit: CanvasEdit): string {
  const ast = recast.parse(source, { parser: parserAdapter });
  const { nodesById, parentOf } = buildComponentTree(ast, source);

  switch (edit.kind) {
    case "StyleChange": {
      const element = nodesById.get(edit.nodeId);
      if (!element) throw new Error(`No element with id "${edit.nodeId}" found in the current source.`);
      applyStyleChange(element, edit.property, edit.value);
      break;
    }
    case "PropChange": {
      const element = nodesById.get(edit.nodeId);
      if (!element) throw new Error(`No element with id "${edit.nodeId}" found in the current source.`);
      applyPropChange(element, edit.prop, edit.value);
      break;
    }
    case "Reparent":
      applyReparent(nodesById, parentOf, edit);
      break;
    case "ComponentInsert":
      applyComponentInsert(nodesById, edit);
      break;
  }

  return recast.print(ast).code;
}
