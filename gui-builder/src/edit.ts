/**
 * Real §6.2 "Canvas -> Code" half (task #12): applies a structured
 * `CanvasEdit` directly onto the real recast/Babel AST (never
 * string-templating) and regenerates source via `recast.print`, which
 * preserves the original formatting/comments/structure of every node this
 * edit doesn't touch -- the real mechanism behind
 * `docs/architecture-spec.md` §6.2's own "preserves formatting, comments,
 * and existing code structure the user wrote by hand" requirement.
 *
 * Deliberately, honestly a v1 subset of the full `CanvasEdit` union named
 * in §6.2: only `StyleChange` and `PropChange` are implemented.
 * `Reparent` and `ComponentInsert` are NOT implemented in this pass --
 * both need this package's id scheme (a pure function of tree structure,
 * recomputed fresh on every parse, see tree.ts's own doc comment) to
 * survive a *structural* edit, which a plain sequential counter cannot do
 * by construction (inserting or moving a node shifts every id after it).
 * A real, stable node-identity scheme (e.g. content-addressed or
 * JSX-source-position-anchored ids) is real, separate future work, not
 * attempted here.
 */
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

/** Parses `source`, locates the element `edit.nodeId` refers to (using
 * the exact same id-assignment traversal `parseComponent` uses, run fresh
 * against this parse -- see tree.ts), mutates it in place, and returns
 * the regenerated source. Throws a real, descriptive error (rather than
 * silently no-op'ing) if the id can't be found -- e.g. because the source
 * changed structurally since the id was last computed. */
export function applyCanvasEdit(source: string, edit: CanvasEdit): string {
  const ast = recast.parse(source, { parser: parserAdapter });
  const { nodesById } = buildComponentTree(ast, source);
  const element = nodesById.get(edit.nodeId);
  if (!element) {
    throw new Error(`No element with id "${edit.nodeId}" found in the current source.`);
  }

  switch (edit.kind) {
    case "StyleChange":
      applyStyleChange(element, edit.property, edit.value);
      break;
    case "PropChange":
      applyPropChange(element, edit.prop, edit.value);
      break;
  }

  return recast.print(ast).code;
}
