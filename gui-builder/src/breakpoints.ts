/** Project-scoped responsive breakpoint profiles used by the GUI Builder matrix. */

export interface ResponsiveBreakpoint {
  name: string;
  width: number;
  height: number;
}

export interface PreviewBreakpoint extends ResponsiveBreakpoint {
  id: string;
  label: string;
}

const MIN_DIMENSION = 200;
const MAX_DIMENSION = 3000;
const MAX_BREAKPOINTS = 24;

function safeDimension(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  const rounded = Math.round(value);
  return rounded >= MIN_DIMENSION && rounded <= MAX_DIMENSION ? rounded : null;
}

/** Defensively loads user-authored profiles from localStorage-shaped data. */
export function normalizeResponsiveBreakpoints(value: unknown): ResponsiveBreakpoint[] {
  if (!Array.isArray(value)) return [];
  const names = new Set<string>();
  const result: ResponsiveBreakpoint[] = [];
  for (const candidate of value) {
    if (!candidate || typeof candidate !== "object") continue;
    const item = candidate as Record<string, unknown>;
    const name = typeof item.name === "string" ? item.name.trim().slice(0, 80) : "";
    const width = safeDimension(item.width);
    const height = safeDimension(item.height);
    if (!name || names.has(name) || width === null || height === null) continue;
    names.add(name);
    result.push({ name, width, height });
    if (result.length >= MAX_BREAKPOINTS) break;
  }
  return result;
}

/** Combines built-in profiles with project profiles for the responsive matrix. */
export function buildPreviewBreakpoints(
  defaults: readonly ResponsiveBreakpoint[],
  custom: readonly ResponsiveBreakpoint[],
): PreviewBreakpoint[] {
  return [
    ...defaults.map((item) => ({ ...item, id: item.name.toLowerCase(), label: item.name })),
    ...custom.map((item, index) => ({
      ...item,
      id: `custom-${index}-${item.name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "breakpoint"}`,
      label: item.name,
    })),
  ];
}
