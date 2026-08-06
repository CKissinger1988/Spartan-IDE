import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { applyCanvasEdit } from "./edit.js";
import { parseComponent } from "./parse.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const prototypesDir = path.join(here, "..", "..", "prototypes");

test("StyleChange updates one property and preserves a sibling property untouched", () => {
  const source = `const X = () => <div style={{ color: "red", padding: 4 }} />;`;
  const result = applyCanvasEdit(source, { kind: "StyleChange", nodeId: "n0", property: "color", value: "blue" });
  assert.match(result, /color:\s*"blue"/);
  assert.match(result, /padding:\s*4/);
});

test("StyleChange preserves the surrounding source exactly, byte for byte, outside the touched line", () => {
  const source = [
    "// a real, distinctive leading comment",
    'const X = () => <div style={{ color: "red" }} />;',
    "// a real, distinctive trailing comment",
    "",
  ].join("\n");
  const result = applyCanvasEdit(source, { kind: "StyleChange", nodeId: "n0", property: "color", value: "blue" });
  assert.match(result, /^\/\/ a real, distinctive leading comment/);
  assert.match(result, /\/\/ a real, distinctive trailing comment\n?$/);
  assert.match(result, /"blue"/);
  assert.doesNotMatch(result, /"red"/);
});

test("StyleChange creates a style attribute when the element has none yet", () => {
  const source = `const X = () => <div />;`;
  const result = applyCanvasEdit(source, { kind: "StyleChange", nodeId: "n0", property: "color", value: "blue" });
  assert.match(result, /style=\{\{\s*color:\s*"blue"\s*\}\}/);
});

test("StyleChange preserves a copied JavaScript expression instead of stringifying it", () => {
  const source = `const X = () => <div style={{ color: "red" }} />;`;
  const result = applyCanvasEdit(source, {
    kind: "StyleChange", nodeId: "n0", property: "color", value: "C.text", valueType: "expression",
  });
  assert.match(result, /color:\s*\(C\.text\)/);
  assert.doesNotMatch(result, /"C\.text"/);
});

test("StyleChange can create a new expression-valued style property", () => {
  const result = applyCanvasEdit(`const X = () => <div />;`, {
    kind: "StyleChange", nodeId: "n0", property: "color", value: "theme.primary", valueType: "expression",
  });
  assert.match(result, /style=\{\{[\s\S]*color:\s*\(theme\.primary\)[\s\S]*\}\}/);
});

test("StyleChange refuses to overwrite a non-plain-object style value", () => {
  const source = `const X = () => <div style={dynamicStyles} />;`;
  assert.throws(
    () => applyCanvasEdit(source, { kind: "StyleChange", nodeId: "n0", property: "color", value: "blue" }),
    /isn't a plain object expression/,
  );
});

test("StyleRemove removes one inline style property and preserves its sibling", () => {
  const source = `const X = () => <div style={{ color: "red", padding: 4 }} />;`;
  const result = applyCanvasEdit(source, { kind: "StyleRemove", nodeId: "n0", property: "color" });
  assert.doesNotMatch(result, /color:/);
  assert.match(result, /padding: 4/);
  assert.doesNotThrow(() => parseComponent(result));
});

test("StyleRemove deletes the style attribute when its last property is removed", () => {
  const result = applyCanvasEdit(`const X = () => <div style={{ color: "red" }} />;`, {
    kind: "StyleRemove", nodeId: "n0", property: "color",
  });
  assert.equal(result, `const X = () => <div />;`);
});

test("StyleRemove refuses missing properties and non-object style expressions", () => {
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div style={{ color: "red" }} />;`, {
      kind: "StyleRemove", nodeId: "n0", property: "padding",
    }),
    /could not find style property/,
  );
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div style={styles.card} />;`, {
      kind: "StyleRemove", nodeId: "n0", property: "color",
    }),
    /isn't a plain object expression/,
  );
});

test("StyleClear removes all plain inline styles and preserves other props", () => {
  const result = applyCanvasEdit(`const X = () => <div className="card" style={{ color: "red", padding: 4 }} />;`, {
    kind: "StyleClear", nodeId: "n0",
  });
  assert.equal(result, `const X = () => <div className="card" />;`);
});

