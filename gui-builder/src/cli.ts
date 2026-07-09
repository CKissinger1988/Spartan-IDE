#!/usr/bin/env node
/**
 * Real §6.2 step 1 dev-server bridge entry point (task #12) -- the actual
 * process `spartan-editor-core`'s Design mode spawns to get a real parsed
 * component tree for the active file. A deliberately minimal v1 contract:
 * one file in, one JSON tree out, no persistent server, no file watching,
 * no HMR -- see gui_bridge.rs's own doc comment (Rust side) and README.md
 * for exactly what this does and does not do.
 *
 * Usage: node dist/cli.js <path-to-jsx-or-tsx-file>
 *
 * On success: prints `{ "roots": ComponentNode[] }` to stdout, exits 0.
 * On failure (missing file, real parse error): prints
 * `{ "error": string }` to stderr, exits 1 -- deliberately not mixed into
 * stdout, so the Rust side never has to distinguish a real payload from a
 * real error by sniffing JSON shape.
 */
import { readFileSync } from "node:fs";
import { parseComponent } from "./parse.js";

function main(): void {
  const path = process.argv[2];
  if (!path) {
    process.stderr.write(JSON.stringify({ error: "usage: cli.js <path-to-jsx-or-tsx-file>" }));
    process.exit(1);
  }

  let source: string;
  try {
    source = readFileSync(path, "utf8");
  } catch (e) {
    process.stderr.write(JSON.stringify({ error: `failed to read ${path}: ${(e as Error).message}` }));
    process.exit(1);
  }

  try {
    const roots = parseComponent(source);
    process.stdout.write(JSON.stringify({ roots }));
  } catch (e) {
    process.stderr.write(JSON.stringify({ error: `failed to parse ${path}: ${(e as Error).message}` }));
    process.exit(1);
  }
}

main();
