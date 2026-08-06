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
 *   node dist/cli.js bundle <path-to-jsx-or-tsx-file> (live visual render, §75.52)
 *   node dist/cli.js parse-source <path-to-jsx-or-tsx-file> (source from stdin)
 *   node dist/cli.js bundle-source <path-to-jsx-or-tsx-file> (source from stdin)
 *   node dist/cli.js components <project-dir> [from-file] (component browser, task #278)
 *   node dist/cli.js assets <project-dir> [from-file] (image asset browser)
 *   node dist/cli.js tokens <project-dir> (CSS custom-property browser)
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
import { bundleComponent } from "./bundle.js";
import { bundleComponentSource } from "./bundle.js";
import { discoverComponents } from "./components.js";
import { discoverAssets } from "./assets.js";
import { applyTokenValue, defineTokenValue, discoverTokens, discoverTokensInSource } from "./tokens.js";
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

async function runBundle(path: string | undefined): Promise<void> {
  if (!path) {
    fail("usage: cli.js bundle <path-to-jsx-or-tsx-file>");
  }
  const result = await bundleComponent(path);
  if ("error" in result) {
    fail(result.error);
  }
  process.stdout.write(JSON.stringify({ code: result.code }));
}

async function runBundleSource(path: string | undefined): Promise<void> {
  if (!path) fail("usage: cli.js bundle-source <path-to-jsx-or-tsx-file> (source read from stdin)");
  try {
    const result = await bundleComponentSource(path, readStdin());
    if ("error" in result) fail(result.error);
    process.stdout.write(JSON.stringify({ code: result.code }));
  } catch (e) {
    fail(`failed to bundle ${path}: ${(e as Error).message}`);
  }
}

function runParseSource(path: string | undefined): void {
  if (!path) fail("usage: cli.js parse-source <path-to-jsx-or-tsx-file> (source read from stdin)");
  try {
    const roots = parseComponent(readStdin());
    process.stdout.write(JSON.stringify({ roots }));
  } catch (e) {
    fail(`failed to parse ${path}: ${(e as Error).message}`);
  }
}

/** Real component-library discovery (task #278). `fromFile` is optional
 * but strongly wanted by a real caller: with it, each result carries the
 * relative module specifier an import in that file would actually need. */
function runComponents(rootDir: string | undefined, fromFile: string | undefined): void {
  if (!rootDir) {
    fail("usage: cli.js components <project-dir> [from-file]");
  }
  try {
    const components = discoverComponents(rootDir, fromFile);
    process.stdout.write(JSON.stringify({ components }));
  } catch (e) {
    fail(`failed to discover components: ${(e as Error).message}`);
  }
}

function runAssets(rootDir: string | undefined, fromFile: string | undefined): void {
  if (!rootDir) {
    fail("usage: cli.js assets <project-dir> [from-file]");
  }
  try {
    const assets = discoverAssets(rootDir, fromFile);
    process.stdout.write(JSON.stringify({ assets }));
  } catch (e) {
    fail(`failed to discover assets: ${(e as Error).message}`);
  }
}

function runTokens(rootDir: string | undefined): void {
  if (!rootDir) fail("usage: cli.js tokens <project-dir>");
  try {
    process.stdout.write(JSON.stringify({ tokens: discoverTokens(rootDir) }));
  } catch (e) {
    fail(`failed to discover tokens: ${(e as Error).message}`);
  }
}

function runTokenSource(path: string | undefined, rootDir: string | undefined): void {
  if (!path || !rootDir) fail("usage: cli.js token-source <css-file> <project-dir> (source read from stdin)");
  try {
    process.stdout.write(JSON.stringify({ tokens: discoverTokensInSource(readStdin(), path, rootDir) }));
  } catch (e) {
    fail(`failed to discover tokens from ${path}: ${(e as Error).message}`);
  }
}

function runTokenApply(path: string | undefined, name: string | undefined, value: string | undefined): void {
  if (!path || !name || value === undefined) fail("usage: cli.js token-apply <css-file> <token-name> <value> (source read from stdin)");
  try {
    process.stdout.write(JSON.stringify({ source: applyTokenValue(readStdin(), name, value) }));
  } catch (e) {
    fail(`failed to apply token: ${(e as Error).message}`);
  }
}

function runTokenDefine(path: string | undefined, name: string | undefined, value: string | undefined): void {
  if (!path || !name || value === undefined) fail("usage: cli.js token-define <css-file> <token-name> <value> (source read from stdin)");
  try {
    process.stdout.write(JSON.stringify({ source: defineTokenValue(readStdin(), name, value) }));
  } catch (e) {
    fail(`failed to define token: ${(e as Error).message}`);
  }
}

async function main(): Promise<void> {
  const mode = process.argv[2];
  if (mode === "apply") {
    runApply(process.argv[3]);
  } else if (mode === "bundle") {
    await runBundle(process.argv[3]);
  } else if (mode === "bundle-source") {
    await runBundleSource(process.argv[3]);
  } else if (mode === "parse-source") {
    runParseSource(process.argv[3]);
  } else if (mode === "components") {
    runComponents(process.argv[3], process.argv[4]);
  } else if (mode === "assets") {
    runAssets(process.argv[3], process.argv[4]);
  } else if (mode === "tokens") {
    runTokens(process.argv[3]);
  } else if (mode === "token-source") {
    runTokenSource(process.argv[3], process.argv[4]);
  } else if (mode === "token-apply") {
    runTokenApply(process.argv[3], process.argv[4], process.argv[5]);
  } else if (mode === "token-define") {
    runTokenDefine(process.argv[3], process.argv[4], process.argv[5]);
  } else {
    runParse(mode);
  }
}

main();