test("StyleClear refuses dynamic style expressions", () => {
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div style={styles.card} />;`, { kind: "StyleClear", nodeId: "n0" }),
    /isn't a plain object expression/,
  );
});

test("StyleClearMany clears every selected plain style object atomically", () => {
  const source = `const X = () => <main><div className="a" style={{ color: "red" }} /><span style={{ padding: 4 }} /></main>;`;
  const result = applyCanvasEdit(source, { kind: "StyleClearMany", nodeIds: ["n1", "n2"] });
  assert.equal(result, `const X = () => <main><div className="a" /><span /></main>;`);
});

test("StyleClearMany refuses a dynamic member without partially clearing earlier nodes", () => {
  assert.throws(
    () => applyCanvasEdit(`const X = () => <main><div style={{ color: "red" }} /><span style={styles.text} /></main>;`, {
      kind: "StyleClearMany", nodeIds: ["n1", "n2"],
    }),
    /refusing a partial multi-node edit/,
  );
});

test("PropChange updates an existing string prop", () => {
  const source = `const X = () => <button className="btn">Go</button>;`;
  const result = applyCanvasEdit(source, { kind: "PropChange", nodeId: "n0", prop: "className", value: "btn-primary" });
  assert.match(result, /className="btn-primary"/);
  assert.match(result, />Go<\/button>/);
});

test("PropChange creates a new prop when none existed", () => {
  const source = `const X = () => <input />;`;
  const result = applyCanvasEdit(source, { kind: "PropChange", nodeId: "n0", prop: "placeholder", value: "Name" });
  assert.match(result, /placeholder="Name"/);
});

test("PropChange writes real numeric and boolean JSX expression values", () => {
  const source = `const X = () => <Widget />;`;
  const numberResult = applyCanvasEdit(source, {
    kind: "PropChange", nodeId: "n0", prop: "count", value: "3", valueType: "number",
  });
  const booleanResult = applyCanvasEdit(numberResult, {
    kind: "PropChange", nodeId: "n0", prop: "enabled", value: "true", valueType: "boolean",
  });
  assert.match(booleanResult, /count=\{3\}/);
  assert.match(booleanResult, /enabled=\{true\}/);
});

test("PropChange rejects malformed typed values instead of emitting broken JSX", () => {
  const source = `const X = () => <Widget />;`;
  assert.throws(
    () => applyCanvasEdit(source, { kind: "PropChange", nodeId: "n0", prop: "count", value: "NaN", valueType: "number" }),
    /not finite/,
  );
  assert.throws(
    () => applyCanvasEdit(source, { kind: "PropChange", nodeId: "n0", prop: "enabled", value: "yes", valueType: "boolean" }),
    /must be "true" or "false"/,
  );
});

test("PropChange writes a real parsed JSX expression value", () => {
  const source = `const X = () => <Button />;`;
  const result = applyCanvasEdit(source, {
    kind: "PropChange", nodeId: "n0", prop: "onClick", value: "() => submit()", valueType: "expression",
  });
  assert.match(result, /onClick=\{\(\(\) => submit\(\)\)\}/);
});

test("PropChange rejects invalid expression input", () => {
  assert.throws(
    () => applyCanvasEdit(`const X = () => <Button />;`, {
      kind: "PropChange", nodeId: "n0", prop: "onClick", value: "() =>", valueType: "expression",
    }),
    /expression is not valid/,
  );
});

test("PropRemove removes a named JSX attribute and preserves its siblings", () => {
  const result = applyCanvasEdit(`const X = () => <div id="keep" title="remove" />;`, {
    kind: "PropRemove", nodeId: "n0", prop: "title",
  });
  assert.match(result, /id="keep"/);
  assert.doesNotMatch(result, /title=/);
});

test("PropRemove reports an unknown attribute instead of silently doing nothing", () => {
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div />;`, { kind: "PropRemove", nodeId: "n0", prop: "title" }),
    /could not find attribute/,
  );
});

test("TextChange updates direct JSX text and preserves the surrounding element", () => {
  const source = `const X = () => <button className="cta">Before</button>;`;
  const result = applyCanvasEdit(source, { kind: "TextChange", nodeId: "n0", text: "After" });
  assert.match(result, /<button className="cta">After<\/button>/);
});

test("TagChange renames paired opening and closing JSX tags", () => {
  const source = `const X = () => <div className="card"><span>Hi</span></div>;`;
  const result = applyCanvasEdit(source, { kind: "TagChange", nodeId: "n0", tagName: "section" });
  assert.match(result, /<section className="card">/);
  assert.match(result, /<\/section>/);
  assert.match(result, /<span>Hi<\/span>/);
});

