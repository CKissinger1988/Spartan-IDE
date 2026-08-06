export type HandoffFindingSeverity = "error" | "warning" | "pass" | "info";

export interface DesignHandoffFinding {
  severity: HandoffFindingSeverity;
  message: string;
}

export interface DesignHandoffInput {
  tagName: string;
  nodeId: string;
  sourceLocation: string;
  sourceText: string;
  renderedSnapshot: string;
  screenReaderAnnouncement: string;
  screenReaderDetails: string[];
  findings: DesignHandoffFinding[];
}

/** Builds the portable Markdown handoff artifact from live Design data. */
export function buildDesignHandoffMarkdown(input: DesignHandoffInput): string {
  const findings = input.findings.length > 0
    ? input.findings.map((finding) => `- ${finding.severity.toUpperCase()}: ${finding.message}`).join("\n")
    : "- INFO: No audit findings were produced.";
  return [
    "# Spartan GUI Builder design handoff",
    `Element: <${input.tagName}> #${input.nodeId}`,
    `Source: ${input.sourceLocation}`,
    "",
    "## JSX source",
    input.sourceText || "(exact JSX source unavailable)",
    "",
    "## Rendered inspection",
    input.renderedSnapshot,
    "",
    "## Accessibility audit",
    findings,
    "",
    "## Estimated screen-reader announcement",
    input.screenReaderAnnouncement,
    ...input.screenReaderDetails.map((detail) => `- ${detail}`),
    "",
  ].join("\n");
}
