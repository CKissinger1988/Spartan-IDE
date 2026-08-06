import { strict as assert } from "node:assert";
import { test } from "node:test";
import { suggestAccessibilityFixes } from "./accessibility-fixes.js";

test("suggests a deterministic decorative-image fix", () => {
  const fixes = suggestAccessibilityFixes({ nodeId: "n1", tagName: "img", props: { alt: null } });
  assert.deepEqual(fixes.map((fix) => fix.edit), [{ kind: "PropChange", nodeId: "n1", prop: "alt", value: "", valueType: "string" }]);
});

test("suggests keyboard focus and target-size fixes for a custom role", () => {
  const fixes = suggestAccessibilityFixes({
    nodeId: "n2",
    tagName: "div",
    props: { role: "button", tabIndex: "-1" },
    inspection: { width: 30, height: 20 },
  });
  assert.deepEqual(fixes.map((fix) => fix.edit), [
    { kind: "PropChange", nodeId: "n2", prop: "tabIndex", value: "0", valueType: "number" },
    { kind: "StyleChange", nodeId: "n2", property: "minWidth", value: "44px" },
    { kind: "StyleChange", nodeId: "n2", property: "minHeight", value: "44px" },
  ]);
});

test("does not invent an accessible label or contrast correction", () => {
  assert.deepEqual(suggestAccessibilityFixes({
    nodeId: "n3", tagName: "button", props: { "aria-label": null }, inspection: { width: 50, height: 50 },
  }), []);
});