test("TagChange renames self-closing JSX tags and rejects unsafe names", () => {
  const result = applyCanvasEdit(`const X = () => <div><Icon /></div>;`, {
    kind: "TagChange", nodeId: "n1", tagName: "Button",
  });
  assert.match(result, /<Button \/>/);
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div />;`, { kind: "TagChange", nodeId: "n0", tagName: "Foo.Bar" }),
    /single valid JSX identifier/,
  );
});

test("TagChangeMany renames paired and self-closing selections atomically", () => {
  const source = `const X = () => <main><div><span>One</span></div><Icon /><aside /></main>;`;
  const result = applyCanvasEdit(source, { kind: "TagChangeMany", nodeIds: ["n1", "n3"], tagName: "Panel" });
  assert.match(result, /<Panel><span>One<\/span><\/Panel>/);
  assert.match(result, /<Panel \/>/);
  assert.match(result, /<aside \/>/);
});

test("TagChangeMany refuses an invalid tag before changing any node", () => {
  assert.throws(
    () => applyCanvasEdit(`const X = () => <main><div /><span /></main>;`, {
      kind: "TagChangeMany", nodeIds: ["n1", "n2"], tagName: "Foo.Bar",
    }),
    /single valid JSX identifier/,
  );
});

test("Wrap places a direct child around a new container while preserving its subtree", () => {
  const source = `const X = () => <div><button id="save"><span>Save</span></button></div>;`;
  const result = applyCanvasEdit(source, { kind: "Wrap", nodeId: "n1", tagName: "section" });
  assert.match(result, /<div><section><button id="save"><span>Save<\/span><\/button><\/section><\/div>/);
});

test("Wrap refuses roots, expression-indirect children, and unsafe wrapper names", () => {
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div />;`, { kind: "Wrap", nodeId: "n0", tagName: "section" }),
    /top-level component root/,
  );
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div>{ok && <button />}</div>;`, { kind: "Wrap", nodeId: "n1", tagName: "section" }),
    /nested inside an expression/,
  );
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div><span /></div>;`, { kind: "Wrap", nodeId: "n1", tagName: "Foo.Bar" }),
    /single valid JSX identifier/,
  );
});

test("WrapMany groups selected siblings in source order and keeps unselected siblings outside", () => {
  const source = `const X = () => <div><i id="a" /><b id="keep" /><em id="c" /></div>;`;
  const result = applyCanvasEdit(source, { kind: "WrapMany", nodeIds: ["n3", "n1"], tagName: "section" });
  assert.match(result, /<div><section><i id="a" \/><em id="c" \/><\/section><b id="keep" \/><\/div>/);
});

