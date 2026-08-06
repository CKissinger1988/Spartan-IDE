import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { parseComponent } from "./parse.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const prototypesDir = path.join(here, "..", "..", "prototypes");

test("parses a simple element's tag, string prop, and text content", () => {
  const [root] = parseComponent(`const X = () => <button className="btn">Click me</button>;`);
  assert.equal(root.tagName, "button");
  assert.deepEqual(root.props.className, { kind: "string", value: "btn" });
  assert.equal(root.textContent, "Click me");
  assert.equal(root.children.length, 0);
  assert.deepEqual(root.sourceLocation, { startLine: 1, startColumn: 16, endLine: 1, endColumn: 57 });
});

test("nests children in document order with distinct, stable ids", () => {
  const [root] = parseComponent(`const X = () => (
    <div>
      <span>a</span>
      <span>b</span>
    </div>
  );`);
  assert.equal(root.children.length, 2);
  assert.equal(root.children[0].textContent, "a");
  assert.equal(root.children[1].textContent, "b");
  assert.notEqual(root.id, root.children[0].id);
  assert.notEqual(root.children[0].id, root.children[1].id);
});

test("recognizes a plain style object as a style PropSummary", () => {
  const [root] = parseComponent(`const X = () => <div style={{ color: "red", padding: 4 }} />;`);
  assert.deepEqual(root.props.style, {
    kind: "style",
    entries: {
      color: { kind: "literal", value: "red" },
      padding: { kind: "literal", value: "4" },
    },
  });
});

test("a style object mixing a literal and a design-token expression summarizes each key on its own terms", () => {
  const [root] = parseComponent(`const X = () => <div style={{ fontSize: 12, color: C.text }} />;`);
  assert.deepEqual(root.props.style, {
    kind: "style",
    entries: {
      fontSize: { kind: "literal", value: "12" },
      color: { kind: "expression", source: "C.text" },
    },
  });
});

test("a style object containing a spread falls back to a whole-prop expression", () => {
  const [root] = parseComponent(`const X = () => <div style={{ ...base, color: "red" }} />;`);
  assert.equal(root.props.style.kind, "expression");
});

test("records a non-literal prop as an expression, verbatim", () => {
  const [root] = parseComponent(`const X = () => <button onClick={() => doThing(count)} />;`);
  assert.equal(root.props.onClick.kind, "expression");
  assert.equal((root.props.onClick as { kind: "expression"; source: string }).source, "() => doThing(count)");
});

test("a style prop bound to a variable is an expression, not a fabricated style summary", () => {
  const [root] = parseComponent(`const X = () => <div style={dynamicStyles} />;`);
  assert.equal(root.props.style.kind, "expression");
});

test("finds JSX nested inside a conditional expression", () => {
  const [root] = parseComponent(`const X = ({ ok }) => <div>{ok && <span>yes</span>}</div>;`);
  assert.equal(root.children.length, 1);
  assert.equal(root.children[0].tagName, "span");
});

test("finds every top-level JSX root across multiple components in one file", () => {
  const roots = parseComponent(`
    const A = () => <div className="a" />;
    const B = () => <span className="b" />;
  `);
  assert.equal(roots.length, 2);
  assert.equal(roots[0].tagName, "div");
  assert.equal(roots[1].tagName, "span");
});

test("real fixture: parses this repo's own signature-features.jsx prototype without throwing", () => {
  const source = readFileSync(path.join(prototypesDir, "signature-features.jsx"), "utf8");
  const roots = parseComponent(source);
  assert.ok(roots.length > 0, "expected at least one top-level JSX root in a real prototype file");
});

test("real fixture: parses this repo's own interface-prototype.jsx without throwing", () => {
  const source = readFileSync(path.join(prototypesDir, "interface-prototype.jsx"), "utf8");
  const roots = parseComponent(source);
  assert.ok(roots.length > 0, "expected at least one top-level JSX root in a real prototype file");
});
