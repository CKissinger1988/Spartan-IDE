import { strict as assert } from "node:assert";
import { test } from "node:test";
import { describeResponsiveDiff } from "./responsive-diff.js";

test("describes geometry and computed-style changes between breakpoints", () => {
  assert.deepEqual(describeResponsiveDiff(
    { rect: { x: 10, y: 20, width: 300, height: 80 }, styles: { display: "flex", fontSize: "16px" } },
    { rect: { x: 10, y: 24, width: 280, height: 96 }, styles: { display: "block", fontSize: "14px" } },
  ), ["y +4px", "width -20px", "height +16px", "display: flex → block", "fontSize: 16px → 14px"]);
});

test("returns no changes for missing or identical inspections", () => {
  const inspection = { rect: { x: 0, y: 0, width: 100, height: 40 }, styles: { display: "block" } };
  assert.deepEqual(describeResponsiveDiff(null, inspection), []);
  assert.deepEqual(describeResponsiveDiff(inspection, inspection), []);
});
