/** Pure comparison helpers for selected-element responsive preview inspections. */

export interface ResponsiveInspection {
  rect: { x: number; y: number; width: number; height: number };
  styles: Record<string, string>;
}

function rounded(value: number): number {
  return Math.round(value * 10) / 10;
}

function deltaLabel(label: string, previous: number, current: number): string | null {
  const delta = rounded(current - previous);
  return delta === 0 ? null : `${label} ${delta > 0 ? "+" : ""}${delta}px`;
}

/** Returns human-readable changes from one rendered breakpoint to the next. */
export function describeResponsiveDiff(
  previous: ResponsiveInspection | null,
  current: ResponsiveInspection | null,
): string[] {
  if (!previous || !current) return [];
  const changes = [
    deltaLabel("x", previous.rect.x, current.rect.x),
    deltaLabel("y", previous.rect.y, current.rect.y),
    deltaLabel("width", previous.rect.width, current.rect.width),
    deltaLabel("height", previous.rect.height, current.rect.height),
  ].filter((value): value is string => value !== null);
  for (const property of ["display", "position", "fontSize", "color", "backgroundColor", "flexDirection", "gridTemplateColumns", "overflowX", "overflowY"]) {
    const before = previous.styles[property] ?? "";
    const after = current.styles[property] ?? "";
    if (before !== after) changes.push(`${property}: ${before || "—"} → ${after || "—"}`);
  }
  return changes;
}
