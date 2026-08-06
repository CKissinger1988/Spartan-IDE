import assert from "node:assert/strict";
import test from "node:test";
import { buildThemeOverride } from "./theme.js";

test("buildThemeOverride emits a deterministic root token override", () => {
  assert.equal(buildThemeOverride([
    { name: "--brand", value: "#123456" },
    { name: "--space", value: "16px" },
  ]), ":root{--brand:#123456;--space:16px;}");
});

test("buildThemeOverride skips unsafe or malformed token values", () => {
  assert.equal(buildThemeOverride([
    { name: "brand", value: "red" },
    { name: "--bad", value: "red}" },
    { name: "--good", value: "blue" },
  ]), ":root{--good:blue;}");
});
