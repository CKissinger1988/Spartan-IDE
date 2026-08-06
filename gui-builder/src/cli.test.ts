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

test("real subprocess: CLI asset-source returns sanitized SVG markup", () => {
  const dir = mkdtempSync(path.join(tmpdir(), "gui-builder-cli-svg-test-"));
  const file = path.join(dir, "icon.svg");
  writeFileSync(file, `<svg onclick="bad()"><script>bad()</script><path d="M0 0" /></svg>`);
  const { stdout, status } = runCli(["asset-source", file]);
  rmSync(dir, { recursive: true, force: true });
  assert.equal(status, 0);
  assert.match(JSON.parse(stdout).source, /<path d="M0 0" \/>/);
  assert.doesNotMatch(JSON.parse(stdout).source, /script|onclick/i);
});

test("real subprocess: CLI usage discovery reads source overrides from stdin", () => {
  const dir = mkdtempSync(path.join(tmpdir(), "gui-builder-cli-usage-test-"));
  const app = path.join(dir, "App.tsx");
  const asset = path.join(dir, "logo.svg");
  const stylesheet = path.join(dir, "theme.css");
  writeFileSync(app, "const view = 'var(--accent)';\nconst logo = './logo.svg';\n");
  writeFileSync(asset, "<svg />");
  writeFileSync(stylesheet, ":root { --accent: red; } .logo { background: url('./logo.svg'); }");
  const overrides = JSON.stringify({ [app]: "const view = 'live';\n" });
  const tokens = runCli(["tokens", dir, "--source-overrides"], overrides);
  const assets = runCli(["assets", dir, "", "--source-overrides"], overrides);
  rmSync(dir, { recursive: true, force: true });

  assert.equal(tokens.status, 0);
  assert.equal(JSON.parse(tokens.stdout).tokens[0].usageCount, 0);
  assert.equal(assets.status, 0);
  assert.equal(JSON.parse(assets.stdout).assets.find((item: { file: string }) => item.file === asset).usageCount, 1);
});

test("real subprocess: component discovery reads source overrides from stdin", () => {
  const dir = mkdtempSync(path.join(tmpdir(), "gui-builder-cli-components-test-"));
  const app = path.join(dir, "App.jsx");
  const button = path.join(dir, "Button.jsx");
  writeFileSync(app, "import { Button } from './Button';\nexport default function App() { return <Button />; }\n");
  writeFileSync(button, "export function Button() { return <button />; }\n");
  const result = runCli(["components", dir, app, "--source-overrides"], JSON.stringify({
    [app]: "export default function App() { return <main />; }\n",
  }));
  rmSync(dir, { recursive: true, force: true });

  assert.equal(result.status, 0);
  const discovered = JSON.parse(result.stdout).components as Array<{ name: string; usageCount?: number }>;
  assert.equal(discovered.find((component) => component.name === "Button")?.usageCount, 0);
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
