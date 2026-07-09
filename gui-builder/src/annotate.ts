/**
 * Real click-to-select support for the live visual canvas (§75.53, task
 * #12) -- injects a `data-spartan-id` attribute onto every real JSX
 * element, carrying the exact same id `tree.ts`'s own canonical
 * traversal already assigns for the structural tree and `edit.ts`'s own
 * `CanvasEdit` targeting. Reuses `buildComponentTree` directly (not a
 * second, separate id-assignment pass) so the live-rendered DOM's ids can
 * never drift from the structural tree's -- a click on a real rendered
 * element and a click on its corresponding text-tree row resolve to the
 * literal same id.
 */
import * as recast from "recast";
import { parserAdapter } from "./parserAdapter.js";
import { buildComponentTree } from "./tree.js";

const b = recast.types.builders;

export function injectNodeIds(source: string): string {
  const ast = recast.parse(source, { parser: parserAdapter });
  const { nodesById } = buildComponentTree(ast, source);
  for (const [id, element] of nodesById) {
    element.openingElement.attributes.push(
      b.jsxAttribute(b.jsxIdentifier("data-spartan-id"), b.stringLiteral(id)),
    );
  }
  return recast.print(ast).code;
}
