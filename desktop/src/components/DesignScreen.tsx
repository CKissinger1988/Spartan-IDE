import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { OpenFile } from "./Editor";

interface StyleEntryValue {
  kind: "literal" | "expression";
  value?: string;
  source?: string;
}

type PropSummary =
  | { kind: "string"; value: string }
  | { kind: "style"; entries: Record<string, StyleEntryValue> }
  | { kind: "expression"; source: string };

interface ComponentNode {
  id: string;
  tagName: string;
  sourceLocation?: {
    startLine: number;
    startColumn: number;
    endLine: number;
    endColumn: number;
  };
  sourceText?: string;
  props: Record<string, PropSummary>;
  children: ComponentNode[];
  textContent: string | null;
}

/** One real component discovered in the project by `gui-builder`'s own
 * `discoverComponents` (task #278). `importFrom` is `null` when the
 * component is declared in the currently-open file, so inserting it needs
 * no import at all. */
interface DiscoveredComponent {
  name: string;
  file: string;
  isDefault: boolean;
  importFrom: string | null;
}

interface DiscoveredAsset {
  file: string;
  relativePath: string;
  referencePath: string;
  kind: "image" | "font";
  label: string;
  fontFaceSnippet?: string;
}

interface DiscoveredToken {
  name: string;
  value: string;
  file: string;
  relativePath: string;
}

interface VariantPreset {
  name: string;
  source: string;
  updatedAt: number;
}

type PreviewInteractionState = "normal" | "focus" | "hover" | "active";

interface InteractionPreset {
  name: string;
  state: PreviewInteractionState;
  updatedAt: number;
}

interface PreviewInspection {
  nodeId: string;
  rect: { width: number; height: number };
  styles: Record<string, string>;
}

interface StyleClipboard {
  sourcePath: string;
  sourceTagName: string;
  entries: Record<string, StyleEntryValue>;
}

interface PropClipboardEntry {
  kind: "string" | "expression";
  value?: string;
  source?: string;
}

interface PropClipboard {
  sourcePath: string;
  sourceTagName: string;
  entries: Record<string, PropClipboardEntry>;
}

interface DesignScreenProps {
  activeFile: OpenFile | null;
  openFiles: OpenFile[];
  onOpenFile: (path: string) => void;
  onRevealSource: (path: string, line: number, character: number) => void;
  onContentChange: (path: string, content: string, saved?: boolean) => void;
  /** Real project root, used to scan for the component palette. Absent
   * means the palette simply isn't offered -- there's nothing honest to
   * scan without it. */
  projectRoot?: string;
}

const DESIGN_VIEWPORTS = [
  { id: "desktop", label: "Desktop", width: 1280, height: 800 },
  { id: "tablet", label: "Tablet", width: 768, height: 1024 },
  { id: "mobile", label: "Mobile", width: 390, height: 844 },
] as const;
type DesignViewportId = (typeof DESIGN_VIEWPORTS)[number]["id"] | "custom";
type ComponentInsertPlacement = "child" | "sibling";

interface ViewportPreset {
  name: string;
  width: number;
  height: number;
}

function isComponentFile(path: string): boolean {
  return path.endsWith(".jsx") || path.endsWith(".tsx");
}

function isValidTagName(name: string): boolean {
  const parts = name.trim().split(".");
  return parts.length > 0 && parts.every((part) => /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(part));
}

function searchableNodeText(node: ComponentNode): string {
  const propText = Object.entries(node.props).flatMap(([name, summary]) => {
    if (summary.kind === "string") return [name, summary.value];
    if (summary.kind === "expression") return [name, summary.source];
    return [name, ...Object.keys(summary.entries)];
  });
  return [node.id, node.tagName, node.textContent ?? "", ...propText].join(" ").toLowerCase();
}

/** Keeps a matching node and every ancestor needed to reach it. The returned
 * nodes are shallow views with filtered child arrays; the real source-backed
 * node objects and ids remain untouched for selection/edit operations. */
function filterTree(nodes: ComponentNode[], query: string): ComponentNode[] {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return nodes;
  return nodes.flatMap((node) => {
    const children = filterTree(node.children, normalized);
    return searchableNodeText(node).includes(normalized) || children.length > 0
      ? [{ ...node, children }]
      : [];
  });
}

function TreeNode({
  node,
  depth,
  selectedIds,
  onSelect,
  filterActive,
}: {
  node: ComponentNode;
  depth: number;
  selectedIds: string[];
  onSelect: (id: string, additive: boolean) => void;
  filterActive: boolean;
}): React.ReactElement {
  const [expanded, setExpanded] = useState(true);
  const hasChildren = node.children.length > 0;
  const visibleExpanded = filterActive || expanded;
  return (
    <div>
      <div
        className={`design-tree-row ${selectedIds.includes(node.id) ? "design-tree-row-active" : ""}`}
        style={{ paddingLeft: 8 + depth * 14 }}
        role="treeitem"
        tabIndex={0}
        aria-selected={selectedIds.includes(node.id)}
        aria-expanded={hasChildren ? visibleExpanded : undefined}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            onSelect(node.id, event.shiftKey);
          } else if (event.key === "ArrowRight" && hasChildren) {
            event.preventDefault();
            if (!visibleExpanded && !filterActive) setExpanded(true);
          } else if (event.key === "ArrowLeft" && hasChildren) {
            event.preventDefault();
            if (visibleExpanded && !filterActive) setExpanded(false);
          }
        }}
        onClick={(event) => onSelect(node.id, event.shiftKey)}
      >
        <button
          type="button"
          className="design-tree-toggle"
          aria-label={hasChildren ? `${visibleExpanded ? "Collapse" : "Expand"} ${node.tagName}` : `${node.tagName} has no children`}
          aria-expanded={hasChildren ? visibleExpanded : undefined}
          disabled={!hasChildren || filterActive}
          onClick={(event) => {
            event.stopPropagation();
            if (hasChildren) setExpanded((value) => !value);
          }}
        >
          {hasChildren ? (visibleExpanded ? "▾" : "▸") : "·"}
        </button>
        <span className="mono">
          &lt;{node.tagName}&gt; <span className="design-tree-id">#{node.id}</span>
        </span>
      </div>
      {visibleExpanded && node.children.length > 0 && (
        <div role="group">
          {node.children.map((child) => (
            <TreeNode key={child.id} node={child} depth={depth + 1} selectedIds={selectedIds} onSelect={onSelect} filterActive={filterActive} />
          ))}
        </div>
      )}
    </div>
  );
}

function findNode(roots: ComponentNode[], id: string): ComponentNode | null {
  for (const root of roots) {
    if (root.id === id) return root;
    const found = findNode(root.children, id);
    if (found) return found;
  }
  return null;
}

function findParentEntry(roots: ComponentNode[], id: string): { parent: ComponentNode; index: number } | null {
  for (const root of roots) {
    const index = root.children.findIndex((child) => child.id === id);
    if (index !== -1) return { parent: root, index };
    const nested = findParentEntry(root.children, id);
    if (nested) return nested;
  }
  return null;
}

/** Flattens the real tree into one list for the Reparent/Insert target
 * dropdowns -- both operations need to name a node that isn't necessarily
 * the currently-selected one, so a simple id+tagName picker is the real,
 * minimal v1 UI for it (matching this form's own established "narrow,
 * functional, not fancy" style, same as the existing Prop/Style radio
 * pair). */
function flattenNodes(roots: ComponentNode[], depth = 0): { id: string; tagName: string; depth: number }[] {
  return roots.flatMap((node) => [
    { id: node.id, tagName: node.tagName, depth },
    ...flattenNodes(node.children, depth + 1),
  ]);
}

type InsertPropValueType = "number" | "boolean" | "expression";

interface ParsedInsertProps {
  props: Record<string, string>;
  propValues: Record<string, { value: string; valueType: InsertPropValueType }>;
}

/** Parses the inspector's deliberately small, readable prop format:
 * `name=value` creates a string prop, while `name:number=3`,
 * `name:boolean=true`, and `name:expression=handleClick` create typed JSX
 * expression values. Braced expressions are accepted too, so
 * `onClick:expression={handleClick}` is convenient in the form. */
function parseInsertProps(input: string): ParsedInsertProps {
  const props: Record<string, string> = {};
  const propValues: ParsedInsertProps["propValues"] = {};
  for (const [index, rawLine] of input.split("\n").entries()) {
    const line = rawLine.trim();
    if (!line) continue;
    const separator = line.indexOf("=");
    if (separator <= 0) {
      throw new Error(`Inserted prop line ${index + 1} must use name=value format.`);
    }
    const rawName = line.slice(0, separator).trim();
    const typedMatch = rawName.match(/^(.*):(number|boolean|expression)$/);
    const name = typedMatch?.[1]?.trim() ?? rawName;
    if (!/^[A-Za-z_:][A-Za-z0-9:._-]*$/.test(name)) {
      throw new Error(`Inserted prop "${name}" must be a valid JSX attribute name.`);
    }
    const value = line.slice(separator + 1).trim();
    if (typedMatch) {
      const valueType = typedMatch[2] as InsertPropValueType;
      const expressionValue = valueType === "expression" && value.startsWith("{") && value.endsWith("}")
        ? value.slice(1, -1).trim()
        : value;
      propValues[name] = { value: expressionValue, valueType };
    } else {
      props[name] = value;
    }
  }
  return { props, propValues };
}

function cssPropertyName(name: string): string {
  return name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
}

function inspectionCssSnapshot(inspection: PreviewInspection, tagName: string): string {
  const lines = Object.entries(inspection.styles)
    .filter(([, value]) => value)
    .map(([name, value]) => `  ${cssPropertyName(name)}: ${value};`);
  lines.push(`  width: ${Math.round(inspection.rect.width)}px;`);
  lines.push(`  height: ${Math.round(inspection.rect.height)}px;`);
  return `/* Rendered <${tagName}> ${inspection.nodeId} snapshot */\n${tagName} {\n${lines.join("\n")}\n}`;
}

type AccessibilitySeverity = "error" | "warning" | "pass" | "info";

interface AccessibilityFinding {
  severity: AccessibilitySeverity;
  message: string;
}

function propSummaryValue(node: ComponentNode, name: string): string | null {
  const summary = node.props[name];
  if (!summary) return null;
  if (summary.kind === "string") return summary.value.trim();
  if (summary.kind === "expression") return summary.source.trim();
  return null;
}

function parseCssColor(value: string): [number, number, number] | null {
  const normalized = value.trim().toLowerCase();
  const hex = normalized.match(/^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/);
  if (hex) {
    const digits = hex[1];
    const expanded = digits.length <= 4 ? digits.split("").map((digit) => digit + digit).join("") : digits;
    if (expanded.length === 8 && Number.parseInt(expanded.slice(6), 16) === 0) return null;
    return [0, 2, 4].map((offset) => Number.parseInt(expanded.slice(offset, offset + 2), 16)) as [number, number, number];
  }
  const rgb = normalized.match(/^rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)(?:[,\s/]+([\d.]+%?))?\s*\)$/);
  if (!rgb) return null;
  const alpha = rgb[4]?.endsWith("%") ? Number.parseFloat(rgb[4]) / 100 : rgb[4] === undefined ? 1 : Number.parseFloat(rgb[4]);
  if (!Number.isFinite(alpha) || alpha === 0) return null;
  return [1, 2, 3].map((index) => Math.max(0, Math.min(255, Number.parseFloat(rgb[index])))) as [number, number, number];
}

function relativeLuminance(color: [number, number, number]): number {
  const channels = color.map((channel) => channel / 255).map((channel) => channel <= 0.03928 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4);
  return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
}

function contrastRatio(foreground: [number, number, number], background: [number, number, number]): number {
  const light = Math.max(relativeLuminance(foreground), relativeLuminance(background));
  const dark = Math.min(relativeLuminance(foreground), relativeLuminance(background));
  return (light + 0.05) / (dark + 0.05);
}

function accessibilityAudit(node: ComponentNode, inspection: PreviewInspection | null): AccessibilityFinding[] {
  const findings: AccessibilityFinding[] = [];
  const tagName = node.tagName.toLowerCase();
  if (tagName === "img") {
    const alt = propSummaryValue(node, "alt");
    findings.push(alt ? { severity: "pass", message: "Image has an alt value." } : { severity: "error", message: "Image is missing a non-empty alt prop." });
  }
  const interactive = new Set(["a", "button", "input", "select", "textarea"]);
  if (interactive.has(tagName)) {
    const accessibleName = propSummaryValue(node, "aria-label") || propSummaryValue(node, "title") || node.textContent?.trim();
    findings.push(accessibleName
      ? { severity: "pass", message: "Interactive element has a detectable accessible name." }
      : { severity: "error", message: "Interactive element has no text, aria-label, or title." });
    if (inspection) {
      const tooSmall = inspection.rect.width < 44 || inspection.rect.height < 44;
      findings.push(tooSmall
        ? { severity: "warning", message: `Interactive target is ${Math.round(inspection.rect.width)}×${Math.round(inspection.rect.height)}px; aim for at least 44×44px.` }
        : { severity: "pass", message: "Interactive target meets the 44×44px minimum size." });
    }
  }
  if (inspection) {
    const foreground = parseCssColor(inspection.styles.color);
    const background = parseCssColor(inspection.styles.backgroundColor);
    if (foreground && background) {
      const ratio = contrastRatio(foreground, background);
      const fontSize = Number.parseFloat(inspection.styles.fontSize);
      const fontWeight = Number.parseInt(inspection.styles.fontWeight || "400", 10);
      const largeText = fontSize >= 18 || (fontSize >= 14 && fontWeight >= 700);
      const required = largeText ? 3 : 4.5;
      findings.push(ratio >= required
        ? { severity: "pass", message: `Text contrast is ${ratio.toFixed(2)}:1 (minimum ${required}:1).` }
        : { severity: "error", message: `Text contrast is ${ratio.toFixed(2)}:1 (minimum ${required}:1).` });
    } else {
      findings.push({ severity: "info", message: "Contrast could not be computed from the selected element's direct colors." });
    }
  }
  return findings;
}

