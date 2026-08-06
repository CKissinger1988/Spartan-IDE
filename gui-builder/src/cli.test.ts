import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const cliPath = path.join(here, "cli.ts");

function runCli(args: string[], input?: string): { stdout: string; status: number } {
  try {
    const stdout = execFileSync("node", ["--import", "tsx", cliPath, ...args], {
      encoding: "utf8",
      input: input ?? "",
    });
    return { stdout, status: 0 };
  } catch (e) {
    const err = e as { stdout?: Buffer | string; status?: number };
    return { stdout: err.stdout?.toString() ?? "", status: err.status ?? 1 };
  }
}

test("real subprocess: CLI parses a real file and prints a real ComponentNode tree as JSON", () => {
  const dir = mkdtempSync(path.join(tmpdir(), "gui-builder-cli-test-"));
  const file = path.join(dir, "App.jsx");
  writeFileSync(file, `const X = () => <div className="app">Hello</div>;`);

  const { stdout, status } = runCli([file]);
  rmSync(dir, { recursive: true, force: true });

  assert.equal(status, 0);
  const parsed = JSON.parse(stdout);
  assert.equal(parsed.roots.length, 1);
  assert.equal(parsed.roots[0].tagName, "div");
  assert.deepEqual(parsed.roots[0].props.className, { kind: "string", value: "app" });
});

test("real subprocess: CLI parse-source reads the live source from stdin", () => {
  const dir = mkdtempSync(path.join(tmpdir(), "gui-builder-cli-source-test-"));
  const file = path.join(dir, "App.jsx");
  const { stdout, status } = runCli(["parse-source", file], `const X = () => <button>unsaved</button>;`);
  rmSync(dir, { recursive: true, force: true });
  assert.equal(status, 0);
  assert.equal(JSON.parse(stdout).roots[0].tagName, "button");
});

test("real subprocess: CLI reports a real error (non-zero exit) for a missing file", () => {
  const { status } = runCli(["/nonexistent/path/does/not/exist.jsx"]);
  assert.equal(status, 1);
});

test("real subprocess: CLI reports a real error (non-zero exit) with no arguments", () => {
  const { status } = runCli([]);
  assert.equal(status, 1);
});

test("real subprocess: CLI apply mode reads source from stdin and returns real regenerated source", () => {
  const source = `const X = () => <div className="app">Hello</div>;`;
  const edit = JSON.stringify({ kind: "PropChange", nodeId: "n0", prop: "className", value: "updated" });

  const { stdout, status } = runCli(["apply", edit], source);

  assert.equal(status, 0);
  const parsed = JSON.parse(stdout);
  assert.match(parsed.source, /className="updated"/);
  // recast preserves everything untouched -- the arrow function and
  // semicolon survive verbatim, real proof this isn't string templating.
  assert.match(parsed.source, /const X = \(\) =>/);
});

test("real subprocess: CLI apply mode reports a real error for an unknown node id", () => {
  const source = `const X = () => <div>Hello</div>;`;
  const edit = JSON.stringify({ kind: "PropChange", nodeId: "does-not-exist", prop: "id", value: "x" });

  const { status } = runCli(["apply", edit], source);
  assert.equal(status, 1);
});

test("real subprocess: CLI apply mode reports a real error for invalid edit JSON", () => {
  const { status } = runCli(["apply", "{not valid json"], "const X = () => <div/>;");
  assert.equal(status, 1);
});
