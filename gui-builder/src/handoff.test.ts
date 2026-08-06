import assert from "node:assert/strict";
import test from "node:test";
import { buildDesignHandoffMarkdown } from "./handoff.js";

test("buildDesignHandoffMarkdown creates a portable spec with source, inspection, and accessibility sections", () => {
  const report = buildDesignHandoffMarkdown({
    tagName: "button",
    nodeId: "n3",
    sourceLocation: "src/Card.jsx:8:4",
    sourceText: "<button aria-label=\"Save\">Save</button>",
    renderedSnapshot: "button {\n  width: 120px;\n}",
    screenReaderAnnouncement: "button, Save",
    screenReaderDetails: ["name “Save”"],
    findings: [{ severity: "pass", message: "Interactive element has a detectable accessible name." }],
  });
  assert.match(report, /^# Spartan GUI Builder design handoff/m);
  assert.match(report, /## JSX source[\s\S]*aria-label=/);
  assert.match(report, /## Rendered inspection[\s\S]*width: 120px/);
  assert.match(report, /## Accessibility audit[\s\S]*PASS:/);
  assert.match(report, /## Estimated screen-reader announcement[\s\S]*button, Save/);
});

test("buildDesignHandoffMarkdown states when no audit findings were produced", () => {
  const report = buildDesignHandoffMarkdown({
    tagName: "div",
    nodeId: "n0",
    sourceLocation: "Card.jsx:1:1",
    sourceText: "<div />",
    renderedSnapshot: "div {}",
    screenReaderAnnouncement: "generic",
    screenReaderDetails: [],
    findings: [],
  });
  assert.match(report, /INFO: No audit findings were produced\./);
});