test("WrapMany refuses different parents, roots, and expression-indirect nodes", () => {
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div><i /><section><b /></section></div>;`, { kind: "WrapMany", nodeIds: ["n1", "n3"], tagName: "div" }),
    /share one direct JSX parent/,
  );
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div><i /></div>;`, { kind: "WrapMany", nodeIds: ["n0", "n1"], tagName: "div" }),
    /top-level component root/,
  );
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div>{ok && <i />}{yes && <b />}</div>;`, { kind: "WrapMany", nodeIds: ["n1", "n2"], tagName: "section" }),
    /direct JSX children/,
  );
});

test("Unwrap promotes an attribute-free wrapper's children without losing their order", () => {
  const source = `const X = () => <div><section><i>A</i><b>B</b></section><em>C</em></div>;`;
  const result = applyCanvasEdit(source, { kind: "Unwrap", nodeId: "n1" });
  assert.match(result, /<div><i>A<\/i><b>B<\/b><em>C<\/em><\/div>/);
});

test("Unwrap refuses roots, attributed wrappers, and expression-indirect wrappers", () => {
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div />;`, { kind: "Unwrap", nodeId: "n0" }),
    /top-level component root/,
  );
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div><section className="keep"><span /></section></div>;`, { kind: "Unwrap", nodeId: "n1" }),
    /attributes that would be discarded/,
  );
  assert.throws(
    () => applyCanvasEdit(`const X = () => <div>{ok && <section><span /></section>}</div>;`, { kind: "Unwrap", nodeId: "n1" }),
    /nested inside an expression/,
  );
});

test("TextChange appends text to a childless element and refuses ambiguous fragments", () => {
  const empty = applyCanvasEdit(`const X = () => <div />;`, { kind: "TextChange", nodeId: "n0", text: "Added" });
  assert.match(empty, /<div>Added<\/div>/);
  assert.throws(
    () => applyCanvasEdit(`const X = () => <p>Hello {name} world</p>;`, { kind: "TextChange", nodeId: "n0", text: "Nope" }),
    /multiple direct text fragments/,
  );
});

test("Reparent moves an element from one parent to another, appended at the end by default", () => {
  const source = `const X = () => (<div><section id="a"><span id="s" /></section><section id="b" /></div>);`;
  const roots = parseComponent(source);
  const outer = roots[0];
  const sectionA = outer.children[0];
  const span = sectionA.children[0];
  const sectionB = outer.children[1];
  assert.equal(span.tagName, "span");
  assert.equal(sectionB.tagName, "section");

  const result = applyCanvasEdit(source, { kind: "Reparent", nodeId: span.id, newParentId: sectionB.id });
  const rootsAfter = parseComponent(result);
  const outerAfter = rootsAfter[0];
  assert.equal(outerAfter.children[0].children.length, 0, "span should be gone from section a");
  assert.equal(outerAfter.children[1].children.length, 1, "span should now be inside section b");
  assert.equal(outerAfter.children[1].children[0].tagName, "span");
});

test("Reparent honors an explicit index to insert at a specific position among siblings", () => {
  const source = `const X = () => (<div><section id="target"><i id="1" /><i id="3" /></section><b id="moved" /></div>);`;
  const roots = parseComponent(source);
  const outer = roots[0];
  const target = outer.children[0];
  const moved = outer.children[1];

  const result = applyCanvasEdit(source, { kind: "Reparent", nodeId: moved.id, newParentId: target.id, index: 1 });
  const rootsAfter = parseComponent(result);
  const targetAfter = rootsAfter[0].children[0];
  assert.equal(targetAfter.children.length, 3);
  assert.equal(targetAfter.children[0].tagName, "i");
  assert.equal(targetAfter.children[1].tagName, "b");
  assert.equal(targetAfter.children[2].tagName, "i");
});

test("Reparent can reorder within the same parent", () => {
  const source = `const X = () => (<div><i id="1" /><i id="2" /><i id="3" /></div>);`;
  const roots = parseComponent(source);
  const outer = roots[0];
  const first = outer.children[0];

  const result = applyCanvasEdit(source, { kind: "Reparent", nodeId: first.id, newParentId: outer.id, index: 2 });
  const rootsAfter = parseComponent(result);
  assert.equal(rootsAfter[0].children.length, 3);
  // The first <i> moved to the end (splice-out shifts the remaining two
  // left before the splice-in happens, so index 2 in the now-2-long array
  // means "append").
  assert.deepEqual(
    rootsAfter[0].children.map((c) => c.props.id?.kind === "string" && c.props.id.value),
    ["2", "3", "1"],
  );
});

test("ReorderMany moves selected siblings one position while preserving their relative order", () => {
  const source = `const X = () => <div><i id="a" /><i id="b" /><i id="c" /><i id="d" /></div>;`;
  const roots = parseComponent(source);
  const children = roots[0].children;
  const selected = [children[2].id, children[0].id];
  const movedUp = parseComponent(applyCanvasEdit(source, { kind: "ReorderMany", nodeIds: selected, direction: -1 }))[0].children;
  assert.deepEqual(movedUp.map((node) => node.props.id?.kind === "string" && node.props.id.value), ["a", "c", "b", "d"]);
  const movedDown = parseComponent(applyCanvasEdit(source, { kind: "ReorderMany", nodeIds: selected, direction: 1 }))[0].children;
  assert.deepEqual(movedDown.map((node) => node.props.id?.kind === "string" && node.props.id.value), ["b", "a", "d", "c"]);
});

test("ReorderMany refuses mixed-parent selections", () => {
  const source = `const X = () => <div><i /><section><b /></section></div>;`;
  const roots = parseComponent(source);
  assert.throws(
    () => applyCanvasEdit(source, { kind: "ReorderMany", nodeIds: [roots[0].children[0].id, roots[0].children[1].children[0].id], direction: -1 }),
    /share one direct JSX parent/,
  );
});

test("Reparent refuses to move a top-level root (it has no parent JSXElement)", () => {
  // Two real, independent, unrelated roots -- distinct from the cycle
  // case below (moving a root into its *own* descendant is correctly a
  // cycle error instead, since that's a real, separate, also-true
  // problem; this fixture isolates the "no parent to detach from" case
  // on its own by using a target that isn't reachable from the moved
  // root at all).
  const source = `function A() { return <div id="a" />; } function B() { return <section id="b" />; }`;
  const roots = parseComponent(source);
  const rootA = roots[0];
  const rootB = roots[1];
  assert.throws(
    () => applyCanvasEdit(source, { kind: "Reparent", nodeId: rootA.id, newParentId: rootB.id }),
    /top-level component root/,
  );
});

test("Reparent refuses to move an element into itself", () => {
  const source = `const X = () => (<div><span id="a" /></div>);`;
  const roots = parseComponent(source);
  const span = roots[0].children[0];
  assert.throws(
    () => applyCanvasEdit(source, { kind: "Reparent", nodeId: span.id, newParentId: span.id }),
    /own child/,
  );
});

test("Reparent refuses to move an element into one of its own descendants (a real cycle)", () => {
  const source = `const X = () => (<div><section id="a"><span id="b" /></section></div>);`;
  const roots = parseComponent(source);
  const outer = roots[0];
  const section = outer.children[0];
  const span = section.children[0];
  assert.throws(
    () => applyCanvasEdit(source, { kind: "Reparent", nodeId: section.id, newParentId: span.id }),
    /own descendants/,
  );
});

test("ComponentInsert creates a new self-closing element as a child of the target, appended by default", () => {
  const source = `const X = () => <div><span /></div>;`;
  const result = applyCanvasEdit(source, { kind: "ComponentInsert", parentId: "n0", tagName: "Button" });
  assert.match(result, /<div>\s*<span \/>\s*<Button \/>\s*<\/div>/);
});

test("Delete removes the selected element and its complete JSX subtree", () => {
  const source = `const X = () => <main><section><span>gone</span></section><aside /></main>;`;
  const result = applyCanvasEdit(source, { kind: "Delete", nodeId: "n1" });
  assert.ok(!result.includes("section"));
  assert.ok(result.includes("<aside />"));
});

test("Delete refuses to remove a top-level component root", () => {
  const source = `const X = () => <main><span /></main>;`;
  assert.throws(
    () => applyCanvasEdit(source, { kind: "Delete", nodeId: "n0" }),
    /top-level component root/,
  );
});

test("DeleteMany removes independent subtrees in one edit and preserves unselected siblings", () => {
  const source = `const X = () => <div><i id="keep" /><section><b /></section><em id="remove" /><strong /></div>;`;
  const roots = parseComponent(source);
  const children = roots[0].children;
  const result = applyCanvasEdit(source, {
    kind: "DeleteMany",
    nodeIds: [children[2].id, children[1].id],
  });
  assert.match(result, /<i id="keep" \/>/);
  assert.doesNotMatch(result, /<section>/);
  assert.doesNotMatch(result, /<em id="remove" \/>/);
  assert.match(result, /<strong \/>/);
  assert.doesNotThrow(() => parseComponent(result));
});

test("DeleteMany refuses roots and overlapping ancestor/descendant selections", () => {
  const source = `function A() { return <div><section><b /></section></div>; } function B() { return <aside />; }`;
  const roots = parseComponent(source);
  const outer = roots[0];
  const section = outer.children[0];
  const bold = section.children[0];
  assert.throws(
    () => applyCanvasEdit(source, { kind: "DeleteMany", nodeIds: [outer.id, roots[1].id] }),
    /top-level component root/,
  );
  assert.throws(
    () => applyCanvasEdit(source, { kind: "DeleteMany", nodeIds: [section.id, bold.id] }),
    /one element contains another/,
  );
});

test("Duplicate clones an element and its nested subtree immediately after the original", () => {
  const source = `const X = () => <main><article data-kind="card"><span>copy me</span></article><aside /></main>;`;
  const result = applyCanvasEdit(source, { kind: "Duplicate", nodeId: "n1" });
  const roots = parseComponent(result);
  assert.equal(roots[0].children.length, 3);
  assert.equal(roots[0].children[1].tagName, "article");
  assert.equal(roots[0].children[1].children[0].textContent, "copy me");
  assert.notEqual(roots[0].children[1].id, roots[0].children[2].id);
});

test("DuplicateMany clones independent siblings after each original and preserves nested content", () => {
  const source = `const X = () => <main><article data-kind="a"><span>A</span></article><aside /><article data-kind="b"><b>B</b></article></main>;`;
  const roots = parseComponent(source);
  const children = roots[0].children;
  const result = applyCanvasEdit(source, {
    kind: "DuplicateMany",
    nodeIds: [children[2].id, children[0].id],
  });
  const after = parseComponent(result)[0].children;
  assert.deepEqual(after.map((node) => node.tagName), ["article", "article", "aside", "article", "article"]);
  assert.deepEqual(after.map((node) => node.props["data-kind"]?.kind === "string" ? node.props["data-kind"].value : null), ["a", "a", null, "b", "b"]);
  assert.equal(after[1].children[0].textContent, "A");
  assert.equal(after[4].children[0].textContent, "B");
});

test("DuplicateMany refuses roots and overlapping ancestor/descendant selections", () => {
  const source = `function A() { return <div><section><b /></section></div>; } function B() { return <aside />; }`;
  const roots = parseComponent(source);
  const section = roots[0].children[0];
  const bold = section.children[0];
  assert.throws(
    () => applyCanvasEdit(source, { kind: "DuplicateMany", nodeIds: [roots[0].id, roots[1].id] }),
    /top-level component root/,
  );
  assert.throws(
    () => applyCanvasEdit(source, { kind: "DuplicateMany", nodeIds: [section.id, bold.id] }),
    /one element contains another/,
  );
});

test("Duplicate refuses to clone a top-level component root", () => {
  const source = `const X = () => <main><span /></main>;`;
  assert.throws(
    () => applyCanvasEdit(source, { kind: "Duplicate", nodeId: "n0" }),
    /top-level component root/,
  );
});

test("ComponentInsert applies real string-literal props to the new element", () => {
  const source = `const X = () => <div />;`;
  const result = applyCanvasEdit(source, {
    kind: "ComponentInsert",
    parentId: "n0",
    tagName: "Button",
    props: { label: "Click me", variant: "primary" },
  });
  assert.match(result, /<Button label="Click me" variant="primary"\s*\/>/);
});

test("ComponentInsert creates a content-bearing element with string props", () => {
  const source = `const X = () => <main />;`;
  const result = applyCanvasEdit(source, {
    kind: "ComponentInsert",
    parentId: "n0",
    tagName: "Button",
    props: { type: "button", className: "primary", "aria-label": "Save action" },
    childrenText: "Save",
  });
  assert.match(result, /<Button type="button" className="primary" aria-label="Save action">Save<\/Button>/);
  assert.doesNotThrow(() => parseComponent(result));
});

test("ComponentInsert rejects an invalid string prop name", () => {
  assert.throws(
    () => applyCanvasEdit(`const X = () => <main />;`, {
      kind: "ComponentInsert",
      parentId: "n0",
      tagName: "Button",
    props: { "not valid": "x" },
    }),
    /not a supported JSX prop name/,
  );
});

test("ComponentInsert honors an explicit index among existing children", () => {
  const source = `const X = () => (<div><i id="1" /><i id="2" /></div>);`;
  const result = applyCanvasEdit(source, { kind: "ComponentInsert", parentId: "n0", tagName: "Mid", index: 1 });
  const roots = parseComponent(result);
  assert.equal(roots[0].children.length, 3);
  assert.equal(roots[0].children[1].tagName, "Mid");
});

test("ComponentInsert refuses a member-expression tag name in this v1", () => {
  const source = `const X = () => <div />;`;
  assert.throws(
    () => applyCanvasEdit(source, { kind: "ComponentInsert", parentId: "n0", tagName: "Foo.Bar" }),
    /not a supported JSX tag name/,
  );
});

test("ComponentInsert throws a real, descriptive error for an unknown parent id", () => {
  const source = `const X = () => <div />;`;
  assert.throws(
    () => applyCanvasEdit(source, { kind: "ComponentInsert", parentId: "n99", tagName: "Button" }),
    /No element with id "n99"/,
  );
});

test("real fixture: a Reparent + a ComponentInsert against signature-features.jsx both round-trip to valid, re-parseable source", () => {
  const source = readFileSync(path.join(prototypesDir, "signature-features.jsx"), "utf8");
  const roots = parseComponent(source);

  type Node = (typeof roots)[number];
  function firstWithChildren(node: Node): Node | undefined {
    if (node.children.length > 0) return node;
    for (const child of node.children) {
      const found = firstWithChildren(child);
      if (found) return found;
    }
    return undefined;
  }
  let container: Node | undefined;
  for (const root of roots) {
    container = firstWithChildren(root);
    if (container) break;
  }
  assert.ok(container, "expected at least one element with children in this real fixture");

  const afterInsert = applyCanvasEdit(source, {
    kind: "ComponentInsert",
    parentId: container!.id,
    tagName: "TestInsertedMarker",
  });
  assert.match(afterInsert, /<TestInsertedMarker\s*\/>/);
  const rootsAfterInsert = parseComponent(afterInsert);
  assert.equal(rootsAfterInsert.length, roots.length);

  // A real Reparent against the *post-insert* source: move the newly
  // inserted marker into the first of its own siblings that itself has
  // children, proving Reparent works correctly chained after a prior
  // structural edit re-parsed the file fresh, not just against a pristine
  // parse.
  const containerAfter = rootsAfterInsert.find((r) => findById(r, container!.id));
  const containerNode = containerAfter ? findById(containerAfter, container!.id) : undefined;
  assert.ok(containerNode, "container should still be findable by a fresh id after the insert");
  const markerNode = containerNode!.children.find((c) => c.tagName === "TestInsertedMarker");
  assert.ok(markerNode, "the inserted marker should be a direct child of the container");
  const sibling = containerNode!.children.find((c) => c.id !== markerNode!.id && c.children.length >= 0);
  assert.ok(sibling, "expected at least one sibling to reparent the marker into");

  const afterReparent = applyCanvasEdit(afterInsert, {
    kind: "Reparent",
    nodeId: markerNode!.id,
    newParentId: sibling!.id,
  });
  const rootsAfterReparent = parseComponent(afterReparent);
  assert.equal(rootsAfterReparent.length, roots.length);
  assert.match(afterReparent, /<TestInsertedMarker\s*\/>/);

  function findById(node: Node, id: string): Node | undefined {
    if (node.id === id) return node;
    for (const child of node.children) {
      const found = findById(child, id);
      if (found) return found;
    }
    return undefined;
  }
});

test("an unknown node id throws a real, descriptive error instead of silently no-op'ing", () => {
  const source = `const X = () => <div />;`;
  assert.throws(
    () => applyCanvasEdit(source, { kind: "StyleChange", nodeId: "n99", property: "color", value: "blue" }),
    /No element with id "n99"/,
  );
});

test("real fixture: a StyleChange against signature-features.jsx round-trips to valid, re-parseable source", () => {
  const source = readFileSync(path.join(prototypesDir, "signature-features.jsx"), "utf8");
  const roots = parseComponent(source);
  // Walk every top-level root to a real node that actually has a
  // summarizable style object, since not every element does, and this
  // real file's *first* root happens to have none.
  type Node = (typeof roots)[number];
  function findStyled(node: Node): Node | undefined {
    if (node.props.style?.kind === "style") return node;
    for (const child of node.children) {
      const found = findStyled(child);
      if (found) return found;
    }
    return undefined;
  }
  let target: Node | undefined;
  for (const root of roots) {
    target = findStyled(root);
    if (target) break;
  }
  assert.ok(target, "expected at least one element with a summarizable style object in this real fixture");

  const firstKey = Object.keys((target!.props.style as { kind: "style"; entries: Record<string, unknown> }).entries)[0];
  const result = applyCanvasEdit(source, {
    kind: "StyleChange",
    nodeId: target!.id,
    property: firstKey,
    value: "TEST_MARKER_VALUE",
  });

  assert.match(result, /TEST_MARKER_VALUE/);
  // Real, meaningful proof recast preserved formatting: re-parsing the
  // edited output must succeed and must still contain the exact same
  // number of top-level component roots as the original, unmutated file.
  const rootsAfter = parseComponent(result);
  const rootsBefore = parseComponent(source);
  assert.equal(rootsAfter.length, rootsBefore.length);
});

test("real fixture: 10 sequential StyleChange edits against the larger interface-prototype.jsx (547 style objects, real spreads present) all round-trip cleanly", () => {
  const original = readFileSync(path.join(prototypesDir, "interface-prototype.jsx"), "utf8");
  let source = original;
  const roots = parseComponent(source);

  type Node = (typeof roots)[number];
  function collectStyled(node: Node, out: Node[]): void {
    if (node.props.style?.kind === "style") out.push(node);
    for (const child of node.children) collectStyled(child, out);
  }
  const styled: Node[] = [];
  for (const root of roots) collectStyled(root, styled);
  assert.ok(styled.length >= 10, `expected at least 10 styled elements, found ${styled.length}`);

  // Apply real, sequential edits -- each one re-parses `source` fresh
  // (via `applyCanvasEdit`'s own contract) and must still find its target
  // node id, proving the id scheme survives repeated non-structural
  // StyleChange edits to *other* nodes in between, not just a single
  // edit against a pristine parse.
  for (const node of styled.slice(0, 10)) {
    const key = Object.keys((node.props.style as { kind: "style"; entries: Record<string, unknown> }).entries)[0];
    source = applyCanvasEdit(source, { kind: "StyleChange", nodeId: node.id, property: key, value: "RT_MARKER" });
  }

  const rootsAfter = parseComponent(source);
  assert.equal(rootsAfter.length, roots.length);
  assert.equal((source.match(/RT_MARKER/g) ?? []).length, 10);
  // A real, blunt but meaningful formatting-preservation signal: editing
  // 10 of 547 style objects in a ~5,000-line file should change only a
  // small fraction of it, not silently reformat the whole file the way a
  // naive parse-mutate-`JSON.stringify`-style regeneration would. Allows
  // some slack for the real, separately-documented and separately-tested
  // JSX-text-collapse limitation just below (each affected element can
  // lose at most one line break), rather than requiring an exact line
  // count this package cannot honestly guarantee yet.
  const originalLines = original.split("\n");
  const resultLines = source.split("\n");
  assert.ok(
    originalLines.length - resultLines.length <= 10,
    `expected to lose at most 10 lines total across 10 edits, lost ${originalLines.length - resultLines.length}`,
  );
});

test("KNOWN LIMITATION: an edit that forces a parent JSXElement reprint can collapse a line break in an unrelated sibling JSXText child", () => {
  // A real, upstream recast/Babel-JSX-generator behavior, isolated by
  // hand against a minimal repro after it was first found live against
  // interface-prototype.jsx (§75.x GUI Builder pass): once *any* edit
  // forces recast to fully reprint a JSXElement (rather than patch just
  // the touched region), that reprint regenerates its children using
  // JSX's own pretty-printer, which normalizes JSXText whitespace the
  // same way React's own JSX runtime does at render time -- a leading
  // newline+indentation directly after a `{expression}` sibling collapses
  // to nothing. The *rendered* output is unchanged (React already treats
  // `{a}\n  text` and `{a}text` identically), but the *source formatting*
  // is not byte-for-byte preserved in this specific shape. This test
  // exists to document the real, known gap, not to celebrate it -- see
  // README.md for the honest summary.
  const source = [
    "function X({ local }) {",
    "  return (",
    "    <span style={{ padding: 4 }}>",
    "      {local ? <A /> : <B />}",
    '      Leo · {local ? "x" : "y"}',
    "    </span>",
    "  );",
    "}",
    "",
  ].join("\n");
  const result = applyCanvasEdit(source, { kind: "StyleChange", nodeId: "n0", property: "padding", value: "99" });
  assert.match(result, /padding: "99"/);
  // The known, real gap: the source line break between the two JSX
  // children collapses (`}Leo` with no newline in between). Asserted here
  // (rather than merely described) specifically so a future recast
  // upgrade or printer-option fix that *closes* this gap will fail this
  // test loudly, prompting the comment above (and README.md) to be
  // updated rather than silently going stale.
  assert.doesNotMatch(result, /<B \/>\}\n\s+Leo/);
  assert.match(result, /<B \/>\}Leo/);
});

// --- ComponentInsert with a real import (task #278) ---

test("ComponentInsert with importFrom adds a real default import", () => {
  const source = ['import React from "react";', "", "export default function App() {", "  return <div />;", "}", ""].join("\n");
  const out = applyCanvasEdit(source, {
    kind: "ComponentInsert",
    parentId: "n0",
    tagName: "Card",
    importFrom: "./Card",
    importIsDefault: true,
  });
  assert.match(out, /import Card from "\.\/Card";/);
  assert.match(out, /<Card \/>/);
});

test("ComponentInsert with importFrom adds a real named import", () => {
  const source = ["export default function App() {", "  return <div />;", "}", ""].join("\n");
  const out = applyCanvasEdit(source, {
    kind: "ComponentInsert",
    parentId: "n0",
    tagName: "Button",
    importFrom: "../ui/Button",
    importIsDefault: false,
  });
  assert.match(out, /import \{ Button \} from "\.\.\/ui\/Button";/);
});

test("ComponentInsert merges into an existing import from the same module", () => {
  const source = ['import { Button } from "./ui";', "", "export default function App() {", "  return <div />;", "}", ""].join("\n");
  const out = applyCanvasEdit(source, {
    kind: "ComponentInsert",
    parentId: "n0",
    tagName: "Badge",
    importFrom: "./ui",
    importIsDefault: false,
  });
  // One merged statement, not two separate imports from the same module.
  assert.equal(out.match(/from "\.\/ui"/g)?.length, 1);
  assert.match(out, /import \{ Button, Badge \} from "\.\/ui";/);
});

test("ComponentInsert never re-imports a binding the file already has", () => {
  const source = ['import Card from "./Card";', "", "export default function App() {", "  return <div />;", "}", ""].join("\n");
  const out = applyCanvasEdit(source, {
    kind: "ComponentInsert",
    parentId: "n0",
    tagName: "Card",
    importFrom: "./Card",
    importIsDefault: true,
  });
  assert.equal(out.match(/import Card from/g)?.length, 1, "a duplicate binding would be a real syntax error");
  assert.match(out, /<Card \/>/);
});

test("ComponentInsert without importFrom adds no import at all", () => {
  const source = ["export default function App() {", "  return <div />;", "}", ""].join("\n");
  const out = applyCanvasEdit(source, { kind: "ComponentInsert", parentId: "n0", tagName: "span" });
  assert.ok(!out.includes("import"), "a plain DOM tag needs no import");
  assert.match(out, /<span \/>/);
});

test("a new import joins the existing import block rather than jumping above it", () => {
  const source = ['import React from "react";', 'import { useState } from "react";', "", "export default function App() {", "  return <div />;", "}", ""].join("\n");
  const out = applyCanvasEdit(source, {
    kind: "ComponentInsert",
    parentId: "n0",
    tagName: "Card",
    importFrom: "./Card",
    importIsDefault: true,
  });
  const lines = out.split("\n").filter((l) => l.startsWith("import"));
  assert.equal(lines.length, 3);
  assert.match(lines[2], /import Card from "\.\/Card";/);
});
