import assert from "node:assert/strict";
import test from "node:test";
import { LAYOUT_PRESETS } from "./layout.js";

test("layout presets provide real flex and grid recipes", () => {
  assert.deepEqual(LAYOUT_PRESETS.map((preset) => preset.id), ["stack", "row", "grid", "center"]);
  assert.equal(LAYOUT_PRESETS.find((preset) => preset.id === "stack")?.entries.flexDirection, "column");
  assert.equal(LAYOUT_PRESETS.find((preset) => preset.id === "row")?.entries.justifyContent, undefined);
  assert.equal(LAYOUT_PRESETS.find((preset) => preset.id === "grid")?.entries.gridTemplateColumns, "repeat(2, minmax(0, 1fr))");
  assert.equal(LAYOUT_PRESETS.find((preset) => preset.id === "center")?.entries.justifyContent, "center");
});