/** What kind of real input widget a curated style property deserves.
 * `text` is the honest fallback for anything whose real value space is
 * too open to constrain (a shadow, a transform, a font stack). */
type StyleControlKind = "color" | "length" | "select" | "text";

interface StylePropertyDef {
  /** The real JSX style key (camelCase) -- exactly the string a
   * `StyleChange` edit writes, so this catalog never needs a separate
   * name-translation step. */
  name: string;
  label: string;
  group: string;
  control: StyleControlKind;
  /** Only for `control: "select"` -- the real allowed CSS keywords. */
  options?: string[];
}

/** A deliberately curated set, not an exhaustive CSS property list: the
 * properties a real component author actually reaches for, each paired
 * with a widget that matches its real value space. Anything absent is
 * still reachable through the "Custom…" escape hatch, so this catalog
 * narrows the common case without ever removing capability the raw
 * key/value form already had. */
const STYLE_PROPERTIES: StylePropertyDef[] = [
  { name: "color", label: "Text color", group: "Color", control: "color" },
  { name: "backgroundColor", label: "Background", group: "Color", control: "color" },
  { name: "borderColor", label: "Border color", group: "Color", control: "color" },
  { name: "fontSize", label: "Font size", group: "Typography", control: "length" },
  {
    name: "fontWeight",
    label: "Font weight",
    group: "Typography",
    control: "select",
    options: ["normal", "bold", "100", "200", "300", "400", "500", "600", "700", "800", "900"],
  },
  { name: "fontFamily", label: "Font family", group: "Typography", control: "text" },
  { name: "lineHeight", label: "Line height", group: "Typography", control: "text" },
  { name: "letterSpacing", label: "Letter spacing", group: "Typography", control: "length" },
  {
    name: "textAlign",
    label: "Text align",
    group: "Typography",
    control: "select",
    options: ["left", "center", "right", "justify"],
  },
  { name: "padding", label: "Padding", group: "Spacing", control: "length" },
  { name: "paddingTop", label: "Padding top", group: "Spacing", control: "length" },
  { name: "paddingRight", label: "Padding right", group: "Spacing", control: "length" },
  { name: "paddingBottom", label: "Padding bottom", group: "Spacing", control: "length" },
  { name: "paddingLeft", label: "Padding left", group: "Spacing", control: "length" },
  { name: "margin", label: "Margin", group: "Spacing", control: "length" },
  { name: "marginTop", label: "Margin top", group: "Spacing", control: "length" },
  { name: "marginRight", label: "Margin right", group: "Spacing", control: "length" },
  { name: "marginBottom", label: "Margin bottom", group: "Spacing", control: "length" },
  { name: "marginLeft", label: "Margin left", group: "Spacing", control: "length" },
  { name: "gap", label: "Gap", group: "Spacing", control: "length" },
  { name: "width", label: "Width", group: "Layout", control: "length" },
  { name: "minWidth", label: "Min width", group: "Layout", control: "length" },
  { name: "height", label: "Height", group: "Layout", control: "length" },
  { name: "maxWidth", label: "Max width", group: "Layout", control: "length" },
  { name: "minHeight", label: "Min height", group: "Layout", control: "length" },
  { name: "maxHeight", label: "Max height", group: "Layout", control: "length" },
  {
    name: "display",
    label: "Display",
    group: "Layout",
    control: "select",
    options: ["block", "inline", "inline-block", "flex", "inline-flex", "grid", "none"],
  },
  {
    name: "flexDirection",
    label: "Flex direction",
    group: "Layout",
    control: "select",
    options: ["row", "row-reverse", "column", "column-reverse"],
  },
  {
    name: "justifyContent",
    label: "Justify content",
    group: "Layout",
    control: "select",
    options: ["flex-start", "center", "flex-end", "space-between", "space-around", "space-evenly"],
  },
  {
    name: "alignItems",
    label: "Align items",
    group: "Layout",
    control: "select",
    options: ["flex-start", "center", "flex-end", "stretch", "baseline"],
  },
  {
    name: "flexWrap",
    label: "Flex wrap",
    group: "Layout",
    control: "select",
    options: ["nowrap", "wrap", "wrap-reverse"],
  },
  {
    name: "alignContent",
    label: "Align content",
    group: "Layout",
    control: "select",
    options: ["flex-start", "center", "flex-end", "space-between", "space-around", "stretch"],
  },
  { name: "flex", label: "Flex", group: "Layout", control: "text" },
  { name: "alignSelf", label: "Align self", group: "Layout", control: "select", options: ["auto", "flex-start", "center", "flex-end", "stretch", "baseline"] },
  { name: "order", label: "Order", group: "Layout", control: "text" },
  { name: "gridTemplateColumns", label: "Grid columns", group: "Layout", control: "text" },
  { name: "gridTemplateRows", label: "Grid rows", group: "Layout", control: "text" },
  { name: "gridColumn", label: "Grid column", group: "Layout", control: "text" },
  { name: "gridRow", label: "Grid row", group: "Layout", control: "text" },
  {
    name: "position",
    label: "Position",
    group: "Position",
    control: "select",
    options: ["static", "relative", "absolute", "fixed", "sticky"],
  },
  { name: "top", label: "Top", group: "Position", control: "length" },
  { name: "right", label: "Right", group: "Position", control: "length" },
  { name: "bottom", label: "Bottom", group: "Position", control: "length" },
  { name: "left", label: "Left", group: "Position", control: "length" },
  { name: "zIndex", label: "Z-index", group: "Position", control: "text" },
  { name: "borderRadius", label: "Border radius", group: "Border", control: "length" },
  { name: "borderWidth", label: "Border width", group: "Border", control: "length" },
  {
    name: "borderStyle",
    label: "Border style",
    group: "Border",
    control: "select",
    options: ["none", "solid", "dashed", "dotted", "double"],
  },
  { name: "opacity", label: "Opacity", group: "Effects", control: "text" },
  { name: "boxShadow", label: "Box shadow", group: "Effects", control: "text" },
  { name: "backgroundImage", label: "Background image", group: "Effects", control: "text" },
  { name: "transform", label: "Transform", group: "Effects", control: "text" },
  { name: "transition", label: "Transition", group: "Effects", control: "text" },
  {
    name: "overflow",
    label: "Overflow",
    group: "Effects",
    control: "select",
    options: ["visible", "hidden", "clip", "scroll", "auto"],
  },
  {
    name: "cursor",
    label: "Cursor",
    group: "Effects",
    control: "select",
    options: ["default", "pointer", "text", "move", "not-allowed", "grab", "none"],
  },
];

/** Sentinel for the "Custom…" dropdown entry. Deliberately a string no
 * real CSS property can collide with, so `styleDef` lookup stays a plain
 * name match with no separate "is this custom" flag to keep in sync. */
const CUSTOM_STYLE_PROPERTY = "__custom__";
const CUSTOM_PROP = "__spartan_custom_prop__";

/** The catalog grouped for `<optgroup>`, preserving each group's first
 * appearance order in `STYLE_PROPERTIES` rather than sorting -- the
 * catalog is already written in a deliberate order (color, then type,
 * then spacing, then layout) that alphabetizing would scramble. */
const STYLE_GROUPS: [string, StylePropertyDef[]][] = STYLE_PROPERTIES.reduce(
  (groups, def) => {
    const existing = groups.find(([name]) => name === def.group);
    if (existing) existing[1].push(def);
    else groups.push([def.group, [def]]);
    return groups;
  },
  [] as [string, StylePropertyDef[]][]
);

/** The real units the length control offers. `""` is a genuine, distinct
 * option -- a unitless value is meaningful for some properties (a raw
 * `lineHeight`, a `0`), so it can't just be modelled as "no unit chosen
 * yet". */
const LENGTH_UNITS = ["px", "rem", "em", "%", "vh", "vw", ""];

/** Splits a real CSS length into its number and unit for the number+unit
 * control. A keyword (`auto`, `inherit`, `var(--x)`) has no number to
 * show, so it comes back with an empty number and the whole string as
 * the unit -- which the control renders as a plain, unmodified value
 * rather than pretending it's a measurable length. */
function parseLength(value: string): { num: string; unit: string } {
  const trimmed = value.trim();
  if (trimmed === "") return { num: "", unit: "px" };
  const match = /^(-?\d*\.?\d+)\s*([a-z%]*)$/i.exec(trimmed);
  if (!match) return { num: "", unit: trimmed };
  return { num: match[1], unit: match[2] };
}

/** Normalizes a real style value to something `<input type="color">`
 * will actually accept (it only ever takes a 7-char `#rrggbb`), or
 * `null` when the value genuinely isn't a plain hex -- a named color, a
 * `var(--token)`, an `rgba(...)`, or nothing at all. Returning `null`
 * rather than a silent fallback is what lets the color control keep the
 * real text field authoritative and only offer the swatch when it can
 * faithfully round-trip the value. */
function toColorInputValue(value: string): string | null {
  const trimmed = value.trim();
  const short = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/i.exec(trimmed);
  if (short) return `#${short[1]}${short[1]}${short[2]}${short[2]}${short[3]}${short[3]}`.toLowerCase();
  const full = /^#([0-9a-f]{6})$/i.exec(trimmed);
  if (full) return `#${full[1]}`.toLowerCase();
  return null;
}

/** Reads the selected node's own real current value for a style property
 * out of the already-parsed tree, so picking a property seeds the control
 * with what's actually in the source instead of an empty field. An
 * `expression` entry (e.g. `color: C.text`) has no literal to seed from
 * and deliberately returns `null` -- overwriting a real design-token
 * reference with its resolved literal would be a silent, lossy edit. */
function currentStyleValue(node: ComponentNode | null, property: string): string | null {
  if (!node) return null;
  const style = node.props.style;
  if (!style || style.kind !== "style") return null;
  const entry = style.entries[property];
  if (!entry || entry.kind !== "literal") return null;
  return entry.value ?? null;
}

/** The real, property-appropriate value widget for a curated style
 * property. Every kind writes back through the same single `onChange`
 * string, so the surrounding form (and the `StyleChange` edit it builds)
 * stays completely unaware of which widget produced the value.
 *
 * The color case deliberately renders *both* a swatch and a text field:
 * `<input type="color">` can only ever hold a plain `#rrggbb`, so a real
 * `var(--accent)` / `transparent` / `rgba(...)` value would be silently
 * destroyed if the swatch were the only control. The text field stays
 * authoritative; the swatch is an additive convenience that only appears
 * to agree with it when the value is genuinely a hex. */
