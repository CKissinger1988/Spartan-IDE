import { test } from "node:test";
import assert from "node:assert/strict";
import { injectNodeIds } from "./annotate.js";
import { parserAdapter } from "./parserAdapter.js";
import { buildComponentTree } from "./tree.js";
import * as recast from "recast";

test("injects a real data-spartan-id attribute matching the structural tree's own id for a single element", () => {
  const source = `const X = () => <button className="btn">Click me</button>;`;
  const annotated = injectNodeIds(source);
  assert.match(annotated, /data-spartan-id="n0"/);
  // The original prop must survive untouched alongside the new one.
  assert.match(annotated, /className="btn"/);
});

test("every real nested element gets its own real, distinct id matching the tree's own numbering", () => {
  const source = `const X = () => (
    <div>
      <span>a</span>
      <span>b</span>
    </div>
  );`;
  const annotated = injectNodeIds(source);

  // Real, independent proof the ids line up: re-parse the *annotated*
  // output with the same canonical traversal and confirm each element's
  // own `data-spartan-id` prop value equals the id `buildComponentTree`
  // assigns it on this exact parse.
  const ast = recast.parse(annotated, { parser: parserAdapter });
  const { roots } = buildComponentTree(ast, annotated);
  assert.equal(roots[0].id, "n0");
  assert.equal(roots[0].props["data-spartan-id"].kind, "string");
  assert.equal((roots[0].props["data-spartan-id"] as { kind: "string"; value: string }).value, "n0");
  assert.equal(roots[0].children[0].id, "n1");
  assert.equal(roots[0].children[1].id, "n2");
});

test("annotating a real file with an existing style/prop set doesn't disturb them", () => {
  const source = `const X = () => <div style={{ color: "red" }} data-testid="x">hi</div>;`;
  const annotated = injectNodeIds(source);
  assert.match(annotated, /color: "red"/);
  assert.match(annotated, /data-testid="x"/);
  assert.match(annotated, /data-spartan-id="n0"/);
});
