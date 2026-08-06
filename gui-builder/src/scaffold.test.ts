import assert from "node:assert/strict";
import test from "node:test";
import { buildComponentPlaygroundScaffold, buildComponentScaffold } from "./scaffold.js";
import { parseComponent } from "./parse.js";

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

test("buildComponentPlaygroundScaffold emits controlled typed prop inputs", () => {
  const source = buildComponentPlaygroundScaffold({
    componentName: "StatusCard",
    componentImportPath: "./StatusCard",
    props: [
      { name: "title", kind: "string" },
      { name: "count", kind: "number" },
      { name: "active", kind: "boolean" },
      { name: "tone", kind: "enum", enumValues: ["primary", "secondary"] },
      { name: "children", kind: "slot" },
    ],
  });
  assert.match(source, /import StatusCard from "\.\/StatusCard"/);
  assert.match(source, /type="number"/);
  assert.match(source, /type="checkbox"/);
  assert.match(source, /<option value="primary">primary<\/option>/);
  assert.match(source, /children=\{children\}/);
  assert.doesNotThrow(() => parseComponent(source));
});

test("buildComponentPlaygroundScaffold rejects unsafe import paths", () => {
  assert.throws(() => buildComponentPlaygroundScaffold({ componentName: "Card", componentImportPath: "Card\";alert(1)", props: [] }), /relative path/);
});
