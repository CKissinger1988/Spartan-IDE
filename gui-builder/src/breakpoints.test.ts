import { strict as assert } from "node:assert";
import { test } from "node:test";
import { buildPreviewBreakpoints, normalizeResponsiveBreakpoints } from "./breakpoints.js";

test("normalizes saved breakpoints and rejects unsafe dimensions", () => {
  assert.deepEqual(normalizeResponsiveBreakpoints([
    { name: "  Laptop  ", width: 1366.4, height: 768.2 },
    { name: "Laptop", width: 1200, height: 800 },
    { name: "Too small", width: 199, height: 600 },
    { name: "Too large", width: 3200, height: 800 },
    { name: "Missing", width: 800 },
  ]), [{ name: "Laptop", width: 1366, height: 768 }]);
});

test("normalizes malformed storage values to an empty profile list", () => {
  assert.deepEqual(normalizeResponsiveBreakpoints(null), []);
  assert.deepEqual(normalizeResponsiveBreakpoints({ name: "Phone" }), []);
});

test("builds deterministic matrix entries for defaults and custom profiles", () => {
  assert.deepEqual(buildPreviewBreakpoints(
    [{ name: "Desktop", width: 1280, height: 800 }],
    [{ name: "Tablet wide", width: 1024, height: 800 }],
  ), [
    { name: "Desktop", width: 1280, height: 800, id: "desktop", label: "Desktop" },
    { name: "Tablet wide", width: 1024, height: 800, id: "custom-0-tablet-wide", label: "Tablet wide" },
  ]);
});
