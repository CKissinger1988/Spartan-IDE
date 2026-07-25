import React, { useCallback, useEffect, useRef, useState } from "react";
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
  props: Record<string, PropSummary>;
  children: ComponentNode[];
  textContent: string | null;
}

interface DesignScreenProps {
  activeFile: OpenFile | null;
  onContentChange: (path: string, content: string, saved?: boolean) => void;
}

function isComponentFile(path: string): boolean {
  return path.endsWith(".jsx") || path.endsWith(".tsx");
}

function TreeNode({
  node,
  depth,
  selectedId,
  onSelect,
}: {
  node: ComponentNode;
  depth: number;
  selectedId: string | null;
  onSelect: (id: string) => void;
}): React.ReactElement {
  return (
    <div>
      <div
        className={`design-tree-row ${node.id === selectedId ? "design-tree-row-active" : ""}`}
        style={{ paddingLeft: 8 + depth * 14 }}
        onClick={() => onSelect(node.id)}
      >
        <span className="mono">
          &lt;{node.tagName}&gt; <span className="design-tree-id">#{node.id}</span>
        </span>
      </div>
      {node.children.map((child) => (
        <TreeNode key={child.id} node={child} depth={depth + 1} selectedId={selectedId} onSelect={onSelect} />
      ))}
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
  { name: "height", label: "Height", group: "Layout", control: "length" },
  { name: "maxWidth", label: "Max width", group: "Layout", control: "length" },
  { name: "minHeight", label: "Min height", group: "Layout", control: "length" },
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
];

/** Sentinel for the "Custom…" dropdown entry. Deliberately a string no
 * real CSS property can collide with, so `styleDef` lookup stays a plain
 * name match with no separate "is this custom" flag to keep in sync. */
const CUSTOM_STYLE_PROPERTY = "__custom__";

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
 * doc comment) -- this component just listens for that message and
 * routes it through the same `selectedId` state a tree-row click uses,
 * so a canvas click and a tree click are indistinguishable.
 *
 * `parse`/`bundle` both read from disk (matching the CLI's own
 * documented v1 contract); `apply` reads the real live, possibly-unsaved
 * buffer from `activeFile.content` and its result is fed back through
 * the exact same `edit` IPC call typing already uses, so a canvas edit
 * gets the same undo/dirty tracking as any other edit.
 *
 * All four real `CanvasEdit` kinds `gui-builder` itself supports are now
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
  onContentChange,
}: DesignScreenProps): React.ReactElement {
  const [roots, setRoots] = useState<ComponentNode[]>([]);
  const [bundleCode, setBundleCode] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [propKey, setPropKey] = useState("");
  const [propValue, setPropValue] = useState("");
  const [editKind, setEditKind] = useState<"PropChange" | "StyleChange" | "Reparent" | "ComponentInsert">(
    "PropChange"
  );
  const [reparentTargetId, setReparentTargetId] = useState("");
  const [insertTagName, setInsertTagName] = useState("");
  const iframeRef = useRef<HTMLIFrameElement>(null);

  // Declared up here, not down beside the other render-time derivations,
  // because `selectStyleProperty`'s own `useCallback` dependency array
  // reads it -- a `const` referenced before its declaration is a real
  // TDZ ReferenceError at first render, not a hoisting no-op.
  const selectedNode = selectedId ? findNode(roots, selectedId) : null;

  // The curated definition for whatever style property is currently
  // named, or `undefined` for the Custom… path -- derived from `propKey`
  // rather than held as separate state, so the two can never disagree
  // (e.g. after a Custom… entry happens to be typed as a real curated
  // name, which correctly upgrades it to the typed control).
  const styleDef = STYLE_PROPERTIES.find((d) => d.name === propKey);

  const refresh = useCallback(async (path: string) => {
    setError(null);
    try {
      const parseResult = (await window.spartan.call("design_parse", { path })) as {
        roots: ComponentNode[];
      };
      setRoots(parseResult.roots);
    } catch (e) {
      setError((e as Error).message);
    }
    try {
      const bundleResult = (await window.spartan.call("design_bundle", { path })) as {
        code: string;
      };
      setBundleCode(bundleResult.code);
    } catch (e) {
      setError((e as Error).message);
    }
  }, []);

  useEffect(() => {
    if (activeFile && isComponentFile(activeFile.path)) {
      refresh(activeFile.path);
    } else {
      setRoots([]);
      setBundleCode(null);
    }
  }, [activeFile?.path, refresh]);

  useEffect(() => {
    const handler = (event: MessageEvent) => {
      if (event.data?.type === "spartan-canvas-click") {
        setSelectedId(event.data.nodeId);
      }
    };
    window.addEventListener("message", handler);
    return () => window.removeEventListener("message", handler);
  }, []);

  // Real, per-kind readiness check -- each structural kind names a
  // different second operand (`reparentTargetId` vs. `insertTagName`)
  // beyond the shared `selectedId`, so "can Apply be pressed" isn't one
  // single condition across all four real edit kinds.
  const canApply =
    !!activeFile &&
    !!selectedId &&
    (editKind === "PropChange" || editKind === "StyleChange"
      ? !!propKey.trim()
      : editKind === "Reparent"
        ? !!reparentTargetId && reparentTargetId !== selectedId
        : !!insertTagName.trim());

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

  const applyEdit = useCallback(async () => {
    if (!activeFile || !selectedId) return;
    let edit: Record<string, unknown>;
    if (editKind === "PropChange") {
      if (!propKey.trim()) return;
      edit = { kind: "PropChange", nodeId: selectedId, prop: propKey, value: propValue };
    } else if (editKind === "StyleChange") {
      if (!propKey.trim()) return;
      edit = { kind: "StyleChange", nodeId: selectedId, property: propKey, value: propValue };
    } else if (editKind === "Reparent") {
      if (!reparentTargetId || reparentTargetId === selectedId) return;
      edit = { kind: "Reparent", nodeId: selectedId, newParentId: reparentTargetId };
    } else {
      if (!insertTagName.trim()) return;
      edit = { kind: "ComponentInsert", parentId: selectedId, tagName: insertTagName.trim() };
    }
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
      onContentChange(activeFile.path, result.source);
      setPropKey("");
      setPropValue("");
      setReparentTargetId("");
      setInsertTagName("");
      await refresh(activeFile.path);
    } catch (e) {
      setError((e as Error).message);
    }
  }, [activeFile, selectedId, propKey, propValue, editKind, reparentTargetId, insertTagName, onContentChange, refresh]);

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
      <div className="design-tree-panel">
        <div className="design-panel-label">Structure</div>
        {roots.map((root) => (
          <TreeNode key={root.id} node={root} depth={0} selectedId={selectedId} onSelect={setSelectedId} />
        ))}
      </div>
      <div className="design-preview">
        {bundleCode ? (
          <iframe
            ref={iframeRef}
            className="design-iframe"
            sandbox="allow-scripts"
            srcDoc={srcDoc}
            title="Live preview"
          />
        ) : (
          <div className="empty-state mono">{error ?? "Bundling..."}</div>
        )}
      </div>
      <div className="design-edit-panel">
        <div className="design-panel-label">Edit</div>
        {selectedNode ? (
          <>
            <div className="design-selected mono">
              &lt;{selectedNode.tagName}&gt; #{selectedNode.id}
            </div>
            <div className="design-edit-kind">
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
            </div>
            {editKind === "PropChange" && (
              <>
                <input
                  className="design-input mono"
                  placeholder="prop name"
                  value={propKey}
                  onChange={(e) => setPropKey(e.target.value)}
                />
                <input
                  className="design-input mono"
                  placeholder="value"
                  value={propValue}
                  onChange={(e) => setPropValue(e.target.value)}
                />
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
            {editKind === "ComponentInsert" && (
              <input
                className="design-input mono"
                placeholder="new tag name (e.g. Button)"
                value={insertTagName}
                onChange={(e) => setInsertTagName(e.target.value)}
              />
            )}
            <button className="leo-btn leo-btn-approve" onClick={applyEdit} disabled={!canApply}>
              Apply
            </button>
          </>
        ) : (
          <div className="leo-status-message mono">Click a node in the tree or preview to select it.</div>
        )}
        {error && <div className="leo-error mono">{error}</div>}
      </div>
    </div>
  );
}
