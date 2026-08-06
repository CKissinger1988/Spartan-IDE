import assert from "node:assert/strict";
import test from "node:test";
import { buildComponentPropInput, componentPropControl, missingRequiredComponentProps } from "./component-insertion.js";

test("maps safe prop types to guided controls", () => {
  assert.deepEqual(componentPropControl("boolean"), { kind: "boolean" });
  assert.deepEqual(componentPropControl("number"), { kind: "number" });
  assert.deepEqual(componentPropControl('"primary" | "secondary"'), { kind: "enum", options: ["primary", "secondary"] });
  assert.deepEqual(componentPropControl("React.ReactNode"), { kind: "text" });
});

test("builds typed insertion props and omits blank optional values", () => {
  const component = {
    propHints: [
      { name: "label", type: "string", required: true },
      { name: "count", type: "number", required: false },
      { name: "enabled", type: "boolean", required: false },
      { name: "tone", type: '"quiet" | "loud"', required: false },
    ],
  };
  assert.equal(buildComponentPropInput(component, { label: "Save", count: "2", enabled: "true", tone: "quiet" }), "label=Save\ncount:number=2\nenabled:boolean=true\ntone=quiet");
  assert.deepEqual(missingRequiredComponentProps(component.propHints, { label: "", count: "2" }), ["label"]);
});
