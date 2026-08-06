import { strict as assert } from "node:assert";
import { test } from "node:test";
import { describeToken } from "./token-model.js";

test("classifies explicit primitive, semantic, and component tiers", () => {
  assert.deepEqual(describeToken("--gray-900", "#111"), { tier: "primitive", references: [] });
  assert.deepEqual(describeToken("--semantic-text", "var(--gray-900)"), { tier: "semantic", references: ["--gray-900"] });
  assert.deepEqual(describeToken("--component-button-bg", "var(--semantic-accent)"), { tier: "component", references: ["--semantic-accent"] });
});

test("deduplicates repeated aliases while preserving reference order", () => {
  assert.deepEqual(describeToken("--theme-card", "var(--surface) var(--surface) var(--space)"), {
    tier: "semantic",
    references: ["--surface", "--space"],
  });
});
