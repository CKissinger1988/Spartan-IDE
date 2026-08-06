import { strict as assert } from "node:assert";
import { test } from "node:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { discoverTokens } from "./tokens.js";

test("discovers real CSS custom properties and preserves their source values", () => {
  const root = mkdtempSync(join(tmpdir(), "spartan-tokens-"));
  mkdirSync(join(root, "styles"), { recursive: true });
  writeFileSync(join(root, "styles/theme.css"), ":root { --brand-red: #d33; --space-md: 16px; }\n");
  const tokens = discoverTokens(root);
  assert.deepEqual(tokens.map((token) => [token.name, token.value]), [["--brand-red", "#d33"], ["--space-md", "16px"]]);
  assert.equal(tokens[0].relativePath, "styles/theme.css");
});

test("discovers repeated declarations as real file-level entries", () => {
  const root = mkdtempSync(join(tmpdir(), "spartan-tokens-"));
  writeFileSync(join(root, "a.css"), ".one { --color: red; } .two { --color: blue; }");
  assert.deepEqual(discoverTokens(root).map((token) => token.value), ["red", "blue"]);
});

test("never walks dependency or build output", () => {
  const root = mkdtempSync(join(tmpdir(), "spartan-tokens-"));
  mkdirSync(join(root, "node_modules/pkg"), { recursive: true });
  mkdirSync(join(root, "dist"), { recursive: true });
  writeFileSync(join(root, "node_modules/pkg/theme.css"), ":root { --bad: red; }");
  writeFileSync(join(root, "dist/theme.css"), ":root { --bad: blue; }");
  writeFileSync(join(root, "theme.css"), ":root { --good: green; }");
  assert.deepEqual(discoverTokens(root).map((token) => token.name), ["--good"]);
});
