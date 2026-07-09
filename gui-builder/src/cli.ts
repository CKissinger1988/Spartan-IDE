#!/usr/bin/env node
/**
 * Real §6.2 dev-server bridge entry point (task #12) -- the actual process
 * `spartan-editor-core`'s Design mode spawns for both real sync directions.
 * A deliberately minimal v1 contract: one file/edit in, one JSON result
 * out, no persistent server, no file watching, no HMR -- see gui_bridge.rs's
 * own doc comment (Rust side) and README.md for exactly what this does and
 * does not do.
 *
 * Usage:
 *   node dist/cli.js <path-to-jsx-or-tsx-file>       (Code -> Canvas, §75.41)
 *   node dist/cli.js apply <editJson>                (Canvas -> Code, §75.42)
 *
 * "parse" mode reads the file at `<path>` from disk (§6.2 step 1 -- there is
 * no live buffer on this side, so it always reflects what's actually on
 * disk, which may lag an unsaved in-editor edit). "apply" mode
 * deliberately reads its source from **stdin** instead of from disk, so the
 * real Rust caller can feed it the live, possibly-unsaved in-memory buffer
 * -- editing through this CLI must never silently discard or race against
 * unsaved keystrokes the user already made in the real editor.
 *
 * On success: "parse" prints `{ "roots": ComponentNode[] }`; "apply" prints
 * `{ "source": string }` (the real regenerated file source, not written to
 * any file -- the Rust caller owns feeding it into the live `Document`).
 * Both exit 0.
 *
 * On failure (missing file, invalid edit JSON, unknown node id, a real
 * parse/edit error): prints `{ "error": string }` to stderr, exits 1 --
 * deliberately not mixed into stdout, so the Rust side never has to
 * distinguish a real payload from a real error by sniffing JSON shape.
 */
import { readFileSync } from "node:fs";
import { parseComponent } from "./parse.js";
import { applyCanvasEdit } from "./edit.js";
import type { CanvasEdit } from "./types.js";

function fail(message: string): never {
  process.stderr.write(JSON.stringify({ error: message }));
  process.exit(1);
}

function readStdin(): string {
  return readFileSync(0, "utf8");
}

function runParse(path: string | undefined): void {
  if (!path) {
    fail("usage: cli.js <path-to-jsx-or-tsx-file>");
  }

  let source: string;
  try {
    source = readFileSync(path, "utf8");
  } catch (e) {
    fail(`failed to read ${path}: ${(e as Error).message}`);
  }

  try {
    const roots = parseComponent(source);
    process.stdout.write(JSON.stringify({ roots }));
  } catch (e) {
    fail(`failed to parse ${path}: ${(e as Error).message}`);
  }
}

function runApply(editJson: string | undefined): void {
  if (!editJson) {
    fail("usage: cli.js apply <editJson> (source read from stdin)");
  }

  let edit: CanvasEdit;
  try {
    edit = JSON.parse(editJson) as CanvasEdit;
  } catch (e) {
    fail(`invalid edit JSON: ${(e as Error).message}`);
  }

  let source: string;
  try {
    source = readStdin();
  } catch (e) {
    fail(`failed to read stdin: ${(e as Error).message}`);
  }

  try {
    const newSource = applyCanvasEdit(source, edit);
    process.stdout.write(JSON.stringify({ source: newSource }));
  } catch (e) {
    fail(`failed to apply edit: ${(e as Error).message}`);
  }
}

function main(): void {
  const mode = process.argv[2];
  if (mode === "apply") {
    runApply(process.argv[3]);
  } else {
    runParse(mode);
  }
}

main();
