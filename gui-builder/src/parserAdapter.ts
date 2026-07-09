/**
 * Real recast <-> @babel/parser adapter (task #12). `recast.parse` needs a
 * parser object exposing a `.parse(source)` method returning a
 * Babel-shaped AST with location info -- this is that adapter, configured
 * for real JSX/TSX source (`docs/architecture-spec.md` §6.2 names
 * Babel/SWC as the intended parser; Babel was chosen here specifically
 * because `recast` -- the tool that actually delivers §6.2's "preserves
 * formatting, comments, and existing code structure" requirement -- is
 * built on top of `ast-types`/Babel-shaped ASTs, not `swc`'s).
 */
import { parse as babelParse } from "@babel/parser";

export const parserAdapter = {
  parse(source: string) {
    return babelParse(source, {
      sourceType: "module",
      allowReturnOutsideFunction: true,
      plugins: ["jsx", "typescript"],
      tokens: true,
    });
  },
};