function StyleValueControl({
  def,
  value,
  onChange,
}: {
  def: StylePropertyDef;
  value: string;
  onChange: (next: string) => void;
}): React.ReactElement {
  if (def.control === "color") {
    const swatch = toColorInputValue(value);
    return (
      <div className="design-control-row">
        <input
          type="color"
          className="design-color-swatch"
          aria-label={`${def.label} color picker`}
          value={swatch ?? "#000000"}
          onChange={(e) => onChange(e.target.value)}
        />
        <input
          className="design-input mono design-control-grow"
          placeholder="#rrggbb, var(--token), transparent…"
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
      </div>
    );
  }

  if (def.control === "length") {
    const { num, unit } = parseLength(value);
    const known = LENGTH_UNITS.includes(unit);
    return (
      <div className="design-control-row">
        <input
          type="number"
          className="design-input mono design-control-num"
          aria-label={`${def.label} amount`}
          placeholder="0"
          value={num}
          onChange={(e) => onChange(e.target.value === "" ? "" : `${e.target.value}${known ? unit : "px"}`)}
        />
        <select
          className="design-input mono design-control-unit"
          aria-label={`${def.label} unit`}
          value={known ? unit : "px"}
          onChange={(e) => onChange(num === "" ? "" : `${num}${e.target.value}`)}
        >
          {LENGTH_UNITS.map((u) => (
            <option key={u || "none"} value={u}>
              {u || "(none)"}
            </option>
          ))}
        </select>
        {/* A keyword the number+unit pair genuinely can't express
            (`auto`, `inherit`, a `var(--x)`) is shown as-is and stays
            editable, rather than being silently dropped. */}
        {!known && unit !== "" && (
          <input
            className="design-input mono design-control-grow"
            aria-label={`${def.label} keyword`}
            value={value}
            onChange={(e) => onChange(e.target.value)}
          />
        )}
      </div>
    );
  }

  if (def.control === "select") {
    const options = def.options ?? [];
    const isKnown = options.includes(value);
    return (
      <div className="design-control-row">
        <select
          className="design-input mono design-control-grow"
          aria-label={def.label}
          value={isKnown ? value : ""}
          onChange={(e) => onChange(e.target.value)}
        >
          <option value="">Select a value…</option>
          {options.map((o) => (
            <option key={o} value={o}>
              {o}
            </option>
          ))}
        </select>
      </div>
    );
  }

  return (
    <input
      className="design-input mono"
      placeholder="value"
      aria-label={def.label}
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}

/**
 * Real, working GUI Builder + live preview screen (§75.62,
 * user-requested: "the visual GUI Builder and live app preview are
 * mandatory"). Drives the already-real, already-tested `gui-builder/`
 * npm project (§75.38-§75.53) via three real IPC calls
 * (`design_parse`/`design_bundle`/`design_apply_edit`) -- no new AST/
 * bundling logic here, only the real UI wiring that project never had
 * until now.
 *
 * The live preview is a real, sandboxed iframe (`sandbox="allow-scripts"`,
 * deliberately no `allow-same-origin`, matching the exact security
 * posture `webview_bridge.rs` established in the original wgpu shell)
 * showing `gui-builder`'s own real esbuild bundle output, which already
 * includes a real click-to-select `postMessage` relay
 * (`data-spartan-id` + a delegated click listener, see `bundle.ts`'s own
 * doc comment) and a persistent selection outline -- this component just
 * listens for that message and routes it through the same `selectedId` state
 * a tree-row click uses, then sends tree selections back to the iframe so a
 * canvas click and a tree click are visually indistinguishable.
 *
 * `parse`/`bundle` both read from disk (matching the CLI's own
 * documented v1 contract); `apply` reads the real live, possibly-unsaved
 * buffer from `activeFile.content` and its result is fed back through
 * the exact same `edit` IPC call typing already uses, so a canvas edit
 * gets the same undo/dirty tracking as any other edit.
 *
 * All twenty real `CanvasEdit` kinds `gui-builder` itself supports are now
 * wired here: `PropChange`/`StyleChange` (mutate the selected node) and
 * `Reparent`/`ComponentInsert` (structural edits, closing the gap this
 * screen's own edit form used to leave unreachable even after
 * `gui-builder`'s own backend implemented them). Both structural kinds
 * reuse the same real `design_apply_edit` IPC call and the same
 * post-edit `refresh()` -- no new IPC method was needed, only new form
 * state naming a *second* node (the target parent) beyond the tree's
 * existing single-selection state.
 */
export default function DesignScreen({
  activeFile,
  openFiles,
  onOpenFile,
  onRevealSource,
  onContentChange,
  projectRoot,
}: DesignScreenProps): React.ReactElement {
  const [roots, setRoots] = useState<ComponentNode[]>([]);
  const [treeFilter, setTreeFilter] = useState("");
  const [bundleCode, setBundleCode] = useState<string | null>(null);
  const [previewSource, setPreviewSource] = useState<string | null>(null);
  const [variantName, setVariantName] = useState("");
  const [variantPresets, setVariantPresets] = useState<VariantPreset[]>([]);
  const [interactionPresetName, setInteractionPresetName] = useState("");
  const [interactionPresets, setInteractionPresets] = useState<InteractionPreset[]>([]);
  const [previewInteractionState, setPreviewInteractionState] = useState<PreviewInteractionState>("normal");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [styleClipboard, setStyleClipboard] = useState<StyleClipboard | null>(null);
  const [propClipboard, setPropClipboard] = useState<PropClipboard | null>(null);
  const [previewInspection, setPreviewInspection] = useState<PreviewInspection | null>(null);
  const [boxModelVisible, setBoxModelVisible] = useState(false);
  const [copiedInspection, setCopiedInspection] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [propKey, setPropKey] = useState("");
  const [propValue, setPropValue] = useState("");
  const [textValue, setTextValue] = useState("");
  const [propValueType, setPropValueType] = useState<"string" | "number" | "boolean" | "expression">("string");
  const [editKind, setEditKind] = useState<"PropChange" | "PropRemove" | "StyleChange" | "StyleRemove" | "StyleClear" | "TextChange" | "TagChange" | "Wrap" | "Unwrap" | "Reparent" | "ComponentInsert" | "ComponentInsertSibling">(
    "PropChange"
  );
  const [tagName, setTagName] = useState("");
  const [wrapTagName, setWrapTagName] = useState("");
  const [reparentTargetId, setReparentTargetId] = useState("");
  const [insertTagName, setInsertTagName] = useState("");
  const [insertProps, setInsertProps] = useState("");
  const [insertText, setInsertText] = useState("");
  const [palette, setPalette] = useState<DiscoveredComponent[]>([]);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteInsertPlacement, setPaletteInsertPlacement] = useState<ComponentInsertPlacement>("child");
  const [paletteFilter, setPaletteFilter] = useState("");
  const [assets, setAssets] = useState<DiscoveredAsset[]>([]);
  const [assetsOpen, setAssetsOpen] = useState(false);
  const [assetFilter, setAssetFilter] = useState("");
  const [copiedAsset, setCopiedAsset] = useState<string | null>(null);
  const [copiedFontCss, setCopiedFontCss] = useState<string | null>(null);
  const [tokens, setTokens] = useState<DiscoveredToken[]>([]);
  const [tokenDrafts, setTokenDrafts] = useState<Record<string, string>>({});
  const [tokensOpen, setTokensOpen] = useState(false);
  const [tokenFilter, setTokenFilter] = useState("");
  const [copiedToken, setCopiedToken] = useState<string | null>(null);
  const [newTokenName, setNewTokenName] = useState("");
  const [newTokenValue, setNewTokenValue] = useState("");
  const [newTokenFile, setNewTokenFile] = useState("");
  const [viewportId, setViewportId] = useState<DesignViewportId>("desktop");
  const [customViewportWidth, setCustomViewportWidth] = useState(1024);
  const [customViewportHeight, setCustomViewportHeight] = useState(768);
  const [viewportPresetName, setViewportPresetName] = useState("");
  const [viewportPresets, setViewportPresets] = useState<ViewportPreset[]>([]);
  const [previewZoom, setPreviewZoom] = useState(75);
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const refreshGenerationRef = useRef(0);
  const viewportStorageKey = projectRoot ? `spartan.gui-builder.viewports:${projectRoot}` : null;
  const viewport = viewportId === "custom"
    ? { id: "custom", label: "Custom", width: customViewportWidth, height: customViewportHeight }
    : DESIGN_VIEWPORTS.find((item) => item.id === viewportId) ?? DESIGN_VIEWPORTS[0];

  // Declared up here, not down beside the other render-time derivations,
  // because `selectStyleProperty`'s own `useCallback` dependency array
  // reads it -- a `const` referenced before its declaration is a real
  // TDZ ReferenceError at first render, not a hoisting no-op.
  const selectedNode = selectedId ? findNode(roots, selectedId) : null;
  const accessibilityFindings = useMemo(
    () => selectedNode ? accessibilityAudit(selectedNode, previewInspection?.nodeId === selectedNode.id ? previewInspection : null) : [],
    [selectedNode, previewInspection]
  );

  const saveViewportPreset = useCallback(() => {
    if (!viewportStorageKey || !viewportPresetName.trim()) return;
    const preset: ViewportPreset = {
      name: viewportPresetName.trim(),
      width: customViewportWidth,
      height: customViewportHeight,
    };
    const next = [preset, ...viewportPresets.filter((item) => item.name !== preset.name)];
    setViewportPresets(next);
    setViewportPresetName("");
    try {
      window.localStorage.setItem(viewportStorageKey, JSON.stringify(next));
    } catch (e) {
      setError(`Could not save viewport preset: ${(e as Error).message}`);
    }
  }, [viewportStorageKey, viewportPresetName, customViewportWidth, customViewportHeight, viewportPresets]);

  const applyViewportPreset = useCallback((preset: ViewportPreset) => {
    setViewportId("custom");
    setCustomViewportWidth(preset.width);
    setCustomViewportHeight(preset.height);
  }, []);

  const deleteViewportPreset = useCallback((name: string) => {
    if (!viewportStorageKey) return;
    const next = viewportPresets.filter((preset) => preset.name !== name);
    setViewportPresets(next);
    try {
      window.localStorage.setItem(viewportStorageKey, JSON.stringify(next));
    } catch (e) {
      setError(`Could not delete viewport preset: ${(e as Error).message}`);
    }
  }, [viewportStorageKey, viewportPresets]);
  const selectionCount = selectedIds.length;
  const hasSingleSelection = selectionCount === 1;
  const selectedSibling = selectedId && hasSingleSelection ? findParentEntry(roots, selectedId) : null;
  const canUnwrap = hasSingleSelection && !!selectedSibling && !!selectedNode && Object.keys(selectedNode.props).length === 0;
  const selectedParentEntries = selectedIds.map((id) => findParentEntry(roots, id));
  const firstSelectedParentId = selectedParentEntries[0]?.parent.id;
  const canWrapSelection = selectedParentEntries.length > 0
    && !!firstSelectedParentId
    && selectedParentEntries.every((entry) => entry?.parent.id === firstSelectedParentId);
  const canReorderSelection = selectedParentEntries.length > 0
    && !!firstSelectedParentId
    && selectedParentEntries.every((entry) => entry?.parent.id === firstSelectedParentId);
  const canMoveSelectionUp = canReorderSelection && selectedParentEntries.some(
    (entry) => !!entry && entry.index > 0 && !selectedIds.includes(entry.parent.children[entry.index - 1].id),
  );
  const canMoveSelectionDown = canReorderSelection && selectedParentEntries.some(
    (entry) => !!entry && entry.index < entry.parent.children.length - 1 && !selectedIds.includes(entry.parent.children[entry.index + 1].id),
  );
  const selectedStyleNodes = selectedIds.map((id) => findNode(roots, id));
  const canClearSelectionStyles = selectedStyleNodes.length > 0
    && selectedStyleNodes.every((node) => node?.props.style?.kind === "style");
  const filteredRoots = useMemo(() => filterTree(roots, treeFilter), [roots, treeFilter]);
  const filteredPalette = useMemo(() => {
    const query = paletteFilter.trim().toLowerCase();
    if (!query) return palette;
    return palette.filter((component) => `${component.name} ${component.file} ${component.importFrom ?? ""}`.toLowerCase().includes(query));
  }, [palette, paletteFilter]);
  const filteredAssets = useMemo(() => {
    const query = assetFilter.trim().toLowerCase();
    if (!query) return assets;
    return assets.filter((asset) => `${asset.label} ${asset.file} ${asset.relativePath} ${asset.referencePath}`.toLowerCase().includes(query));
  }, [assets, assetFilter]);
  const filteredTokens = useMemo(() => {
    const query = tokenFilter.trim().toLowerCase();
    if (!query) return tokens;
    return tokens.filter((token) => `${token.name} ${token.value} ${token.file} ${token.relativePath}`.toLowerCase().includes(query));
  }, [tokens, tokenFilter]);
  const openCssFiles = useMemo(
    () => openFiles.filter((file) => /\.(css|scss|sass|less)$/i.test(file.path)),
    [openFiles]
  );

  useEffect(() => {
    setTextValue(selectedNode?.textContent ?? "");
  }, [selectedId, selectedNode?.textContent]);

  // Keep the real visual canvas selection highlight synchronized when the
  // tree selects a node, and when a fresh bundle replaces the iframe DOM.
  useEffect(() => {
    setPreviewInspection(null);
    setBoxModelVisible(false);
    setPreviewInteractionState("normal");
    iframeRef.current?.contentWindow?.postMessage(
      { type: "spartan-canvas-select", nodeId: selectedId, nodeIds: selectedIds },
      "*",
    );
    iframeRef.current?.contentWindow?.postMessage(
      { type: "spartan-canvas-state", nodeId: selectedId, state: null },
      "*",
    );
    iframeRef.current?.contentWindow?.postMessage(
      { type: "spartan-canvas-box-model", nodeId: selectedId, visible: false },
      "*",
    );
    if (selectedId) {
      iframeRef.current?.contentWindow?.postMessage(
        { type: "spartan-canvas-inspect", nodeId: selectedId },
        "*",
      );
    }
  }, [selectedId, selectedIds, bundleCode]);

  const selectNodes = useCallback((id: string, additive: boolean) => {
    if (!additive) {
      setSelectedIds([id]);
      return;
    }
    setSelectedIds((current) => {
      const next = current.includes(id) ? current.filter((value) => value !== id) : [...current, id];
      return next;
    });
  }, []);

  useEffect(() => {
    setSelectedId(selectedIds[selectedIds.length - 1] ?? null);
  }, [selectedIds]);

  // The curated definition for whatever style property is currently
  // named, or `undefined` for the Custom… path -- derived from `propKey`
  // rather than held as separate state, so the two can never disagree
  // (e.g. after a Custom… entry happens to be typed as a real curated
  // name, which correctly upgrades it to the typed control).
  const styleDef = STYLE_PROPERTIES.find((d) => d.name === propKey);

  const refresh = useCallback(async (path: string, source?: string) => {
    const generation = ++refreshGenerationRef.current;
    setError(null);
    try {
      const parseResult = (await window.spartan.call(
        source === undefined ? "design_parse" : "design_parse_source",
        source === undefined ? { path } : { path, source },
      )) as {
        roots: ComponentNode[];
      };
      if (generation !== refreshGenerationRef.current) return;
      setRoots(parseResult.roots);
    } catch (e) {
      if (generation !== refreshGenerationRef.current) return;
      setError((e as Error).message);
    }
    if (generation !== refreshGenerationRef.current) return;
    try {
      const bundleResult = (await window.spartan.call(
        source === undefined ? "design_bundle" : "design_bundle_source",
        source === undefined ? { path } : { path, source },
      )) as {
        code: string;
      };
      if (generation !== refreshGenerationRef.current) return;
      setBundleCode(bundleResult.code);
    } catch (e) {
      if (generation !== refreshGenerationRef.current) return;
      setError((e as Error).message);
    }
  }, []);

  /** Applies one already-built `CanvasEdit` and re-syncs -- the single
   * real path every edit in this screen goes through (the Apply button,
   * the component palette, and a canvas drag all end up here), so none of
   * them can drift apart in how they touch the live buffer. */
  const applyEditObject = useCallback(
    async (edit: Record<string, unknown>) => {
      if (!activeFile) return;
      try {
        const result = (await window.spartan.call("design_apply_edit", {
          edit,
          source: activeFile.content,
        })) as { source: string };
        const oldLength = [...activeFile.content].length;
        await window.spartan.call("edit", {
          doc_id: activeFile.docId,
          start_char: 0,
          end_char: oldLength,
          text: result.source,
        });
        setPreviewSource(null);
        onContentChange(activeFile.path, result.source);
        await refresh(activeFile.path, result.source);
      } catch (e) {
        setError((e as Error).message);
      }
    },
    [activeFile, onContentChange, refresh]
  );

  const applyEditBatch = useCallback(
    async (edits: Record<string, unknown>[]) => {
      if (!activeFile || edits.length === 0) return;
      try {
        let source = activeFile.content;
        for (const edit of edits) {
          const result = (await window.spartan.call("design_apply_edit", { edit, source })) as { source: string };
          source = result.source;
        }
        await window.spartan.call("edit", {
          doc_id: activeFile.docId,
          start_char: 0,
          end_char: [...activeFile.content].length,
          text: source,
        });
        setPreviewSource(null);
        onContentChange(activeFile.path, source);
        await refresh(activeFile.path, source);
      } catch (e) {
        setError((e as Error).message);
      }
    },
    [activeFile, onContentChange, refresh]
  );

  const undoRedo = useCallback(
    async (direction: "undo" | "redo") => {
      if (!activeFile) return;
      try {
        const result = (await window.spartan.call(direction, { doc_id: activeFile.docId })) as {
          changed: boolean;
          content: string;
        };
        if (!result.changed) return;
        setPreviewSource(null);
        onContentChange(activeFile.path, result.content);
        await refresh(activeFile.path, result.content);
      } catch (e) {
        setError((e as Error).message);
      }
    },
    [activeFile, onContentChange, refresh]
  );

  const buildFormEdit = useCallback((): Record<string, unknown> | null => {
    if (!selectedId) return null;
    if (editKind === "PropChange") {
      if (!propKey.trim()) return null;
      return { kind: "PropChange", nodeId: selectedId, prop: propKey, value: propValue, valueType: propValueType };
    }
    if (editKind === "PropRemove") {
      if (!propKey.trim()) return null;
      return { kind: "PropRemove", nodeId: selectedId, prop: propKey };
    }
    if (editKind === "StyleChange") {
      if (!propKey.trim()) return null;
      return { kind: "StyleChange", nodeId: selectedId, property: propKey, value: propValue };
    }
    if (editKind === "StyleRemove") {
      if (!propKey.trim()) return null;
      return { kind: "StyleRemove", nodeId: selectedId, property: propKey };
    }
    if (editKind === "StyleClear") return { kind: "StyleClear", nodeId: selectedId };
    if (editKind === "TextChange") {
      return { kind: "TextChange", nodeId: selectedId, text: textValue };
    }
    if (editKind === "TagChange") {
      if (!isValidTagName(tagName)) return null;
      return { kind: "TagChange", nodeId: selectedId, tagName: tagName.trim() };
    }
    if (editKind === "Wrap") {
      if (!isValidTagName(wrapTagName)) return null;
      return { kind: "Wrap", nodeId: selectedId, tagName: wrapTagName.trim() };
    }
    if (editKind === "Unwrap") return { kind: "Unwrap", nodeId: selectedId };
    return null;
  }, [selectedId, editKind, propKey, propValue, propValueType, textValue, tagName, wrapTagName]);

  const previewFormEdit = useCallback(async () => {
    if (!activeFile) return;
    const edit = buildFormEdit();
    if (!edit) return;
    try {
      const result = (await window.spartan.call("design_apply_edit", {
        edit,
        source: previewSource ?? activeFile.content,
      })) as { source: string };
      setPreviewSource(result.source);
      await refresh(activeFile.path, result.source);
    } catch (e) {
      setError((e as Error).message);
    }
  }, [activeFile, buildFormEdit, previewSource, refresh]);

  const resetPreview = useCallback(async () => {
    if (!activeFile || previewSource === null) return;
    setPreviewSource(null);
    await refresh(activeFile.path, activeFile.content);
  }, [activeFile, previewSource, refresh]);

  const variantStorageKey = activeFile ? `spartan.gui-builder.variants:${activeFile.path}` : null;
  const interactionStorageKey = activeFile ? `spartan.gui-builder.interactions:${activeFile.path}` : null;

  const saveVariant = useCallback(() => {
    if (!variantStorageKey || !previewSource || !variantName.trim()) return;
    const next: VariantPreset[] = [
      { name: variantName.trim(), source: previewSource, updatedAt: Date.now() },
      ...variantPresets.filter((preset) => preset.name !== variantName.trim()),
    ];
    setVariantPresets(next);
    setVariantName("");
    try {
      window.localStorage.setItem(variantStorageKey, JSON.stringify(next));
    } catch (e) {
      setError(`Could not save variant preset: ${(e as Error).message}`);
    }
  }, [variantStorageKey, previewSource, variantName, variantPresets]);

  const loadVariant = useCallback(
    async (preset: VariantPreset) => {
      if (!activeFile) return;
      setPreviewSource(preset.source);
      await refresh(activeFile.path, preset.source);
    },
    [activeFile, refresh]
  );

  const deleteVariant = useCallback(
    (name: string) => {
      if (!variantStorageKey) return;
      const next = variantPresets.filter((preset) => preset.name !== name);
      setVariantPresets(next);
      try {
        window.localStorage.setItem(variantStorageKey, JSON.stringify(next));
      } catch (e) {
        setError(`Could not delete variant preset: ${(e as Error).message}`);
      }
    },
    [variantStorageKey, variantPresets]
  );

  const saveInteractionPreset = useCallback(() => {
    if (!interactionStorageKey || !interactionPresetName.trim()) return;
    const name = interactionPresetName.trim();
    const next: InteractionPreset[] = [
      { name, state: previewInteractionState, updatedAt: Date.now() },
      ...interactionPresets.filter((preset) => preset.name !== name),
    ];
    setInteractionPresets(next);
    setInteractionPresetName("");
    try {
      window.localStorage.setItem(interactionStorageKey, JSON.stringify(next));
    } catch (e) {
      setError(`Could not save interaction preset: ${(e as Error).message}`);
    }
  }, [interactionStorageKey, interactionPresetName, previewInteractionState, interactionPresets]);

  const deleteInteractionPreset = useCallback((name: string) => {
    if (!interactionStorageKey) return;
    const next = interactionPresets.filter((preset) => preset.name !== name);
    setInteractionPresets(next);
    try {
      window.localStorage.setItem(interactionStorageKey, JSON.stringify(next));
    } catch (e) {
      setError(`Could not delete interaction preset: ${(e as Error).message}`);
    }
  }, [interactionStorageKey, interactionPresets]);

  /** Real drag-to-reparent (task #279), shared by the canvas drop relay
   * and the "Move into" form. `gui-builder`'s own `Reparent` already
   * refuses a root move, a self-move, and a move into a descendant with
   * real descriptive errors -- those surface here as ordinary errors
   * rather than being pre-checked twice in two places. */
  const applyReparentEdit = useCallback(
    async (nodeId: string, newParentId: string) => {
      if (!nodeId || !newParentId || nodeId === newParentId) return;
      await applyEditObject({ kind: "Reparent", nodeId, newParentId });
    },
    [applyEditObject]
  );

  const deleteSelected = useCallback(async () => {
    if (!activeFile || selectedIds.length === 0) return;
    const description = selectedIds.length === 1
      ? `<${selectedNode?.tagName ?? "element"}> and all of its children`
      : `${selectedIds.length} selected elements and all of their children`;
    if (!window.confirm(`Delete ${description}?`)) return;
    await applyEditObject(selectedIds.length === 1
      ? { kind: "Delete", nodeId: selectedIds[0] }
      : { kind: "DeleteMany", nodeIds: selectedIds });
    setSelectedId(null);
    setSelectedIds([]);
  }, [activeFile, selectedIds, selectedNode?.tagName, applyEditObject]);

  const duplicateSelected = useCallback(async () => {
    if (!activeFile || selectedIds.length === 0) return;
    await applyEditObject(selectedIds.length === 1
      ? { kind: "Duplicate", nodeId: selectedIds[0] }
      : { kind: "DuplicateMany", nodeIds: selectedIds });
  }, [activeFile, selectedIds, applyEditObject]);

  const moveSelected = useCallback(async (direction: -1 | 1) => {
    if (!activeFile || selectedIds.length === 0) return;
    if (selectedIds.length > 1) {
      if (!canReorderSelection) return;
      await applyEditObject({ kind: "ReorderMany", nodeIds: selectedIds, direction });
      return;
    }
    if (!selectedId) return;
    const location = findParentEntry(roots, selectedId);
    if (!location) return;
    const nextIndex = location.index + direction;
    if (nextIndex < 0 || nextIndex >= location.parent.children.length) return;
    await applyEditObject({
      kind: "Reparent",
      nodeId: selectedId,
      newParentId: location.parent.id,
      index: nextIndex,
    });
  }, [activeFile, selectedId, selectedIds, canReorderSelection, roots, applyEditObject]);

  const copyStyles = useCallback(() => {
    if (!selectedNode || !hasSingleSelection) return;
    const style = selectedNode.props.style;
    if (!style || style.kind !== "style" || Object.keys(style.entries).length === 0) {
      setError("The selected element has no plain inline styles to copy.");
      return;
    }
    setStyleClipboard({
      sourcePath: activeFile?.path ?? "",
      sourceTagName: selectedNode.tagName,
      entries: Object.fromEntries(Object.entries(style.entries).map(([name, entry]) => [name, { ...entry }])),
    });
    setError(null);
  }, [activeFile?.path, selectedNode, hasSingleSelection]);

  const copyProps = useCallback(() => {
    if (!selectedNode || !hasSingleSelection) return;
    const entries: Record<string, PropClipboardEntry> = {};
    for (const [name, summary] of Object.entries(selectedNode.props)) {
      if (summary.kind === "string") {
        entries[name] = { kind: "string", value: summary.value };
      } else if (summary.kind === "expression") {
        entries[name] = { kind: "expression", source: summary.source };
      }
    }
    if (Object.keys(entries).length === 0) {
      setError("The selected element has no literal or expression props to copy.");
      return;
    }
    setPropClipboard({
      sourcePath: activeFile?.path ?? "",
      sourceTagName: selectedNode.tagName,
      entries,
    });
    setError(null);
  }, [activeFile?.path, selectedNode, hasSingleSelection]);

  const clearStylesSelected = useCallback(async () => {
    if (!activeFile || selectedIds.length === 0) return;
    if (!canClearSelectionStyles) {
      setError("Every selected element must have a plain inline style object; dynamic style expressions are preserved.");
      return;
    }
    const description = selectedIds.length === 1 ? "the selected element" : `${selectedIds.length} selected elements`;
    if (!window.confirm(`Remove all plain inline styles from ${description}?`)) return;
    await applyEditObject(selectedIds.length === 1
      ? { kind: "StyleClear", nodeId: selectedIds[0] }
      : { kind: "StyleClearMany", nodeIds: selectedIds });
  }, [activeFile, selectedIds, canClearSelectionStyles, applyEditObject]);

  const pasteStyles = useCallback(async () => {
    if (!styleClipboard || !activeFile || selectedIds.length === 0) return;
    const hasExpressions = Object.values(styleClipboard.entries).some((entry) => entry.kind === "expression");
    const entries = styleClipboard.sourcePath === activeFile.path
      ? Object.entries(styleClipboard.entries)
      : Object.entries(styleClipboard.entries).filter(([, entry]) => entry.kind === "literal");
    if (entries.length === 0) {
      setError("The copied styles contain only expression values, which cannot be pasted into another component file.");
      return;
    }
    const edits = selectedIds.flatMap((nodeId) => entries.map(([property, entry]) => ({
      kind: "StyleChange",
      nodeId,
      property,
      value: entry.kind === "literal" ? entry.value ?? "" : entry.source ?? "",
      valueType: entry.kind,
    })));
    await applyEditBatch(edits);
    if (styleClipboard.sourcePath !== activeFile.path && hasExpressions) {
      setError("Pasted literal styles; expression-valued styles were skipped because the target file may not share their bindings.");
    }
  }, [styleClipboard, activeFile, selectedIds, applyEditBatch]);

  const pasteProps = useCallback(async () => {
    if (!propClipboard || !activeFile || selectedIds.length === 0) return;
    const hasExpressions = Object.values(propClipboard.entries).some((entry) => entry.kind === "expression");
    const entries = propClipboard.sourcePath === activeFile.path
      ? Object.entries(propClipboard.entries)
      : Object.entries(propClipboard.entries).filter(([, entry]) => entry.kind === "string");
    if (entries.length === 0) {
      setError("The copied props contain only expressions, which cannot be pasted into another component file.");
      return;
    }
    const edits = selectedIds.flatMap((nodeId) => entries.map(([prop, entry]) => ({
      kind: "PropChange",
      nodeId,
      prop,
      value: entry.kind === "string" ? entry.value ?? "" : entry.source ?? "",
      valueType: entry.kind,
    })));
    await applyEditBatch(edits);
    if (propClipboard.sourcePath !== activeFile.path && hasExpressions) {
      setError("Pasted literal props; expression-valued props were skipped because the target file may not share their bindings.");
    }
  }, [propClipboard, activeFile, selectedIds, applyEditBatch]);

  // Keep common canvas shortcuts scoped to the Design screen and out of the
  // inspector's own inputs. Escape clears any selection; Delete/Backspace and
  // Ctrl/Cmd+D reuse the already-confirmed structural commands, while
  // Ctrl/Cmd+Z/Y reuse the document-history toolbar actions.
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      const target = event.target;
      if (target instanceof HTMLElement && target.closest("input, textarea, select, [contenteditable=\"true\"]")) return;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "z") {
        event.preventDefault();
        void undoRedo(event.shiftKey ? "redo" : "undo");
        return;
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "y") {
        event.preventDefault();
        void undoRedo("redo");
        return;
      }
      if (event.key === "Escape" && selectionCount > 0) {
        event.preventDefault();
        setSelectedIds([]);
        return;
      }
      if ((event.key === "Delete" || event.key === "Backspace") && selectionCount > 0) {
        event.preventDefault();
        void deleteSelected();
        return;
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "d" && selectionCount > 0) {
        event.preventDefault();
        void duplicateSelected();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [selectionCount, deleteSelected, duplicateSelected, undoRedo]);

  useEffect(() => {
    if (!viewportStorageKey) {
      setViewportPresets([]);
      return;
    }
    try {
      const saved = JSON.parse(window.localStorage.getItem(viewportStorageKey) ?? "[]") as ViewportPreset[];
      setViewportPresets(Array.isArray(saved)
        ? saved.filter((preset) => preset && typeof preset.name === "string" && Number.isFinite(preset.width) && Number.isFinite(preset.height)
          && preset.width >= 200 && preset.width <= 3000 && preset.height >= 200 && preset.height <= 3000)
        : []);
    } catch {
      setViewportPresets([]);
    }
  }, [viewportStorageKey]);

  useEffect(() => {
    // Invalidate any parse/bundle already in flight when switching files or
    // leaving Design mode, so an older response cannot repaint the new file.
    refreshGenerationRef.current += 1;
    setSelectedId(null);
    setSelectedIds([]);
    if (activeFile && isComponentFile(activeFile.path)) {
      setPreviewSource(null);
      setVariantName("");
      setInteractionPresetName("");
      setPaletteFilter("");
      setAssetFilter("");
      setTokenFilter("");
      const key = `spartan.gui-builder.variants:${activeFile.path}`;
      const interactionKey = `spartan.gui-builder.interactions:${activeFile.path}`;
      try {
        const saved = JSON.parse(window.localStorage.getItem(key) ?? "[]") as VariantPreset[];
        setVariantPresets(Array.isArray(saved) ? saved.filter((preset) => preset && typeof preset.source === "string") : []);
        const savedInteractions = JSON.parse(window.localStorage.getItem(interactionKey) ?? "[]") as InteractionPreset[];
        setInteractionPresets(Array.isArray(savedInteractions)
          ? savedInteractions.filter((preset) => preset && typeof preset.name === "string" && ["normal", "focus", "hover", "active"].includes(preset.state))
          : []);
      } catch {
        setVariantPresets([]);
        setInteractionPresets([]);
      }
    } else {
      setRoots([]);
      setBundleCode(null);
      setVariantPresets([]);
      setInteractionPresets([]);
      setPreviewInteractionState("normal");
    }
  }, [activeFile?.path, refresh]);

  // Keep Code -> Canvas live while the user types in the active editor. A
  // short debounce avoids spawning a real parse/bundle subprocess for every
  // keystroke, while still making the unsaved buffer the authoritative
  // source without requiring a mode/file switch. Any pending refresh is
  // canceled when another edit arrives or the active file changes.
  useEffect(() => {
    if (!activeFile || !isComponentFile(activeFile.path)) return;
    const path = activeFile.path;
    const source = activeFile.content;
    const timer = window.setTimeout(() => {
      setPreviewSource(null);
      void refresh(path, source);
    }, 300);
    return () => window.clearTimeout(timer);
  }, [activeFile?.path, activeFile?.content, refresh]);

  useEffect(() => {
    const handler = (event: MessageEvent) => {
      if (event.source && iframeRef.current?.contentWindow && event.source !== iframeRef.current.contentWindow) return;
      if (event.data?.type === "spartan-canvas-click") {
        selectNodes(event.data.nodeId, Boolean(event.data.shiftKey));
      } else if (event.data?.type === "spartan-canvas-drop") {
        // Real drag-to-reparent from the live canvas (task #279). Routed
        // through the exact same `Reparent` edit the "Move into" form
        // already uses -- including its own real refusals (moving a root,
        // or into a descendant), which surface here as a normal error.
        void applyReparentEdit(event.data.nodeId, event.data.newParentId);
      } else if (event.data?.type === "spartan-canvas-inspect-result" && event.data.nodeId === selectedId) {
        setPreviewInspection(event.data as PreviewInspection);
      }
    };
    window.addEventListener("message", handler);
    return () => window.removeEventListener("message", handler);
  }, [applyReparentEdit, selectedId, selectNodes]);

  // Real, per-kind readiness check -- each structural kind names a
  // different second operand (`reparentTargetId` vs. `insertTagName`)
  // beyond the shared `selectedId`, so "can Apply be pressed" isn't one
  // single condition across all inspector edit kinds.
  const canApply =
    !!activeFile &&
    selectionCount > 0 &&
    (editKind === "PropChange" || editKind === "PropRemove" || editKind === "StyleChange" || editKind === "StyleRemove"
      ? !!propKey.trim()
      : editKind === "StyleClear"
        ? canClearSelectionStyles
      : editKind === "TextChange"
        ? selectionCount > 0
        : editKind === "TagChange"
          ? selectionCount > 0 && isValidTagName(tagName)
        : editKind === "Wrap"
          ? canWrapSelection && isValidTagName(wrapTagName)
        : editKind === "Unwrap"
          ? canUnwrap
        : editKind === "Reparent"
          ? hasSingleSelection && !!reparentTargetId && reparentTargetId !== selectedId
        : editKind === "ComponentInsertSibling"
          ? hasSingleSelection && !!selectedSibling && !!insertTagName.trim()
        : hasSingleSelection && !!insertTagName.trim());

  /** Picking a curated property seeds the value control from the node's
   * own real current style entry, so the form opens showing what's
   * actually in the source rather than blank. Choosing Custom… (or the
   * empty placeholder) clears the key so the raw text fields take over
   * and `styleDef` correctly resolves to `undefined`. */
  const selectStyleProperty = useCallback(
    (name: string) => {
      if (name === "" || name === CUSTOM_STYLE_PROPERTY) {
        setPropKey("");
        setPropValue("");
        return;
      }
      setPropKey(name);
      setPropValue(currentStyleValue(selectedNode, name) ?? "");
    },
    [selectedNode]
  );

  /** Selecting a real existing prop makes the inspector useful as an
   * inspector, not only as a raw source-edit form: literal and expression
   * summaries seed the matching value control, while a style/object summary
   * deliberately clears the value instead of fabricating source. */
  const selectProp = useCallback(
    (name: string) => {
      if (name === CUSTOM_PROP || name === "") {
        setPropKey("");
        setPropValue("");
        setPropValueType("string");
        return;
      }
      const summary = selectedNode?.props[name];
      setPropKey(name);
      if (summary?.kind === "string") {
        setPropValue(summary.value);
        setPropValueType("string");
      } else if (summary?.kind === "expression") {
        setPropValue(summary.source);
        setPropValueType("expression");
      } else {
        setPropValue("");
        setPropValueType("expression");
      }
    },
    [selectedNode]
  );

  const selectStyleRemovalProperty = useCallback(
    (name: string) => {
      if (name === CUSTOM_STYLE_PROPERTY || name === "") {
        setPropKey("");
        return;
      }
      setPropKey(name);
    },
    []
  );

  /** Re-scans the real project every time the palette is opened, never
   * caching -- a component file can be created or renamed between two
   * opens, the same "state can change between opens" reasoning the Git
   * panel's own branch/tag/log sections already follow. */
  const togglePalette = useCallback(async () => {
    if (paletteOpen) {
      setPaletteOpen(false);
      return;
    }
    setPaletteOpen(true);
    if (!projectRoot) return;
    try {
      const result = (await window.spartan.call("design_components", {
        rootDir: projectRoot,
        fromFile: activeFile?.path,
      })) as { components: DiscoveredComponent[] };
      setPalette(result.components);
    } catch (e) {
      setError((e as Error).message);
    }
  }, [paletteOpen, projectRoot, activeFile?.path]);

  /** Inserts a discovered component at the palette's selected child/sibling
   * placement, carrying its import along when it lives in another file -- the whole
   * point of a component browser over the plain tag-name field, which
   * could only ever produce an undefined binding for a cross-file
   * component. */
  const insertComponent = useCallback(
    async (component: DiscoveredComponent) => {
      if (!activeFile || !selectedId || selectedIds.length === 0) return;
      const edit: Record<string, unknown> = selectedIds.length === 1
        ? {
            kind: "ComponentInsert",
            parentId: paletteInsertPlacement === "child" ? selectedId : findParentEntry(roots, selectedId)?.parent.id,
            tagName: component.name,
          }
        : {
            kind: "ComponentInsertMany",
            nodeIds: selectedIds,
            placement: paletteInsertPlacement,
            tagName: component.name,
          };
      const sibling = paletteInsertPlacement === "sibling" && selectedIds.length === 1 ? findParentEntry(roots, selectedId) : null;
      if (paletteInsertPlacement === "sibling" && selectedIds.length === 1 && !sibling) return;
      if (sibling) edit.index = sibling.index + 1;
      if (component.importFrom) {
        edit.importFrom = component.importFrom;
        edit.importIsDefault = component.isDefault;
      }
      await applyEditObject(edit);
    },
    [activeFile, selectedId, selectedIds, paletteInsertPlacement, roots, applyEditObject]
  );

  const toggleAssets = useCallback(async () => {
    if (assetsOpen) {
      setAssetsOpen(false);
      return;
    }
    setAssetsOpen(true);
    if (!projectRoot) return;
    try {
      const result = (await window.spartan.call("design_assets", {
        rootDir: projectRoot,
        fromFile: activeFile?.path,
      })) as { assets: DiscoveredAsset[] };
      setAssets(result.assets);
    } catch (e) {
      setError((e as Error).message);
    }
  }, [assetsOpen, projectRoot, activeFile?.path]);

  const insertAsset = useCallback(
    async (asset: DiscoveredAsset) => {
      if (!activeFile || !selectedId || !hasSingleSelection) return;
      await applyEditObject({
        kind: "ComponentInsert",
        parentId: selectedId,
        tagName: "img",
        props: { src: asset.referencePath, alt: asset.label },
      });
    },
    [activeFile, selectedId, hasSingleSelection, applyEditObject]
  );

  const applyAssetBackground = useCallback(
    async (asset: DiscoveredAsset) => {
      if (!activeFile || !selectedId || !hasSingleSelection) return;
      const safeReference = asset.referencePath.replace(/["\\]/g, "\\$&");
      await applyEditObject({
        kind: "StyleChange",
        nodeId: selectedId,
        property: "backgroundImage",
        value: `url("${safeReference}")`,
      });
    },
    [activeFile, selectedId, hasSingleSelection, applyEditObject]
  );

  const copyAssetPath = useCallback(async (asset: DiscoveredAsset) => {
    try {
      await navigator.clipboard.writeText(asset.referencePath);
      setCopiedAsset(asset.file);
    } catch (e) {
      setError(`Could not copy asset path: ${(e as Error).message}`);
    }
  }, []);

  const copyFontCss = useCallback(async (asset: DiscoveredAsset) => {
    if (!asset.fontFaceSnippet) return;
    try {
      await navigator.clipboard.writeText(asset.fontFaceSnippet);
      setCopiedFontCss(asset.file);
    } catch (e) {
      setError(`Could not copy @font-face snippet: ${(e as Error).message}`);
    }
  }, []);

  const copyTokenReference = useCallback(async (token: DiscoveredToken) => {
    const key = `${token.file}:${token.name}`;
    try {
      await navigator.clipboard.writeText(`var(${token.name})`);
      setCopiedToken(key);
      window.setTimeout(() => setCopiedToken((current) => current === key ? null : current), 1600);
    } catch (e) {
      setError(`Could not copy token reference: ${(e as Error).message}`);
    }
  }, []);

  const copyInspection = useCallback(async () => {
    if (!previewInspection || !selectedNode) return;
    try {
      await navigator.clipboard.writeText(inspectionCssSnapshot(previewInspection, selectedNode.tagName));
      setCopiedInspection(true);
      window.setTimeout(() => setCopiedInspection(false), 1600);
    } catch (e) {
      setError(`Could not copy rendered style snapshot: ${(e as Error).message}`);
    }
  }, [previewInspection, selectedNode]);

  const copySelectedJsx = useCallback(async () => {
    if (!selectedNode?.sourceText || !hasSingleSelection) return;
    try {
      await navigator.clipboard.writeText(selectedNode.sourceText);
      setError(null);
    } catch (e) {
      setError(`Could not copy selected JSX: ${(e as Error).message}`);
    }
  }, [hasSingleSelection, selectedNode]);

  const setPreviewFocus = useCallback((focused: boolean) => {
    if (!selectedId || !hasSingleSelection) return;
    iframeRef.current?.contentWindow?.postMessage(
      { type: focused ? "spartan-canvas-focus" : "spartan-canvas-blur", nodeId: selectedId },
      "*",
    );
    iframeRef.current?.contentWindow?.postMessage(
      { type: "spartan-canvas-state", nodeId: selectedId, state: null },
      "*",
    );
    setPreviewInteractionState(focused ? "focus" : "normal");
  }, [selectedId, hasSingleSelection]);

  const setPreviewState = useCallback((state: "hover" | "active" | null) => {
    if (!selectedId || !hasSingleSelection) return;
    iframeRef.current?.contentWindow?.postMessage(
      { type: "spartan-canvas-blur", nodeId: selectedId },
      "*",
    );
    iframeRef.current?.contentWindow?.postMessage(
      { type: "spartan-canvas-state", nodeId: selectedId, state },
      "*",
    );
    setPreviewInteractionState(state ?? "normal");
  }, [selectedId, hasSingleSelection]);

  const applyInteractionPreset = useCallback((preset: InteractionPreset) => {
    if (!selectedId || !hasSingleSelection) return;
    iframeRef.current?.contentWindow?.postMessage(
      { type: preset.state === "focus" ? "spartan-canvas-focus" : "spartan-canvas-blur", nodeId: selectedId },
      "*",
    );
    iframeRef.current?.contentWindow?.postMessage(
      { type: "spartan-canvas-state", nodeId: selectedId, state: preset.state === "hover" || preset.state === "active" ? preset.state : null },
      "*",
    );
    setPreviewInteractionState(preset.state);
  }, [selectedId, hasSingleSelection]);

  const setBoxModel = useCallback((visible: boolean) => {
    if (!selectedId || !hasSingleSelection) return;
    setBoxModelVisible(visible);
    iframeRef.current?.contentWindow?.postMessage(
      { type: "spartan-canvas-box-model", nodeId: selectedId, visible },
      "*",
    );
  }, [selectedId, hasSingleSelection]);

  const toggleTokens = useCallback(async () => {
    if (tokensOpen) {
      setTokensOpen(false);
      return;
    }
    setTokensOpen(true);
    if (!projectRoot) return;
    try {
      const result = (await window.spartan.call("design_tokens", { rootDir: projectRoot })) as {
        tokens: DiscoveredToken[];
      };
      setTokens(result.tokens);
      setTokenDrafts(Object.fromEntries(result.tokens.map((token) => [`${token.file}:${token.name}`, token.value])));
    } catch (e) {
      setError((e as Error).message);
    }
  }, [tokensOpen, projectRoot]);

  const applyToken = useCallback(
    async (token: DiscoveredToken) => {
      if (!activeFile || !selectedId || !hasSingleSelection || editKind !== "StyleChange" || !propKey.trim()) return;
      await applyEditObject({
        kind: "StyleChange",
        nodeId: selectedId,
        property: propKey.trim(),
        value: `var(${token.name})`,
      });
      setPropValue(`var(${token.name})`);
    },
    [activeFile, selectedId, hasSingleSelection, editKind, propKey, applyEditObject]
  );

  const applyTokenDefinition = useCallback(
    async (token: DiscoveredToken) => {
      const cssFile = openFiles.find((file) => file.path === token.file);
      if (!cssFile) {
        setError("Open the token's CSS file in the Editor before changing its definition.");
        return;
      }
      const draft = tokenDrafts[`${token.file}:${token.name}`] ?? token.value;
      try {
        const result = (await window.spartan.call("design_token_apply", {
          path: token.file,
          name: token.name,
          value: draft,
          source: cssFile.content,
        })) as { source: string };
        await window.spartan.call("edit", {
          doc_id: cssFile.docId,
          start_char: 0,
          end_char: [...cssFile.content].length,
          text: result.source,
        });
        onContentChange(cssFile.path, result.source);
        setTokens((current) => current.map((item) => item.file === token.file && item.name === token.name ? { ...item, value: draft.trim() } : item));
        if (activeFile) await refresh(activeFile.path, previewSource ?? activeFile.content);
      } catch (e) {
        setError((e as Error).message);
      }
    },
    [openFiles, tokenDrafts, onContentChange, activeFile, previewSource, refresh]
  );

  const defineToken = useCallback(async () => {
    const name = newTokenName.trim();
    const value = newTokenValue.trim();
    const cssFile = openCssFiles.find((file) => file.path === newTokenFile) ?? openCssFiles[0];
    if (!cssFile) {
      setError("Open a CSS file in the Editor before defining a token.");
      return;
    }
    if (!name || !value) {
      setError("Enter both a token name and value.");
      return;
    }
    try {
      const result = (await window.spartan.call("design_token_define", {
        path: cssFile.path,
        name,
        value,
        source: cssFile.content,
      })) as { source: string };
      await window.spartan.call("edit", {
        doc_id: cssFile.docId,
        start_char: 0,
        end_char: [...cssFile.content].length,
        text: result.source,
      });
      onContentChange(cssFile.path, result.source);
      const relativePath = projectRoot && cssFile.path.startsWith(projectRoot)
        ? cssFile.path.slice(projectRoot.length).replace(/^[/\\]/, "")
        : cssFile.path;
      const createdToken: DiscoveredToken = {
        name,
        value: value.trim(),
        file: cssFile.path,
        relativePath,
      };
      setTokens((current) => {
        const existing = current.findIndex((token) => token.file === createdToken.file && token.name === createdToken.name);
        if (existing < 0) return [...current, createdToken];
        return current.map((token, index) => index === existing ? { ...token, value: createdToken.value } : token);
      });
      setTokenDrafts((current) => ({ ...current, [`${cssFile.path}:${name}`]: value.trim() }));
      setNewTokenName("");
      setNewTokenValue("");
      if (activeFile) await refresh(activeFile.path, previewSource ?? activeFile.content);
    } catch (e) {
      setError((e as Error).message);
    }
  }, [newTokenName, newTokenValue, newTokenFile, openCssFiles, onContentChange, projectRoot, activeFile, previewSource, refresh]);

  const applyEdit = useCallback(async () => {
    if (!activeFile || !selectedId) return;
    let edit: Record<string, unknown>;
    if (editKind === "PropChange") {
      if (!propKey.trim()) return;
      edit = { kind: "PropChange", nodeId: selectedId, prop: propKey, value: propValue, valueType: propValueType };
    } else if (editKind === "PropRemove") {
      if (!propKey.trim()) return;
      edit = { kind: "PropRemove", nodeId: selectedId, prop: propKey };
    } else if (editKind === "StyleChange") {
      if (!propKey.trim()) return;
      edit = { kind: "StyleChange", nodeId: selectedId, property: propKey, value: propValue };
    } else if (editKind === "StyleRemove") {
      if (!propKey.trim()) return;
      edit = { kind: "StyleRemove", nodeId: selectedId, property: propKey };
    } else if (editKind === "StyleClear") {
      if (!canClearSelectionStyles) return;
      edit = selectedIds.length > 1
        ? { kind: "StyleClearMany", nodeIds: selectedIds }
        : { kind: "StyleClear", nodeId: selectedId };
    } else if (editKind === "TextChange") {
      edit = selectedIds.length > 1
        ? { kind: "TextChangeMany", nodeIds: selectedIds, text: textValue }
        : { kind: "TextChange", nodeId: selectedId, text: textValue };
    } else if (editKind === "TagChange") {
      if (!isValidTagName(tagName)) return;
      edit = selectedIds.length > 1
        ? { kind: "TagChangeMany", nodeIds: selectedIds, tagName: tagName.trim() }
        : { kind: "TagChange", nodeId: selectedId, tagName: tagName.trim() };
    } else if (editKind === "Wrap") {
      if (!isValidTagName(wrapTagName)) return;
      edit = selectedIds.length > 1
        ? { kind: "WrapMany", nodeIds: selectedIds, tagName: wrapTagName.trim() }
        : { kind: "Wrap", nodeId: selectedId, tagName: wrapTagName.trim() };
    } else if (editKind === "Unwrap") {
      edit = { kind: "Unwrap", nodeId: selectedId };
    } else if (editKind === "Reparent") {
      if (!reparentTargetId || reparentTargetId === selectedId) return;
      edit = { kind: "Reparent", nodeId: selectedId, newParentId: reparentTargetId };
    } else {
      if (!insertTagName.trim()) return;
      if (editKind === "ComponentInsertSibling" && !selectedSibling) return;
      let parsedProps: ParsedInsertProps;
      try {
        parsedProps = parseInsertProps(insertProps);
      } catch (e) {
        setError((e as Error).message);
        return;
      }
      edit = {
        kind: "ComponentInsert",
        parentId: editKind === "ComponentInsertSibling" ? selectedSibling!.parent.id : selectedId,
        tagName: insertTagName.trim(),
      };
      if (editKind === "ComponentInsertSibling") edit.index = selectedSibling!.index + 1;
      if (Object.keys(parsedProps.props).length > 0) edit.props = parsedProps.props;
      if (Object.keys(parsedProps.propValues).length > 0) edit.propValues = parsedProps.propValues;
      if (insertText !== "") edit.childrenText = insertText;
    }
    if (selectedIds.length > 1 && ["PropChange", "PropRemove", "StyleChange", "StyleRemove"].includes(editKind)) {
      await applyEditBatch(selectedIds.map((nodeId) => ({ ...edit, nodeId })));
    } else {
      await applyEditObject(edit);
    }
    setPropKey("");
    setPropValue("");
    setTextValue("");
    setTagName("");
    setWrapTagName("");
    setPropValueType("string");
    setReparentTargetId("");
    setInsertTagName("");
    setInsertProps("");
    setInsertText("");
  }, [activeFile, selectedId, selectedIds, selectedSibling, canClearSelectionStyles, hasSingleSelection, propKey, propValue, propValueType, textValue, tagName, wrapTagName, editKind, reparentTargetId, insertTagName, insertProps, insertText, applyEditObject, applyEditBatch]);

  if (!activeFile || !isComponentFile(activeFile.path)) {
    return (
      <div className="empty-state mono">
        Open a .jsx or .tsx file in the Editor to see its live preview here.
      </div>
    );
  }

  const srcDoc = bundleCode
    ? `<!doctype html><html><head><style>body{margin:0;background:#fff;color:#111;font-family:sans-serif;}</style></head><body><div id="spartan-root"></div><script>${bundleCode}</script></body></html>`
    : "";

  return (
    <div className="design-screen">
      <div className="design-file-toolbar mono">
        <label htmlFor="design-file-select">Component file</label>
        <select
          id="design-file-select"
          value={activeFile.path}
          onChange={(event) => onOpenFile(event.target.value)}
        >
          {openFiles.filter((file) => isComponentFile(file.path)).map((file) => (
            <option key={file.path} value={file.path}>
              {file.path.split(/[\\/]/).pop()}{file.dirty ? " •" : ""}
            </option>
          ))}
        </select>
        <span className="design-file-path" title={activeFile.path}>{activeFile.path}</span>
        <span className="design-file-spacer" />
        <button className="design-toolbar-button mono" title="Undo last visual or editor edit" onClick={() => void undoRedo("undo")}>
          Undo
        </button>
        <button className="design-toolbar-button mono" title="Redo the last undone edit" onClick={() => void undoRedo("redo")}>
          Redo
        </button>
        {previewSource !== null && (
          <button className="design-toolbar-button mono" title="Discard preview-only changes" onClick={() => void resetPreview()}>
            Reset preview
          </button>
        )}
      </div>
      <div className="design-tree-panel">
        <div className="design-tree-header">
          <div className="design-panel-label">Structure</div>
          <input
            className="design-tree-filter mono"
            aria-label="Filter structure tree"
            placeholder="Filter…"
            value={treeFilter}
            onChange={(event) => setTreeFilter(event.target.value)}
          />
        </div>
        <div role="tree" aria-label="Structure tree">
          {filteredRoots.length > 0 ? filteredRoots.map((root) => (
            <TreeNode key={root.id} node={root} depth={0} selectedIds={selectedIds} onSelect={selectNodes} filterActive={treeFilter.trim().length > 0} />
          )) : (
            <div className="design-tree-empty mono">No matching elements.</div>
          )}
        </div>
      </div>
      <div className="design-preview">
        <div className="design-preview-toolbar mono">
          <label>
            Viewport
            <select value={viewportId} onChange={(event) => setViewportId(event.target.value as typeof viewportId)}>
              {DESIGN_VIEWPORTS.map((item) => <option key={item.id} value={item.id}>{item.label}</option>)}
              <option value="custom">Custom</option>
            </select>
          </label>
          {viewportId === "custom" && (
            <span className="design-custom-viewport">
              <label>W <input type="number" min={200} max={3000} value={customViewportWidth} onChange={(event) => setCustomViewportWidth(Math.max(200, Math.min(3000, Number(event.target.value) || 200)))} /></label>
              <label>H <input type="number" min={200} max={3000} value={customViewportHeight} onChange={(event) => setCustomViewportHeight(Math.max(200, Math.min(3000, Number(event.target.value) || 200)))} /></label>
              <input
                className="design-input mono"
                aria-label="Viewport preset name"
                placeholder="preset name"
                value={viewportPresetName}
                onChange={(event) => setViewportPresetName(event.target.value)}
              />
              <button className="design-secondary-action mono" onClick={saveViewportPreset} disabled={!viewportPresetName.trim() || !viewportStorageKey}>
                Save viewport
              </button>
            </span>
          )}
          {viewportPresets.length > 0 && (
            <span className="design-custom-viewport">
              <select
                className="design-input mono"
                aria-label="Saved viewport presets"
                value=""
                onChange={(event) => {
                  const preset = viewportPresets.find((item) => item.name === event.target.value);
                  if (preset) applyViewportPreset(preset);
                }}
              >
                <option value="">Saved viewports…</option>
                {viewportPresets.map((preset) => <option key={preset.name} value={preset.name}>{preset.name} · {preset.width}×{preset.height}</option>)}
              </select>
              {viewportPresets.map((preset) => (
                <button key={`delete-${preset.name}`} className="design-asset-action mono" title={`Delete viewport preset ${preset.name}`} onClick={() => deleteViewportPreset(preset.name)}>
                  ×
                </button>
              ))}
            </span>
          )}
          <label>
            Zoom {previewZoom}%
            <input type="range" min={25} max={100} step={5} value={previewZoom} onChange={(event) => setPreviewZoom(Number(event.target.value))} />
          </label>
          <span>{viewport.width} × {viewport.height}</span>
        </div>
        {bundleCode ? (
          <div className="design-preview-stage">
            <iframe
              ref={iframeRef}
              className="design-iframe"
              style={{ width: viewport.width, height: viewport.height, transform: `scale(${previewZoom / 100})` }}
              sandbox="allow-scripts"
              srcDoc={srcDoc}
              title="Live preview"
              onLoad={() => {
                iframeRef.current?.contentWindow?.postMessage(
                  { type: "spartan-canvas-select", nodeId: selectedId, nodeIds: selectedIds },
                  "*",
                );
                iframeRef.current?.contentWindow?.postMessage(
                  { type: "spartan-canvas-box-model", nodeId: selectedId, visible: boxModelVisible },
                  "*",
                );
                if (selectedId) {
                  iframeRef.current?.contentWindow?.postMessage(
                    { type: "spartan-canvas-inspect", nodeId: selectedId },
                    "*",
                  );
                }
              }}
            />
          </div>
        ) : (
          <div className="empty-state mono">{error ?? "Bundling..."}</div>
        )}
      </div>
      <div className="design-edit-panel">
        <div className="design-panel-label">Edit</div>
        {projectRoot && (
          <>
            <button className="design-palette-toggle mono" onClick={togglePalette}>
              {paletteOpen ? "▾" : "▸"} Components{palette.length > 0 ? ` (${palette.length})` : ""}
            </button>
            {paletteOpen && (
              <div className="design-palette">
                <div className="design-palette-placement mono" aria-label="Component insertion placement">
                  <span>Insert as</span>
                  <label>
                    <input
                      type="radio"
                      checked={paletteInsertPlacement === "child"}
                      onChange={() => setPaletteInsertPlacement("child")}
                    />
                    child
                  </label>
                  <label>
                    <input
                      type="radio"
                      checked={paletteInsertPlacement === "sibling"}
                      onChange={() => setPaletteInsertPlacement("sibling")}
                    />
                    sibling
                  </label>
                </div>
                <input
                  className="design-palette-filter mono"
                  aria-label="Filter discovered components"
                  placeholder="Filter components…"
                  value={paletteFilter}
                  onChange={(event) => setPaletteFilter(event.target.value)}
                />
                {palette.length === 0 ? (
                  <div className="design-palette-empty mono">
                    No exported components found under the project root.
                  </div>
                ) : filteredPalette.length === 0 ? (
                  <div className="design-palette-empty mono">
                    No components match “{paletteFilter}”.
                  </div>
                ) : (
                  filteredPalette.map((c) => (
                    <button
                      key={`${c.file}:${c.name}`}
                      className="design-palette-item mono"
                          disabled={!selectedId || selectedIds.length === 0}
                          title={
                        selectedId && selectedIds.length > 0
                          ? `Insert <${c.name} /> as a ${paletteInsertPlacement} of ${selectedIds.length > 1 ? `${selectedIds.length} selected elements` : "the selected element"}${
                              c.importFrom ? ` and import it from "${c.importFrom}"` : ""
                            }`
                          : "Select one or more elements in the tree first"
                      }
                      onClick={() => insertComponent(c)}
                    >
                      <span className="design-palette-name">&lt;{c.name} /&gt;</span>
                      <span className="design-palette-from">
                        {c.importFrom ?? "this file"}
                      </span>
                    </button>
                  ))
                )}
              </div>
            )}
            <button className="design-palette-toggle mono" onClick={toggleAssets}>
              {assetsOpen ? "▾" : "▸"} Assets{assets.length > 0 ? ` (${assets.length})` : ""}
            </button>
            {assetsOpen && (
              <div className="design-palette">
                <input
                  className="design-palette-filter mono"
                  aria-label="Filter discovered assets"
                  placeholder="Filter assets…"
                  value={assetFilter}
                  onChange={(event) => setAssetFilter(event.target.value)}
                />
                {assets.length === 0 ? (
                  <div className="design-palette-empty mono">
                    No image or font assets found under the project root.
                  </div>
                ) : filteredAssets.length === 0 ? (
                  <div className="design-palette-empty mono">
                    No assets match “{assetFilter}”.
                  </div>
                ) : (
                  <>
                    {filteredAssets.filter((asset) => asset.kind === "image").map((asset) => (
                      <div key={asset.file} className="design-asset-row">
                        <button
                          className="design-palette-item mono"
                          disabled={!selectedId || !hasSingleSelection}
                          title={selectedId && hasSingleSelection ? `Insert ${asset.label} into the selected element` : "Select exactly one element in the tree first"}
                          onClick={() => insertAsset(asset)}
                        >
                          <span className="design-palette-name">▧ {asset.label}</span>
                          <span className="design-palette-from">{asset.relativePath}</span>
                        </button>
                        <button
                          className="design-asset-action mono"
                          disabled={!selectedId || !hasSingleSelection}
                          title={selectedId && hasSingleSelection ? `Use ${asset.label} as the selected element's background image` : "Select exactly one element in the tree first"}
                          onClick={() => void applyAssetBackground(asset)}
                        >
                          BG
                        </button>
                      </div>
                    ))}
                    {filteredAssets.filter((asset) => asset.kind === "font").map((asset) => (
                      <div key={asset.file} className="design-asset-row">
                        <button
                          className="design-palette-item mono"
                          title={`Copy the relative font path ${asset.referencePath}`}
                          onClick={() => void copyAssetPath(asset)}
                        >
                          <span className="design-palette-name">Aa {asset.label}</span>
                          <span className="design-palette-from">{copiedAsset === asset.file ? "Copied · " : "Copy · "}{asset.referencePath}</span>
                        </button>
                        <button
                          className="design-asset-action mono"
                          disabled={!asset.fontFaceSnippet}
                          title="Copy a format-aware @font-face CSS snippet"
                          onClick={() => void copyFontCss(asset)}
                        >
                          {copiedFontCss === asset.file ? "CSS ✓" : "CSS"}
                        </button>
                      </div>
                    ))}
                  </>
                )}
              </div>
            )}
            <button className="design-palette-toggle mono" onClick={toggleTokens}>
              {tokensOpen ? "▾" : "▸"} Design tokens{tokens.length > 0 ? ` (${tokens.length})` : ""}
            </button>
            {tokensOpen && (
              <div className="design-palette">
                <input
                  className="design-palette-filter mono"
                  aria-label="Filter design tokens"
                  placeholder="Filter tokens…"
                  value={tokenFilter}
                  onChange={(event) => setTokenFilter(event.target.value)}
                />
                <div className="design-token-create">
                  <select
                    className="design-token-file mono"
                    aria-label="CSS file for new token"
                    value={newTokenFile || openCssFiles[0]?.path || ""}
                    onChange={(event) => setNewTokenFile(event.target.value)}
                    disabled={openCssFiles.length === 0}
                  >
                    {openCssFiles.length === 0 ? <option value="">Open a CSS file first</option> : openCssFiles.map((file) => (
                      <option key={file.path} value={file.path}>{file.path}</option>
                    ))}
                  </select>
                  <input
                    className="design-token-value mono"
                    aria-label="New token name"
                    placeholder="--token-name"
                    value={newTokenName}
                    onChange={(event) => setNewTokenName(event.target.value)}
                  />
                  <input
                    className="design-token-value mono"
                    aria-label="New token value"
                    placeholder="value"
                    value={newTokenValue}
                    onChange={(event) => setNewTokenValue(event.target.value)}
                  />
                  <button
                    className="design-token-save mono"
                    disabled={openCssFiles.length === 0 || !newTokenName.trim() || !newTokenValue.trim()}
                    title="Define this CSS custom property in the selected file"
                    onClick={() => void defineToken()}
                  >
                    Define
                  </button>
                </div>
                {tokens.length === 0 ? (
                  <div className="design-palette-empty mono">No CSS custom properties found under the project root.</div>
                ) : filteredTokens.length === 0 ? (
                  <div className="design-palette-empty mono">No tokens match “{tokenFilter}”.</div>
                ) : (
                  filteredTokens.map((token, index) => {
                    const tokenKey = `${token.file}:${token.name}`;
                    const cssOpen = openFiles.some((file) => file.path === token.file);
                    return (
                      <div key={`${token.file}:${token.name}:${index}`} className="design-token-row">
                        <button
                          className="design-palette-item mono design-token-use"
                          disabled={!selectedId || !hasSingleSelection || editKind !== "StyleChange" || !propKey.trim()}
                          title={
                            selectedId && hasSingleSelection && editKind === "StyleChange" && propKey.trim()
                              ? `Set ${propKey} to var(${token.name})`
                              : "Choose Style editing and a property first"
                          }
                          onClick={() => applyToken(token)}
                        >
                          <span className="design-palette-name">{token.name}</span>
                          <span className="design-palette-from">{token.value} · {token.relativePath}</span>
                        </button>
                        <input
                          className="design-token-value mono"
                          aria-label={`Value for ${token.name}`}
                          value={tokenDrafts[tokenKey] ?? token.value}
                          onChange={(event) => setTokenDrafts((current) => ({ ...current, [tokenKey]: event.target.value }))}
                        />
                        <button
                          className="design-token-save mono"
                          disabled={!cssOpen}
                          title={cssOpen ? `Update ${token.name}` : "Open this CSS file in the Editor first"}
                          onClick={() => void applyTokenDefinition(token)}
                        >
                          Save
                        </button>
                        <button
                          className="design-token-save mono"
                          title={`Copy var(${token.name})`}
                          onClick={() => void copyTokenReference(token)}
                        >
                          {copiedToken === tokenKey ? "Copied" : "Copy"}
                        </button>
                      </div>
                    );
                  })
                )}
              </div>
            )}
          </>
        )}
        {selectedNode ? (
          <>
            <div className="design-selected mono">
              {selectionCount > 1
                ? `${selectionCount} elements selected · primary <${selectedNode.tagName}> #${selectedNode.id}`
                : ` <${selectedNode.tagName}> #${selectedNode.id}`}
            </div>
            {hasSingleSelection && selectedNode.sourceLocation && (
              <button
                className="design-secondary-action mono design-reveal-source"
                title="Open the source location in Editor"
                onClick={() => onRevealSource(activeFile.path, selectedNode.sourceLocation!.startLine, selectedNode.sourceLocation!.startColumn)}
              >
                Reveal in Editor · line {selectedNode.sourceLocation.startLine}
              </button>
            )}
            {hasSingleSelection && selectedNode.sourceText && (
              <button
                className="design-secondary-action mono design-reveal-source"
                title="Copy the selected element's exact JSX source"
                onClick={() => void copySelectedJsx()}
              >
                Copy JSX
              </button>
            )}
            {hasSingleSelection && previewInspection?.nodeId === selectedNode.id && (
              <div className="design-inspection mono" aria-label="Rendered element inspection">
                <div className="design-inspection-title">Rendered preview</div>
                <div className="design-inspection-grid">
                  <span>size</span><strong>{Math.round(previewInspection.rect.width)} × {Math.round(previewInspection.rect.height)} px</strong>
                  <span>display</span><strong>{previewInspection.styles.display}</strong>
                  <span>position</span><strong>{previewInspection.styles.position}</strong>
                  <span>z-index</span><strong>{previewInspection.styles.zIndex || "auto"}</strong>
                  <span>font</span><strong>{previewInspection.styles.fontSize}</strong>
                  <span>color</span><strong>{previewInspection.styles.color}</strong>
                  <span>background</span><strong>{previewInspection.styles.backgroundColor}</strong>
                  <span>padding</span><strong>{previewInspection.styles.padding} · {previewInspection.styles.paddingTop}/{previewInspection.styles.paddingRight}/{previewInspection.styles.paddingBottom}/{previewInspection.styles.paddingLeft}</strong>
                  <span>border</span><strong>{previewInspection.styles.borderTopWidth}/{previewInspection.styles.borderRightWidth}/{previewInspection.styles.borderBottomWidth}/{previewInspection.styles.borderLeftWidth}</strong>
                  <span>margin</span><strong>{previewInspection.styles.margin} · {previewInspection.styles.marginTop}/{previewInspection.styles.marginRight}/{previewInspection.styles.marginBottom}/{previewInspection.styles.marginLeft}</strong>
                  <span>flex</span><strong>{previewInspection.styles.flexDirection || "—"} · {previewInspection.styles.flexWrap || "—"}</strong>
                  <span>alignment</span><strong>{previewInspection.styles.justifyContent || "—"} / {previewInspection.styles.alignItems || "—"}</strong>
                  <span>grid</span><strong>{previewInspection.styles.gridTemplateColumns || "—"} / {previewInspection.styles.gridTemplateRows || "—"}</strong>
                  <span>overflow</span><strong>{previewInspection.styles.overflowX} / {previewInspection.styles.overflowY}</strong>
                </div>
                <button className="design-secondary-action mono design-inspection-copy" onClick={() => void copyInspection()}>
                  {copiedInspection ? "Copied CSS snapshot" : "Copy CSS snapshot"}
                </button>
                <div className="design-preview-status mono" aria-label="Accessibility audit">
                  Accessibility audit
                </div>
                {accessibilityFindings.map((finding, index) => (
                  <div className="design-preview-status mono" key={`${finding.severity}-${index}`}>
                    {finding.severity === "pass" ? "✓" : finding.severity === "error" ? "×" : finding.severity === "warning" ? "!" : "·"} {finding.message}
                  </div>
                ))}
                <div className="design-inspection-actions">
                  <button className="design-secondary-action mono" onClick={() => setPreviewFocus(true)}>Focus preview</button>
                  <button className="design-secondary-action mono" onClick={() => setPreviewFocus(false)}>Blur preview</button>
                  <button className="design-secondary-action mono" onClick={() => setPreviewState("hover")}>Hover preview</button>
                  <button className="design-secondary-action mono" onClick={() => setPreviewState("active")}>Active preview</button>
                  <button className="design-secondary-action mono" onClick={() => setPreviewState(null)}>Clear state</button>
                  <button className="design-secondary-action mono" onClick={() => setBoxModel(!boxModelVisible)}>
                    {boxModelVisible ? "Hide box model" : "Show box model"}
                  </button>
                </div>
                <div className="design-interaction-presets" aria-label="Reusable interaction state presets">
                  <div className="design-preview-status mono">
                    Interaction state: {previewInteractionState}
                  </div>
                  <div className="design-interaction-save">
                    <input
                      className="design-input mono"
                      aria-label="Interaction preset name"
                      placeholder="save current state as…"
                      value={interactionPresetName}
                      onChange={(event) => setInteractionPresetName(event.target.value)}
                    />
                    <button
                      className="design-secondary-action mono"
                      onClick={saveInteractionPreset}
                      disabled={!interactionPresetName.trim()}
                    >
                      Save state
                    </button>
                  </div>
                  {interactionPresets.length > 0 && (
                    <div className="design-interaction-preset-list">
                      {interactionPresets.map((preset) => (
                        <div className="design-interaction-preset" key={preset.name}>
                          <button
                            className="design-secondary-action mono"
                            onClick={() => applyInteractionPreset(preset)}
                            disabled={!hasSingleSelection}
                            title={`Apply ${preset.state} preview state`}
                          >
                            {preset.name} · {preset.state}
                          </button>
                          <button
                            className="design-asset-action mono"
                            onClick={() => deleteInteractionPreset(preset.name)}
                            title={`Delete ${preset.name} interaction preset`}
                          >
                            ×
                          </button>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            )}
            <button className="design-danger-action mono" onClick={deleteSelected} disabled={selectionCount === 0}>
              {selectionCount > 1 ? `Delete ${selectionCount} selected elements` : "Delete selected element"}
            </button>
            <button className="design-secondary-action mono" onClick={duplicateSelected} disabled={selectionCount === 0}>
              {selectionCount > 1 ? `Duplicate ${selectionCount} selected elements` : "Duplicate selected element"}
            </button>
            <div className="design-inspection-actions">
              <button
                className="design-secondary-action mono"
                onClick={() => void moveSelected(-1)}
                disabled={selectionCount > 1 ? !canMoveSelectionUp : !selectedSibling || selectedSibling.index === 0}
                title={selectionCount > 1 ? "Move selected siblings up one position" : "Move the selected element before its previous sibling"}
              >
                ↑ {selectionCount > 1 ? "Move selected up" : "Move up"}
              </button>
              <button
                className="design-secondary-action mono"
                onClick={() => void moveSelected(1)}
                disabled={selectionCount > 1 ? !canMoveSelectionDown : !selectedSibling || selectedSibling.index >= selectedSibling.parent.children.length - 1}
                title={selectionCount > 1 ? "Move selected siblings down one position" : "Move the selected element after its next sibling"}
              >
                ↓ {selectionCount > 1 ? "Move selected down" : "Move down"}
              </button>
            </div>
            <div className="design-inspection-actions">
              <button
                className="design-secondary-action mono"
                onClick={copyStyles}
                disabled={!hasSingleSelection || selectedNode.props.style?.kind !== "style" || Object.keys(selectedNode.props.style.entries).length === 0}
                title={hasSingleSelection ? "Copy every plain inline style entry from the selected element" : "Select exactly one styled element first"}
              >
                Copy styles
              </button>
              <button
                className="design-secondary-action mono"
                onClick={() => void pasteStyles()}
                disabled={!styleClipboard || selectionCount === 0}
                title={styleClipboard ? `Paste ${Object.keys(styleClipboard.entries).length} copied styles onto the selection` : "Copy styles from an element first"}
              >
                Paste styles{styleClipboard ? ` (${Object.keys(styleClipboard.entries).length})` : ""}
              </button>
              <button
                className="design-secondary-action mono"
                onClick={copyProps}
                disabled={!hasSingleSelection || !selectedNode || Object.keys(selectedNode.props).every((name) => selectedNode.props[name].kind === "style")}
                title={hasSingleSelection ? "Copy literal and expression JSX props from the selected element" : "Select exactly one element first"}
              >
                Copy props
              </button>
              <button
                className="design-secondary-action mono"
                onClick={() => void pasteProps()}
                disabled={!propClipboard || selectionCount === 0}
                title={propClipboard ? `Paste ${Object.keys(propClipboard.entries).length} copied props onto the selection` : "Copy props from an element first"}
              >
                Paste props{propClipboard ? ` (${Object.keys(propClipboard.entries).length})` : ""}
              </button>
              <button
                className="design-danger-action mono"
                onClick={() => void clearStylesSelected()}
                disabled={!canClearSelectionStyles}
                title="Remove every plain inline style from the selection; dynamic style expressions are preserved"
              >
                {selectionCount > 1 ? `Clear styles on ${selectionCount} selected elements` : "Clear inline styles"}
              </button>
            </div>
            {styleClipboard && (
              <div className="design-preview-status mono">
                Clipboard: {Object.keys(styleClipboard.entries).length} styles from &lt;{styleClipboard.sourceTagName}&gt;
              </div>
            )}
            {propClipboard && (
              <div className="design-preview-status mono">
                Prop clipboard: {Object.keys(propClipboard.entries).length} props from &lt;{propClipboard.sourceTagName}&gt;
              </div>
            )}
            <div className="design-edit-kind">
              <label>
                <input
                  type="radio"
                  checked={editKind === "TextChange"}
                  onChange={() => setEditKind("TextChange")}
                />
                Text
              </label>
              <label>
                <input
                  type="radio"
                  checked={editKind === "TagChange"}
                  onChange={() => setEditKind("TagChange")}
                />
                Tag
              </label>
              <label>
                <input
                  type="radio"
                  checked={editKind === "Wrap"}
                  onChange={() => setEditKind("Wrap")}
                />
                Wrap
              </label>
              <label>
                <input
                  type="radio"
                  checked={editKind === "Unwrap"}
                  onChange={() => setEditKind("Unwrap")}
                />
                Unwrap
              </label>
              <label>
                <input
                  type="radio"
                  checked={editKind === "PropChange"}
                  onChange={() => setEditKind("PropChange")}
                />
                Prop
              </label>
              <label>
                <input
                  type="radio"
                  checked={editKind === "StyleChange"}
                  onChange={() => setEditKind("StyleChange")}
                />
                Style
              </label>
              <label>
                <input
                  type="radio"
                  checked={editKind === "StyleRemove"}
                  onChange={() => setEditKind("StyleRemove")}
                />
                Remove style
              </label>
              <label>
                <input
                  type="radio"
                  checked={editKind === "StyleClear"}
                  onChange={() => setEditKind("StyleClear")}
                />
                Clear styles
              </label>
              <label>
                <input
                  type="radio"
                  checked={editKind === "PropRemove"}
                  onChange={() => setEditKind("PropRemove")}
                />
                Remove prop
              </label>
              <label>
                <input
                  type="radio"
                  checked={editKind === "Reparent"}
                  onChange={() => setEditKind("Reparent")}
                />
                Move into
              </label>
              <label>
                <input
                  type="radio"
                  checked={editKind === "ComponentInsert"}
                  onChange={() => setEditKind("ComponentInsert")}
                />
                Insert child
              </label>
              <label>
                <input
                  type="radio"
                  checked={editKind === "ComponentInsertSibling"}
                  onChange={() => setEditKind("ComponentInsertSibling")}
                />
                Insert sibling
              </label>
            </div>
            {editKind === "PropChange" && (
              <>
                <select
                  className="design-input mono"
                  aria-label="Existing or custom prop"
                  value={selectedNode?.props[propKey] ? propKey : ""}
                  onChange={(e) => selectProp(e.target.value)}
                >
                  <option value="">Select an existing prop…</option>
                  {Object.keys(selectedNode.props).map((name) => (
                    <option key={name} value={name}>{name}</option>
                  ))}
                </select>
                <input
                  className="design-input mono"
                  placeholder="prop name (or choose above)"
                  value={propKey}
                  onChange={(e) => setPropKey(e.target.value)}
                />
                <input
                  className={`design-input mono ${propValueType === "expression" ? "design-expression-input" : ""}`}
                  placeholder="value"
                  value={propValue}
                  onChange={(e) => setPropValue(e.target.value)}
                />
                <select
                  className="design-input mono"
                  aria-label="Prop value type"
                  value={propValueType}
                  onChange={(e) => setPropValueType(e.target.value as typeof propValueType)}
                >
                  <option value="string">String</option>
                  <option value="number">Number</option>
                  <option value="boolean">Boolean</option>
                  <option value="expression">Expression</option>
                </select>
              </>
            )}
            {editKind === "StyleChange" && (
              <>
                <select
                  className="design-input mono"
                  aria-label="Style property"
                  value={styleDef ? styleDef.name : propKey === "" ? "" : CUSTOM_STYLE_PROPERTY}
                  onChange={(e) => selectStyleProperty(e.target.value)}
                >
                  <option value="">Select a property…</option>
                  {STYLE_GROUPS.map(([group, defs]) => (
                    <optgroup key={group} label={group}>
                      {defs.map((d) => (
                        <option key={d.name} value={d.name}>
                          {d.label}
                        </option>
                      ))}
                    </optgroup>
                  ))}
                  <option value={CUSTOM_STYLE_PROPERTY}>Custom…</option>
                </select>
                {/* The Custom… path keeps the exact raw key/value form
                    this screen has always had, so no style property the
                    curated catalog happens to omit becomes unreachable. */}
                {!styleDef && (
                  <input
                    className="design-input mono"
                    placeholder="style property"
                    aria-label="Custom style property"
                    value={propKey}
                    onChange={(e) => setPropKey(e.target.value)}
                  />
                )}
                {styleDef ? (
                  <StyleValueControl def={styleDef} value={propValue} onChange={setPropValue} />
                ) : (
                  <input
                    className="design-input mono"
                    placeholder="value"
                    value={propValue}
                    onChange={(e) => setPropValue(e.target.value)}
                  />
                )}
              </>
            )}
            {editKind === "StyleRemove" && (
              <>
                <select
                  className="design-input mono"
                  aria-label="Existing or custom style property"
                  value={selectedNode?.props.style?.kind === "style" && selectedNode.props.style.entries[propKey] ? propKey : ""}
                  onChange={(e) => selectStyleRemovalProperty(e.target.value)}
                >
                  <option value="">Select an existing style…</option>
                  {selectedNode?.props.style?.kind === "style" && Object.keys(selectedNode.props.style.entries).map((name) => (
                    <option key={name} value={name}>{name}</option>
                  ))}
                </select>
                <input
                  className="design-input mono"
                  placeholder="style property to remove (or choose above)"
                  aria-label="Custom style property to remove"
                  value={propKey}
                  onChange={(e) => setPropKey(e.target.value)}
                />
              </>
            )}
            {editKind === "PropRemove" && (
              <input
                className="design-input mono"
                placeholder="prop name to remove"
                value={propKey}
                onChange={(e) => setPropKey(e.target.value)}
              />
            )}
            {editKind === "TextChange" && (
              <textarea
                className="design-input mono design-text-input"
                aria-label="Element text"
                placeholder="direct text content"
                value={textValue}
                onChange={(event) => setTextValue(event.target.value)}
              />
            )}
            {editKind === "TagChange" && (
              <input
                className="design-input mono"
                aria-label="New JSX tag name"
                placeholder={`new tag name (current: ${selectedNode.tagName})`}
                value={tagName}
                onChange={(event) => setTagName(event.target.value)}
              />
            )}
            {editKind === "Wrap" && (
              <input
                className="design-input mono"
                aria-label="Wrapper JSX tag name"
                placeholder="wrapper tag name (e.g. div or section)"
                value={wrapTagName}
                onChange={(event) => setWrapTagName(event.target.value)}
              />
            )}
            {editKind === "Unwrap" && (
              <div className="design-preview-status mono">
                Promotes this attribute-free wrapper&apos;s children into its parent.
              </div>
            )}
            {editKind === "Reparent" && (
              <select
                className="design-input mono"
                value={reparentTargetId}
                onChange={(e) => setReparentTargetId(e.target.value)}
              >
                <option value="">Select target parent…</option>
                {flattenNodes(roots)
                  .filter((n) => n.id !== selectedId)
                  .map((n) => (
                    <option key={n.id} value={n.id}>
                      {"  ".repeat(n.depth)}&lt;{n.tagName}&gt; #{n.id}
                    </option>
                  ))}
              </select>
            )}
            {(editKind === "ComponentInsert" || editKind === "ComponentInsertSibling") && (
              <>
                <input
                  className="design-input mono"
                  placeholder={editKind === "ComponentInsertSibling" ? "sibling tag name (e.g. Button or UI.Button)" : "new tag name (e.g. Button or UI.Button)"}
                  value={insertTagName}
                  onChange={(e) => setInsertTagName(e.target.value)}
                />
                <textarea
                  className="design-input mono design-text-input"
                  aria-label="Inserted string props"
                  placeholder="props: name=value or name:type=value (one per line)"
                  value={insertProps}
                  onChange={(e) => setInsertProps(e.target.value)}
                  rows={3}
                />
                <textarea
                  className="design-input mono design-text-input"
                  aria-label="Inserted text content"
                  placeholder="initial direct text (optional)"
                  value={insertText}
                  onChange={(e) => setInsertText(e.target.value)}
                  rows={2}
                />
              </>
            )}
            <button className="leo-btn leo-btn-approve" onClick={applyEdit} disabled={!canApply}>
              Apply
            </button>
            {(editKind === "PropChange" || editKind === "PropRemove" || editKind === "StyleChange" || editKind === "StyleRemove" || editKind === "TextChange") && (
              <button className="design-secondary-action mono" onClick={() => void previewFormEdit()} disabled={!canApply || !hasSingleSelection}>
                Preview variant
              </button>
            )}
            {previewSource !== null && (
              <div className="design-preview-status mono">Preview-only changes active</div>
            )}
            {previewSource !== null && (
              <div className="design-variant-save">
                <input
                  className="design-input mono"
                  placeholder="variant name"
                  value={variantName}
                  onChange={(event) => setVariantName(event.target.value)}
                />
                <button className="design-secondary-action mono" onClick={saveVariant} disabled={!variantName.trim()}>
                  Save variant
                </button>
              </div>
            )}
          </>
        ) : (
          <div className="leo-status-message mono">Click a node in the tree or preview to select it.</div>
        )}
        {variantPresets.length > 0 && (
          <div className="design-variant-list">
            <div className="design-panel-label">Saved variants</div>
            {variantPresets.map((preset) => (
              <div className="design-variant-row" key={preset.name}>
                <button className="design-variant-load mono" onClick={() => void loadVariant(preset)}>
                  {preset.name}
                </button>
                <button className="design-variant-delete mono" title={`Delete ${preset.name}`} onClick={() => deleteVariant(preset.name)}>
                  ×
                </button>
              </div>
            ))}
          </div>
        )}
        {error && <div className="leo-error mono">{error}</div>}
      </div>
    </div>
  );
}
