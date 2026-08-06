export interface ScreenReaderPropSummary {
  kind: "string" | "expression" | "style";
  value?: string;
  source?: string;
}

export interface ScreenReaderNode {
  tagName: string;
  props: Record<string, ScreenReaderPropSummary>;
  textContent: string | null;
}

export interface ScreenReaderPreview {
  announcement: string;
  details: string[];
}

function propSummaryValue(node: ScreenReaderNode, name: string): string | null {
  const summary = node.props[name];
  if (!summary) return null;
  if (summary.kind === "string") return summary.value?.trim() ?? "";
  if (summary.kind === "expression") return summary.source?.trim() ?? "";
  return null;
}

/** Estimates the announcement a browser accessibility tree is likely to
 * expose for a JSX element. Labelled-by references and runtime state are
 * reported from parsed props without pretending to resolve a live DOM. */
export function screenReaderPreview(node: ScreenReaderNode): ScreenReaderPreview {
  const tagName = node.tagName.toLowerCase();
  const details: string[] = [];
  if (propSummaryValue(node, "aria-hidden")?.toLowerCase() === "true") {
    return { announcement: "Hidden from assistive technology", details: ["aria-hidden=true"] };
  }
  const explicitRole = propSummaryValue(node, "role")?.toLowerCase();
  const inputType = tagName === "input" ? (propSummaryValue(node, "type")?.toLowerCase() ?? "text") : null;
  const implicitRoles: Record<string, string> = {
    a: "link", article: "article", button: "button", dialog: "dialog", form: "form",
    h1: "heading", h2: "heading", h3: "heading", h4: "heading", h5: "heading", h6: "heading",
    img: "img", li: "listitem", main: "main", nav: "navigation", option: "option",
    progress: "progressbar", select: "combobox", table: "table", textarea: "textbox", ul: "list", ol: "list",
  };
  const inputRoles: Record<string, string> = {
    button: "button", checkbox: "checkbox", image: "button", radio: "radio", range: "slider", reset: "button", submit: "button",
  };
  const role = explicitRole || (inputType ? inputRoles[inputType] ?? "textbox" : null) || implicitRoles[tagName] || "generic";
  if (/^h[1-6]$/.test(tagName)) details.push(`level ${tagName.slice(1)}`);

  const ariaLabel = propSummaryValue(node, "aria-label");
  const labelledBy = propSummaryValue(node, "aria-labelledby");
  const title = propSummaryValue(node, "title");
  const alt = tagName === "img" ? propSummaryValue(node, "alt") : null;
  const name = ariaLabel || (labelledBy ? `labelled by ${labelledBy}` : null) || title || (alt === "" ? null : alt) || node.textContent?.trim() || null;
  if (name) details.push(`name “${name}”`);

  const states: string[] = [];
  for (const [prop, label] of [["disabled", "disabled"], ["aria-disabled", "disabled"], ["aria-checked", "checked"], ["aria-expanded", "expanded"], ["aria-selected", "selected"], ["aria-pressed", "pressed"], ["aria-required", "required"]] as const) {
    const value = propSummaryValue(node, prop);
    if (value !== null && value !== "false") states.push(`${label} ${value === "true" ? "true" : value}`);
  }
  if (states.length > 0) details.push(...states);
  return { announcement: [role, name, ...states].filter(Boolean).join(", ") || "Unnamed generic element", details };
}
