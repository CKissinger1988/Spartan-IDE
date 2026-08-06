export type ComponentPropKind = "string" | "number" | "boolean" | "enum" | "slot";

export interface ComponentPropDefinition {
  name: string;
  kind: ComponentPropKind;
  enumValues?: string[];
}

export interface ComponentScaffoldInput {
  componentName: string;
  props: ComponentPropDefinition[];
  defaultVariant?: string;
}

export interface ComponentPlaygroundInput extends ComponentScaffoldInput {
  /** Relative import specifier from the generated playground file. */
  componentImportPath: string;
}

function assertIdentifier(value: string, label: string): string {
  const trimmed = value.trim();
  if (!/^[A-Za-z_$][A-Za-z0-9_$]*$/.test(trimmed)) throw new Error(`${label} must be a valid TypeScript identifier.`);
  return trimmed;
}

function propType(prop: ComponentPropDefinition): string {
  if (prop.kind === "number") return "number";
  if (prop.kind === "boolean") return "boolean";
  if (prop.kind === "slot") return "React.ReactNode";
  if (prop.kind === "enum") {
    const values = (prop.enumValues ?? []).map((value) => {
      const trimmed = value.trim();
      if (!/^[A-Za-z0-9_-]+$/.test(trimmed)) throw new Error("Enum values may contain only letters, numbers, hyphens, and underscores.");
      return JSON.stringify(trimmed);
    });
    if (values.length < 2) throw new Error(`Enum prop "${prop.name}" needs at least two values in parentheses.`);
    return values.join(" | ");
  }
  return "string";
}

function validateProps(input: ComponentScaffoldInput): { componentName: string; props: ComponentPropDefinition[] } {
  const componentName = assertIdentifier(input.componentName, "Component name");
  const props = input.props.map((prop) => ({ ...prop, name: assertIdentifier(prop.name, "Prop name") }));
  const uniqueNames = new Set<string>();
  for (const prop of props) {
    if (uniqueNames.has(prop.name)) throw new Error(`Duplicate prop "${prop.name}".`);
    uniqueNames.add(prop.name);
    propType(prop);
  }
  return { componentName, props };
}

/** Generates a small, valid, reviewable TSX component scaffold. */
export function buildComponentScaffold(input: ComponentScaffoldInput): string {
  const { componentName, props } = validateProps(input);
  const variant = (input.defaultVariant ?? "default").trim() || "default";
  if (!/^[A-Za-z0-9_-]+$/.test(variant)) throw new Error("Default variant may contain only letters, numbers, hyphens, and underscores.");
  const hasChildren = props.some((prop) => prop.kind === "slot" && prop.name === "children");
  const propLines = props.length > 0
    ? props.map((prop) => `  ${prop.name}${prop.kind === "slot" ? "?" : ""}: ${propType(prop)};`).join("\n")
    : "  // Add typed props here when the component API is ready.";
  const renderedProps = props
    .filter((prop) => prop.kind !== "slot")
    .map((prop) => `      <span data-prop="${prop.name}">{String(${prop.name})}</span>`)
    .join("\n");
  const destructured = props.map((prop) => prop.name).join(", ");
  return `import React from "react";

export interface ${componentName}Props {
${propLines}
}

export default function ${componentName}({ ${destructured} }: ${componentName}Props) {
  return (
    <div data-spartan-component="${componentName}" data-variant="${variant}">
${renderedProps || "      <span data-placeholder=\"true\">${componentName}</span>"}
${hasChildren ? "      {children}" : ""}
    </div>
  );
}
`;
}

function playgroundInitialValue(prop: ComponentPropDefinition): string {
  if (prop.kind === "number") return "0";
  if (prop.kind === "boolean") return "false";
  if (prop.kind === "enum") return JSON.stringify(prop.enumValues?.[0]?.trim() ?? "");
  return JSON.stringify(prop.kind === "slot" ? "Slot content" : "Example value");
}

function playgroundControl(prop: ComponentPropDefinition): string {
  const label = JSON.stringify(prop.name);
  if (prop.kind === "boolean") {
    return "        <label><input type=\"checkbox\" checked={" + prop.name + "} onChange={(event) => set" + prop.name + "(event.target.checked)} /> {" + label + "}</label>";
  }
  if (prop.kind === "enum") {
    const options = (prop.enumValues ?? []).map((value) => "            <option value=" + JSON.stringify(value) + ">" + value + "</option>").join("\n");
    return "        <label>{" + label + "}\n          <select value={" + prop.name + "} onChange={(event) => set" + prop.name + "(event.target.value as typeof " + prop.name + ")}>\n" + options + "\n          </select>\n        </label>";
  }
  const inputType = prop.kind === "number" ? "number" : "text";
  const value = prop.kind === "number" ? "Number(event.target.value)" : "event.target.value";
  return "        <label>{" + label + "} <input type=\"" + inputType + "\" value={" + prop.name + "} onChange={(event) => set" + prop.name + "(" + value + ")} /></label>";
}

/** Generates a standalone, typed playground companion for a component. */
export function buildComponentPlaygroundScaffold(input: ComponentPlaygroundInput): string {
  const { componentName, props } = validateProps(input);
  const importPath = input.componentImportPath.trim();
  if (!/^(?:\.\.\/|\.\/)[^\n\r"']+$/.test(importPath)) {
    throw new Error("Component import path must be a relative path without quotes or newlines.");
  }
  const stateLines = props.map((prop) => {
    const typeParameter = prop.kind === "enum" ? "<" + propType(prop) + ">" : "";
    return "  const [" + prop.name + ", set" + prop.name + "] = useState" + typeParameter + "(" + playgroundInitialValue(prop) + ");";
  }).join("\n");
  const controls = props.length > 0
    ? props.map(playgroundControl).join("\n")
    : "        <div>No declared props. Add props to the component schema to generate controls.</div>";
  const bindings = props.map((prop) => "        " + prop.name + "={" + prop.name + "}").join("\n");
  return "import React, { useState } from \"react\";\n"
    + "import " + componentName + " from " + JSON.stringify(importPath) + ";\n\n"
    + "export default function " + componentName + "Playground() {\n"
    + (stateLines || "  // This component has no declared props.") + "\n"
    + "  return (\n"
    + "    <main style={{ fontFamily: \"sans-serif\", maxWidth: 720, margin: \"0 auto\", padding: 24 }}>\n"
    + "      <h1>" + componentName + " playground</h1>\n"
    + "      <fieldset style={{ display: \"grid\", gap: 12, marginBottom: 24 }}>\n"
    + "        <legend>Props</legend>\n"
    + controls + "\n"
    + "      </fieldset>\n"
    + "      <section aria-label=\"Component preview\">\n"
    + "        <" + componentName + "\n"
    + (bindings || "") + "\n"
    + "        />\n"
    + "      </section>\n"
    + "    </main>\n"
    + "  );\n}\n";
}
