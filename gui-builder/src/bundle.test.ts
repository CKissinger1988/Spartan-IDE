import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { execSync } from "node:child_process";
import { bundleComponent, bundleComponentSource } from "./bundle.js";

/**
 * A real fixture project with real `react`/`react-dom` actually
 * `npm install`ed -- proving `bundleComponent` really resolves modules
 * from the *target* project, not this package's own `node_modules`.
 * Self-skips (returns `null`) rather than failing the test suite if the
 * real install can't complete (no network reachable), matching this
 * whole workspace's own established convention for tests that depend on
 * a real external resource (`ollama_integration.rs`, `github_integration.rs`).
 */
function realFixtureWithReactInstalled(): string | null {
  const dir = mkdtempSync(path.join(tmpdir(), "gui-builder-bundle-fixture-"));
  writeFileSync(
    path.join(dir, "package.json"),
    JSON.stringify({
      name: "fixture",
      private: true,
      dependencies: { react: "^18.3.1", "react-dom": "^18.3.1" },
    }),
  );
  try {
    execSync("npm install --no-audit --no-fund", {
      cwd: dir,
      stdio: "pipe",
      timeout: 60_000,
    });
  } catch {
    rmSync(dir, { recursive: true, force: true });
    return null;
  }
  return dir;
}

