import type { ComponentPropHint, DiscoveredComponent } from "./components.js";

export type ComponentPropControlKind = "text" | "number" | "boolean" | "enum";

export interface ComponentPropControl {
  kind: ComponentPropControlKind;
  options?: string[];
}

/** Returns the safe editor control for a source-level prop type. Runtime
 * values are never evaluated; only primitive and string-literal-union types
 * receive specialized controls. */
export function componentPropControl(type: string): ComponentPropControl {
  const normalized = type.trim();
  if (normalized === "boolean") return { kind: "boolean" };
  if (normalized === "number") return { kind: "number" };
  const parts = normalized.split("|").map((part) => part.trim()).filter(Boolean);
  if (parts.length >= 2 && parts.every((part) => /^(['"]).*\1$/.test(part))) {
    return { kind: "enum", options: parts.map((part) => part.slice(1, -1)) };
  }
  return { kind: "text" };
}

/** Converts guided palette values into the same deliberately small insertion
 * format used by the inspector (`name=value` plus typed primitive values). */
export function buildComponentPropInput(
  component: Pick<DiscoveredComponent, "propHints">,
  drafts: Record<string, string>,
): string {
  return (component.propHints ?? []).flatMap((hint) => {
    const value = drafts[hint.name]?.trim() ?? "";
    if (!value) return [];
    const control = componentPropControl(hint.type);
    const suffix = control.kind === "number" || control.kind === "boolean" ? `:${control.kind}` : "";
    return [`${hint.name}${suffix}=${value}`];
  }).join("\n");
}

export function missingRequiredComponentProps(
  hints: ComponentPropHint[] | undefined,
  drafts: Record<string, string>,
): string[] {
  return (hints ?? [])
    .filter((hint) => hint.required && !(drafts[hint.name] ?? "").trim())
    .map((hint) => hint.name);
}
