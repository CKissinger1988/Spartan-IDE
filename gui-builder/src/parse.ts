/**
 * Real, public §6.2 "Code -> Canvas" entry point (task #12).
 */
import * as recast from "recast";
import { parserAdapter } from "./parserAdapter.js";
import { buildComponentTree } from "./tree.js";
import type { ComponentNode } from "./types.js";

/** Parses real JSX/TSX source into the component tree(s) a real canvas UI
 * would render. Returns one entry per top-level JSX root found in the
 * file (see tree.ts's own doc comment for what "top-level" means here). */
export function parseComponent(source: string): ComponentNode[] {
  const ast = recast.parse(source, { parser: parserAdapter });
  return buildComponentTree(ast, source).roots;
}
