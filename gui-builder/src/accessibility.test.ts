import assert from "node:assert/strict";
import test from "node:test";
import { screenReaderPreview } from "./accessibility.js";

const node = (tagName: string, props: Record<string, { kind: "string"; value: string }> = {}, textContent: string | null = null) => ({ tagName, props, textContent });

test("screenReaderPreview infers a named button and disabled state", () => {
  const result = screenReaderPreview(node("button", { disabled: { kind: "string", value: "true" } }, "Save"));
  assert.equal(result.announcement, "button, Save, disabled true");
  assert.deepEqual(result.details, ["name “Save”", "disabled true"]);
});

test("screenReaderPreview uses the input type role and aria label", () => {
  const result = screenReaderPreview(node("input", {
    type: { kind: "string", value: "checkbox" },
    "aria-label": { kind: "string", value: "Receive updates" },
    "aria-checked": { kind: "string", value: "true" },
  }));
  assert.equal(result.announcement, "checkbox, Receive updates, checked true");
});

test("screenReaderPreview reports heading level and labelled-by references", () => {
  const result = screenReaderPreview(node("h2", { "aria-labelledby": { kind: "string", value: "heading-label" } }));
  assert.equal(result.announcement, "heading, labelled by heading-label");
  assert.deepEqual(result.details, ["level 2", "name “labelled by heading-label”"]);
});

test("screenReaderPreview marks aria-hidden nodes as unavailable", () => {
  const result = screenReaderPreview(node("div", { "aria-hidden": { kind: "string", value: "true" } }, "Hidden"));
  assert.equal(result.announcement, "Hidden from assistive technology");
  assert.deepEqual(result.details, ["aria-hidden=true"]);
});

test("screenReaderPreview leaves decorative images unnamed", () => {
  const result = screenReaderPreview(node("img", { alt: { kind: "string", value: "" } }));
  assert.equal(result.announcement, "img");
  assert.deepEqual(result.details, []);
});