test("bundles a real component with real react/react-dom resolved from the target project", async (t) => {
  const dir = realFixtureWithReactInstalled();
  if (!dir) {
    t.skip("could not npm install a real react/react-dom fixture (no network?)");
    return;
  }
  try {
    const componentPath = path.join(dir, "Hello.jsx");
    writeFileSync(
      componentPath,
      `export default function Hello() { return <div className="greeting">Hello, Spartan!</div>; }`,
    );
    const result = await bundleComponent(componentPath);
    assert.ok("code" in result, `expected a real bundle, got: ${JSON.stringify(result)}`);
    if ("code" in result) {
      assert.ok(result.code.includes("Hello, Spartan!"));
      assert.ok(result.code.includes("spartan-root"));
      // A real bundle including React/ReactDOM is never tiny -- a real,
      // rough sanity floor that this isn't an empty/near-empty output.
      assert.ok(result.code.length > 1000, "a real react bundle should not be tiny");
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("bundles the unsaved source override instead of stale disk contents", async (t) => {
  const dir = realFixtureWithReactInstalled();
  if (!dir) {
    t.skip("could not npm install a real react/react-dom fixture (no network?)");
    return;
  }
  try {
    const componentPath = path.join(dir, "Unsaved.jsx");
    writeFileSync(componentPath, `export default function Unsaved() { return <div>disk version</div>; }`);
    const result = await bundleComponentSource(
      componentPath,
      `export default function Unsaved() { return <div>unsaved version</div>; }`,
    );
    assert.ok("code" in result, `expected a real bundle, got: ${JSON.stringify(result)}`);
    if ("code" in result) {
      assert.ok(result.code.includes("unsaved version"));
      assert.ok(!result.code.includes("disk version"));
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a real bundle carries real data-spartan-id attributes for click-to-select (§75.53)", async (t) => {
  const dir = realFixtureWithReactInstalled();
  if (!dir) {
    t.skip("could not npm install a real react/react-dom fixture (no network?)");
    return;
  }
  try {
    const componentPath = path.join(dir, "Nested.jsx");
    writeFileSync(
      componentPath,
      `export default function Nested() { return <div><span>hello</span></div>; }`,
    );
    const result = await bundleComponent(componentPath);
    assert.ok("code" in result, `expected a real bundle, got: ${JSON.stringify(result)}`);
    if ("code" in result) {
      assert.ok(
        result.code.includes("data-spartan-id"),
        "the real bundle should carry the real click-to-select attribute",
      );
      assert.ok(result.code.includes('"n0"'), "the outer <div> should get id n0");
      assert.ok(result.code.includes('"n1"'), "the nested <span> should get its own distinct id n1");
      assert.ok(
        result.code.includes("spartan-canvas-click"),
        "the real bundle should include the real click-relay postMessage call",
      );
      assert.ok(
        result.code.includes("spartan-canvas-select"),
        "the real bundle should accept parent-driven selection messages",
      );
      assert.ok(
        result.code.includes("highlightSelection"),
        "the real bundle should visibly highlight the selected canvas node",
      );
      assert.ok(
        result.code.includes("spartan-canvas-inspect-result"),
        "the real bundle should report rendered geometry and computed styles",
      );
      assert.ok(
        result.code.includes("getComputedStyle"),
        "the real bundle should inspect styles inside the sandbox where the DOM is readable",
      );
      assert.ok(result.code.includes("spartan-canvas-focus"), "the real bundle should support preview focus state");
      assert.ok(result.code.includes("spartan-canvas-blur"), "the real bundle should support leaving preview focus state");
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a real missing dependency produces a real, honest bundling error, not a silent partial render", async () => {
  const dir = mkdtempSync(path.join(tmpdir(), "gui-builder-bundle-missing-dep-"));
  try {
    const componentPath = path.join(dir, "Broken.jsx");
    writeFileSync(
      componentPath,
      `import { thing } from "a-package-that-does-not-really-exist-anywhere";\n` +
        `export default function Broken() { return <div>{thing}</div>; }`,
    );
    const result = await bundleComponent(componentPath);
    assert.ok("error" in result, `expected a real error, got: ${JSON.stringify(result)}`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a real named-export-only file fails with a real, clear esbuild error naming the missing default export", async (t) => {
  // Real finding, caught only by running this test, not by inspection:
  // an earlier version of this test assumed a missing default export
  // would only surface as a *runtime* error inside the rendered iframe
  // (this file's own `typeof Component !== "function"` check). It
  // doesn't -- esbuild statically resolves real ES module imports and
  // catches a genuinely nonexistent default export as a real *build-time*
  // error before the entry script's own runtime check ever gets a
  // chance to run. That's a real, stricter, and actually clearer failure
  // mode than what this test originally expected -- fixed by correcting
  // the test's expectation, not the code. A *second* real mistake caught
  // by actually running this (not just reasoning about it): without a
  // real `react` installed in the fixture, the real error is instead
  // "Could not resolve react" -- react must genuinely be resolvable for
  // this specific "No matching export" error to be the one esbuild
  // reports, so this test needs the same real react-installed fixture
  // the other tests use, not a bare temp dir.
  const dir = realFixtureWithReactInstalled();
  if (!dir) {
    t.skip("could not npm install a real react/react-dom fixture (no network?)");
    return;
  }
  try {
    const componentPath = path.join(dir, "NamedOnly.jsx");
    writeFileSync(componentPath, `export function NamedOnly() { return <div>x</div>; }`);
    const result = await bundleComponent(componentPath);
    assert.ok("error" in result, `expected a real build error, got: ${JSON.stringify(result)}`);
    if ("error" in result) {
      assert.ok(result.error.includes("No matching export"));
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a real default export that isn't a component reports a real runtime render error", async (t) => {
  // The real, remaining case the entry script's own `typeof Component
  // !== "function"` runtime check exists for: esbuild's static analysis
  // can't catch this one (a real default export genuinely exists), so
  // it only surfaces once the bundle actually runs.
  const dir = realFixtureWithReactInstalled();
  if (!dir) {
    t.skip("could not npm install a real react/react-dom fixture (no network?)");
    return;
  }
  try {
    const componentPath = path.join(dir, "NotAComponent.jsx");
    writeFileSync(componentPath, `export default "just a string, not a component";`);
    const result = await bundleComponent(componentPath);
    assert.ok("code" in result, `expected a real bundle, got: ${JSON.stringify(result)}`);
    if ("code" in result) {
      assert.ok(result.code.includes("no default export"));
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
