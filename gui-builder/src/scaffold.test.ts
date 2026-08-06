import assert from "node:assert/strict";
import test from "node:test";
import { buildComponentScaffold } from "./scaffold.js";

test("buildComponentScaffold emits typed props, enum values, and a default variant", () => {
  const source = buildComponentScaffold({
    componentName: "StatusCard",
    defaultVariant: "compact",
    props: [
      { name: "title", kind: "string" },
      { name: "count", kind: "number" },
      { name: "active", kind: "boolean" },
      { name: "tone", kind: "enum", enumValues: ["primary", "secondary"] },
      { name: "children", kind: "slot" },
    ],
  });
  assert.match(source, /export interface StatusCardProps/);
  assert.match(source, /count: number;/);
  assert.match(source, /tone: "primary" \| "secondary";/);
  assert.match(source, /data-variant="compact"/);
  assert.match(source, /\{children\}/);
});

test("buildComponentScaffold rejects invalid and duplicate schema entries", () => {
  assert.throws(() => buildComponentScaffold({ componentName: "123Card", props: [] }), /valid TypeScript identifier/);
  assert.throws(() => buildComponentScaffold({
    componentName: "Card",
    props: [{ name: "title", kind: "string" }, { name: "title", kind: "number" }],
  }), /Duplicate prop/);
  assert.throws(() => buildComponentScaffold({
    componentName: "Card",
    props: [{ name: "tone", kind: "enum", enumValues: ["only"] }],
  }), /at least two values/);
});
