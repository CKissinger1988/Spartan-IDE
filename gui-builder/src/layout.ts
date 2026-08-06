export interface LayoutPreset {
  id: "stack" | "row" | "grid" | "center";
  label: string;
  description: string;
  entries: Record<string, string>;
}

/** Common auto-layout recipes expressed as the same plain inline styles the
 * Design editor already writes. Keeping these in the shared package makes
 * the recipes deterministic and testable instead of hiding them in JSX. */
export const LAYOUT_PRESETS: readonly LayoutPreset[] = [
  {
    id: "stack",
    label: "Stack",
    description: "Vertical flex layout with consistent spacing",
    entries: { display: "flex", flexDirection: "column", gap: "16px", alignItems: "stretch" },
  },
  {
    id: "row",
    label: "Row",
    description: "Horizontal flex layout with centered items",
    entries: { display: "flex", flexDirection: "row", gap: "16px", alignItems: "center" },
  },
  {
    id: "grid",
    label: "Grid",
    description: "Two-column responsive grid with a consistent gap",
    entries: { display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: "16px" },
  },
  {
    id: "center",
    label: "Center",
    description: "Centered flex layout on both axes",
    entries: { display: "flex", justifyContent: "center", alignItems: "center" },
  },
];
