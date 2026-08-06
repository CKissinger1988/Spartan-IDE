import assert from "node:assert/strict";
import test from "node:test";
import { designClipboardShortcut } from "./shortcuts.js";

test("designClipboardShortcut resolves the explicit subtree copy chord", () => {
  assert.equal(designClipboardShortcut("B", true, false, true), "copySubtree");
});

test("designClipboardShortcut resolves the explicit subtree paste chord", () => {
  assert.equal(designClipboardShortcut("p", false, false, true), null);
  assert.equal(designClipboardShortcut("p", true, false, true), "pasteSubtree");
});

test("designClipboardShortcut leaves established clipboard chords untouched", () => {
  assert.equal(designClipboardShortcut("b", true, true, true), null);
  assert.equal(designClipboardShortcut("p", true, true, true), null);
  assert.equal(designClipboardShortcut("x", true, false, true), null);
});
