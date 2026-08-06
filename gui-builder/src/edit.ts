/**
 * Real §6.2 "Canvas -> Code" half (task #12): applies a structured
 * `CanvasEdit` directly onto the real recast/Babel AST (never
 * string-templating) and regenerates source via `recast.print`, which
 * preserves the original formatting/comments/structure of every node this
 * edit doesn't touch -- the real mechanism behind
 * `docs/architecture-spec.md` §6.2's own "preserves formatting, comments,
 * and existing code structure the user wrote by hand" requirement.
 *
 * All seven members of the `CanvasEdit` union are real and implemented:
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

function propLiteral(value: string, valueType: "string" | "number" | "boolean" = "string"): AnyNode {
  if (valueType === "number") {
    const parsed = Number(value);
    if (!Number.isFinite(parsed)) throw new Error(`PropChange number value "${value}" is not finite.`);
    return b.numericLiteral(parsed);
  }
  if (valueType === "boolean") {
    if (value !== "true" && value !== "false") {
      throw new Error(`PropChange boolean value must be "true" or "false", received "${value}".`);
    }
    return b.booleanLiteral(value === "true");
  }
  return b.stringLiteral(value);
}

function applyPropChange(
  element: AnyNode,
  prop: string,
  value: string,
  valueType: "string" | "number" | "boolean" = "string",
): void {
  const existing = findAttribute(element, prop);
  if (existing) {
    existing.value = valueType === "string" ? b.stringLiteral(value) : b.jsxExpressionContainer(propLiteral(value, valueType));
  } else {
    element.openingElement.attributes.push(
      b.jsxAttribute(
        b.jsxIdentifier(prop),
        valueType === "string" ? b.stringLiteral(value) : b.jsxExpressionContainer(propLiteral(value, valueType)),
      ),
    );
  }
}

function applyTextChange(nodesById: Map<string, AnyNode>, edit: Extract<CanvasEdit, { kind: "TextChange" }>): void {
  const element = nodesById.get(edit.nodeId);
  if (!element) throw new Error(`No element with id "${edit.nodeId}" found in the current source.`);
  const textChildren = (element.children as AnyNode[]).filter((child) => child?.type === "JSXText");
  if (textChildren.length > 1) {
    throw new Error(
      `TextChange found multiple direct text fragments in element "${edit.nodeId}" -- refusing to merge text around nested expressions.`,
    );
  }
  if (textChildren.length === 1) {
    textChildren[0].value = edit.text;
    if (textChildren[0].extra) {
      delete textChildren[0].extra.raw;
      delete textChildren[0].extra.rawValue;
    }
    return;
  }
  ensureOpenForChildren(element);
  (element.children as AnyNode[]).push(b.jsxText(edit.text));
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

function applyDelete(
  nodesById: Map<string, AnyNode>,
  parentOf: Map<string, AnyNode | null>,
  edit: Extract<CanvasEdit, { kind: "Delete" }>,
): void {
  const node = nodesById.get(edit.nodeId);
  if (!node) throw new Error(`No element with id "${edit.nodeId}" found in the current source.`);
  const parent = parentOf.get(edit.nodeId);
  if (parent === undefined) throw new Error(`Internal error: no parent entry tracked for id "${edit.nodeId}".`);
  if (parent === null) {
    throw new Error(
      `Element "${edit.nodeId}" is a top-level component root -- deleting it would remove the component root.`,
    );
  }
  spliceOut(parent.children as AnyNode[], node);
}

function applyDuplicate(
  nodesById: Map<string, AnyNode>,
  parentOf: Map<string, AnyNode | null>,
  edit: Extract<CanvasEdit, { kind: "Duplicate" }>,
): void {
  const node = nodesById.get(edit.nodeId);
  if (!node) throw new Error(`No element with id "${edit.nodeId}" found in the current source.`);
  const parent = parentOf.get(edit.nodeId);
  if (parent === undefined) throw new Error(`Internal error: no parent entry tracked for id "${edit.nodeId}".`);
  if (parent === null) {
    throw new Error(`Element "${edit.nodeId}" is a top-level component root -- duplicate a child element instead.`);
  }
  const siblings = parent.children as AnyNode[];
  const index = siblings.indexOf(node);
  if (index === -1) throw new Error("Internal error: a tracked element was not found in its parent's children.");
  // Babel/Recast AST nodes are plain JSON-shaped data. Cloning the complete
  // subtree keeps attributes, text, nested elements, and comments without
  // sharing mutable child arrays with the original.
  const clone = JSON.parse(JSON.stringify(node)) as AnyNode;
  spliceIn(parent, clone, index + 1);
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
      applyPropChange(element, edit.prop, edit.value, edit.valueType);
      break;
    }
    case "TextChange":
      applyTextChange(nodesById, edit);
      break;
    case "Delete":
      applyDelete(nodesById, parentOf, edit);
      break;
    case "Duplicate":
      applyDuplicate(nodesById, parentOf, edit);
      break;
    case "Reparent":
      applyReparent(nodesById, parentOf, edit);
      break;
    case "ComponentInsert":
      applyComponentInsert(nodesById, edit);
      // Inserting a component that lives in another file is only a real
      // edit if its import comes with it -- otherwise the regenerated
      // source references an undefined binding and the live preview
      // breaks on the very next bundle (task #278).
      if (edit.importFrom) {
        ensureImport(ast, edit.tagName, edit.importFrom, edit.importIsDefault ?? false);
      }
      break;
  }

  return recast.print(ast).code;
}

/**
 * Makes sure `name` is imported from `specifier`, adding as little as
 * possible: nothing at all when the binding is already imported (from
 * anywhere -- re-importing an existing name would be a real syntax
 * error), a new specifier merged into an existing `import ... from
 * "<specifier>"` when one is already present, and only otherwise a whole
 * new import statement at the top of the file.
 *
 * A real, deliberate ordering choice: a brand-new import is inserted
 * *after* any existing leading imports rather than at index 0, so an
 * inserted component's import joins the existing import block instead of
 * jumping above a file's own header comment or first import.
 */
