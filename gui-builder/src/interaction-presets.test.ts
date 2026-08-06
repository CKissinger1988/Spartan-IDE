import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  INTERACTION_CLIPBOARD_KIND,
  INTERACTION_CLIPBOARD_VERSION,
  buildInteractionClipboard,
  normalizeInteractionPresets,
  parseInteractionClipboard,
} from "./interaction-presets.js";

test("normalizes legacy interaction presets with a normal data state", () => {
  assert.deepEqual(normalizeInteractionPresets([
    { name: "Keyboard focus", state: "focus", updatedAt: 42 },
    { name: "invalid", state: "unknown" },
  ]), [{ name: "Keyboard focus", state: "focus", dataState: "normal", updatedAt: 42 }]);
});

test("keeps valid combined interaction and data states", () => {
  assert.deepEqual(normalizeInteractionPresets([
    { name: "Loading hover", state: "hover", dataState: "loading", updatedAt: 42 },
  ])[0], { name: "Loading hover", state: "hover", dataState: "loading", updatedAt: 42 });
});

test("round-trips a versioned interaction preset clipboard payload", () => {
  const preset = { name: "Error active", state: "active" as const, dataState: "error" as const, updatedAt: 42 };
  const clipboard = buildInteractionClipboard("/project/Card.tsx", preset);
  const raw = JSON.stringify({ kind: INTERACTION_CLIPBOARD_KIND, version: INTERACTION_CLIPBOARD_VERSION, ...clipboard });
  assert.deepEqual(parseInteractionClipboard(raw), { sourcePath: "/project/Card.tsx", ...preset });
});

test("rejects malformed or wrong-kind interaction clipboard payloads", () => {
  assert.equal(parseInteractionClipboard("not json"), null);
  assert.equal(parseInteractionClipboard(JSON.stringify({ kind: "other", version: 1, sourcePath: "/x" })), null);
  assert.equal(parseInteractionClipboard(JSON.stringify({ kind: INTERACTION_CLIPBOARD_KIND, version: 1, sourcePath: "/x", name: "bad", state: "nope" })), null);
});
