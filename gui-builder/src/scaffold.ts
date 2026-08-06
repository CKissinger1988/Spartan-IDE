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

/** Generates a small, valid, reviewable TSX component scaffold. */
export function buildComponentScaffold(input: ComponentScaffoldInput): string {
  const componentName = assertIdentifier(input.componentName, "Component name");
  const props = input.props.map((prop) => ({ ...prop, name: assertIdentifier(prop.name, "Prop name") }));
  const uniqueNames = new Set<string>();
  for (const prop of props) {
    if (uniqueNames.has(prop.name)) throw new Error(`Duplicate prop "${prop.name}".`);
    uniqueNames.add(prop.name);
  }
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