function ensureImport(
  ast: { program: { body: unknown[] } },
  name: string,
  specifier: string,
  isDefault: boolean
): void {
  const body = ast.program.body as Record<string, unknown>[];

  let lastImportIndex = -1;
  for (let i = 0; i < body.length; i++) {
    const node = body[i];
    if (node.type !== "ImportDeclaration") continue;
    lastImportIndex = i;
    const specifiers = (node.specifiers as Record<string, unknown>[]) ?? [];
    // Already imported under this exact name -- from any module. Adding
    // it again would produce a real duplicate-binding syntax error.
    for (const spec of specifiers) {
      const local = spec.local as Record<string, unknown> | undefined;
      if (local && local.name === name) return;
    }
  }

  // Merge into an existing import from the same module, if there is one.
  for (const node of body) {
    if (node.type !== "ImportDeclaration") continue;
    const source = node.source as Record<string, unknown> | undefined;
    if (!source || source.value !== specifier) continue;
    const specifiers = (node.specifiers as Record<string, unknown>[]) ?? [];
    if (isDefault) {
      // A module can only have one default import binding; if this one
      // already has a different default, fall through to a new statement
      // rather than silently replacing the user's own binding.
      if (specifiers.some((s) => s.type === "ImportDefaultSpecifier")) break;
      specifiers.unshift(b.importDefaultSpecifier(b.identifier(name)) as unknown as Record<string, unknown>);
    } else {
      specifiers.push(b.importSpecifier(b.identifier(name)) as unknown as Record<string, unknown>);
    }
    node.specifiers = specifiers;
    return;
  }

  const specifierNode = isDefault
    ? b.importDefaultSpecifier(b.identifier(name))
    : b.importSpecifier(b.identifier(name));
  const decl = b.importDeclaration([specifierNode], b.stringLiteral(specifier));
  body.splice(lastImportIndex + 1, 0, decl as unknown as Record<string, unknown>);
}
