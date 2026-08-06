import { strict as assert } from "node:assert";
import { test } from "node:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { applyTokenValue, defineTokenValue, discoverTokens, discoverTokensInSource } from "./tokens.js";

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

test("discovers tokens from an unsaved source buffer with the real project-relative path", () => {
  const root = "/workspace/project";
  const file = "/workspace/project/src/theme.css";
  const tokens = discoverTokensInSource(":root { --live: 18px; }", file, root);
  assert.deepEqual(tokens, [{ name: "--live", value: "18px", file, relativePath: "src/theme.css" }]);
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

test("applies one token value while preserving neighboring CSS source", () => {
  const source = ":root {\n  --brand: #d33;\n  --gap: 8px;\n}\n.button { color: var(--brand); }\n";
  const result = applyTokenValue(source, "--brand", "#246");
  assert.match(result, /--brand: #246;/);
  assert.match(result, /--gap: 8px;/);
  assert.match(result, /color: var\(--brand\)/);
});

test("rejects unsafe or unknown token edits", () => {
  assert.throws(() => applyTokenValue(":root { --x: red; }", "--x", "red; --injected: blue"), /cannot contain/);
  assert.throws(() => applyTokenValue(":root { --x: red; }", "--missing", "blue"), /No declaration/);
});

test("defines a new token in a multiline root while preserving surrounding CSS", () => {
  const source = ":root {\n  --brand: #d33;\n}\n.button { color: var(--brand); }\n";
  const result = defineTokenValue(source, "--space-lg", "24px");
  assert.match(result, /--brand: #d33;/);
  assert.match(result, /  --space-lg: 24px;\n}/);
  assert.match(result, /\.button \{ color: var\(--brand\); \}/);
});

test("creates a root block when a stylesheet has no root token scope", () => {
  const result = defineTokenValue(".button { color: red; }\n", "--surface", "white");
  assert.match(result, /^:root \{\n  --surface: white;\n\}\n\.button/);
});

test("defines existing tokens through the same safe update path", () => {
  assert.equal(defineTokenValue(":root { --x: red; }", "--x", "blue"), ":root { --x: blue; }");
  assert.throws(() => defineTokenValue(":root {}", "x", "red"), /Invalid CSS/);
  assert.throws(() => defineTokenValue(":root {}", "--x", "red; --bad: blue"), /cannot contain/);
});
