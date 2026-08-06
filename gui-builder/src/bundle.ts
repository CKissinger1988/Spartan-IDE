/**
 * Real live-rendering bundler (§75.52, task #12) -- the actual mechanism
 * behind turning the text-tree canvas (§75.41/§75.42) into a genuine
 * visual one. Uses `esbuild` (a real, installed dependency, not a mock)
 * to bundle a real component file plus its real imports into a single,
 * self-contained, browser-ready JS file that mounts the component into a
 * `#spartan-root` DOM node.
 *
 * Deliberately resolves every real import (`react`, `lucide-react`, a
 * design-token module, ...) from the *target file's own* directory
 * upward via esbuild's `resolveDir` -- the real project the user has
 * open, never this package's own `node_modules`. This is the technically
 * correct behavior (matching what a real `vite`/`webpack` dev server
 * would do), not a simplification: a project with its own dependencies
 * installed renders correctly; a project missing one produces a real,
 * honest bundling error (surfaced verbatim, never silently swallowed),
 * the same "name the gap, don't fake it" discipline this whole codebase
 * already follows.
 *
 * A real, named v1 assumption: the target file's component is its
 * **default export** (`import Component from "<file>"`). A file using
 * only named exports will bundle successfully (esbuild has no way to
 * know this is wrong at bundle time) but fail at real render time inside
 * the WebView -- the generated entry script's own try/catch reports that
 * failure visibly in the rendered output rather than a silent blank page.
 *
 * As of §75.53, the target file's own real source is annotated with a
 * real `data-spartan-id` attribute per element (via `annotate.ts`,
 * reusing the exact same id-assignment traversal the structural tree
 * uses) through a real esbuild `onLoad` plugin -- so the live-rendered
 * DOM can be clicked and traced back to the exact node id the structural
 * tree/edit panel already use, closing the "no click-to-select on the
 * canvas itself" gap §75.52 named. If annotation itself fails (a real
 * parse error the id-assignment traversal can't handle, distinct from a
 * bundling error), this degrades to bundling the real, unannotated
 * source rather than failing the whole render over a click-to-select-only
 * concern.
 */
import * as esbuild from "esbuild";
import { readFileSync } from "node:fs";
import { dirname, extname, resolve } from "node:path";
import { injectNodeIds } from "./annotate.js";

export interface BundleResult {
  code: string;
}

export interface BundleError {
  error: string;
}

function buildEntrySource(absPath: string): string {
  // JSON.stringify on a path is a real, safe way to embed an arbitrary
  // absolute path (including Windows backslashes) as a valid JS string
  // literal.
  const pathLiteral = JSON.stringify(absPath);
  return `
import React from "react";
import { createRoot } from "react-dom/client";
import Component from ${pathLiteral};

const rootEl = document.getElementById("spartan-root");
try {
  if (typeof Component !== "function") {
    throw new Error(
      "The active file has no default export function/class component " +
      "(only a named export, or none at all) -- the live preview currently " +
      "requires a real default export."
    );
  }
  const root = createRoot(rootEl);
  root.render(React.createElement(Component));

  // Shared drag state, declared before the click relay below because
  // that handler reads \`suppressNextClick\`.
  var dragFromId = null;
  var dragStartX = 0;
  var dragStartY = 0;
  var dragging = false;
  var suppressNextClick = false;
  var hoverEl = null;
  var selectedEls = [];
  var previewStateEl = null;
  var previewStateStyle = null;
  var boxModelOverlay = null;
  var boxModelNode = null;
  var focusedEl = null;
  var focusedHadTabIndex = false;
  var focusedPreviousTabIndex = null;

  // Real persistent selection feedback for the visual canvas. The outline
  // is layered through inline style only, with the exact previous values
  // restored when selection changes, so the user's own component CSS stays
  // authoritative after the builder moves on to another node.
  function clearSelection() {
    selectedEls.forEach(function (element) {
      element.style.outline = element.__spartanPrevSelectedOutline || "";
      element.style.outlineOffset = element.__spartanPrevSelectedOutlineOffset || "";
    });
    selectedEls = [];
  }

  function highlightSelection(nodeIds) {
    clearBoxModel();
    clearSelection();
    var ids = Array.isArray(nodeIds) ? nodeIds : nodeIds ? [nodeIds] : [];
    ids.forEach(function (nodeId) {
      var candidate = document.querySelector('[data-spartan-id="' + nodeId + '"]');
      if (!candidate) return;
      candidate.__spartanPrevSelectedOutline = candidate.style.outline;
      candidate.__spartanPrevSelectedOutlineOffset = candidate.style.outlineOffset;
      candidate.style.outline = "2px solid #2E7DFF";
      candidate.style.outlineOffset = "2px";
      selectedEls.push(candidate);
    });
  }

  function clearBoxModel() {
    if (boxModelOverlay) boxModelOverlay.remove();
    boxModelOverlay = null;
    boxModelNode = null;
  }

  function boxNumber(value) {
    var parsed = parseFloat(value);
    return isFinite(parsed) && parsed > 0 ? parsed : 0;
  }

  function boxPart(parent, left, top, width, height, color, label) {
    if (width <= 0 || height <= 0) return;
    var part = document.createElement("div");
    part.style.position = "absolute";
    part.style.left = left + "px";
    part.style.top = top + "px";
    part.style.width = width + "px";
    part.style.height = height + "px";
    part.style.boxSizing = "border-box";
    part.style.background = color;
    part.style.border = "1px solid rgba(20, 30, 45, 0.35)";
    part.style.color = "#17202f";
    part.style.font = "10px monospace";
    part.style.overflow = "hidden";
    part.style.pointerEvents = "none";
    if (label && width >= 28 && height >= 14) {
      part.textContent = label;
      part.style.padding = "1px 3px";
      part.style.whiteSpace = "nowrap";
    }
    parent.appendChild(part);
  }

  function renderBoxModel(nodeId) {
    clearBoxModel();
    var candidate = nodeId
      ? document.querySelector('[data-spartan-id="' + nodeId + '"]')
      : null;
    if (!candidate) return;
    boxModelNode = candidate;
    var style = window.getComputedStyle(candidate);
    var rect = candidate.getBoundingClientRect();
    var mt = boxNumber(style.marginTop);
    var mr = boxNumber(style.marginRight);
    var mb = boxNumber(style.marginBottom);
    var ml = boxNumber(style.marginLeft);
    var bt = boxNumber(style.borderTopWidth);
    var br = boxNumber(style.borderRightWidth);
    var bb = boxNumber(style.borderBottomWidth);
    var bl = boxNumber(style.borderLeftWidth);
    var pt = boxNumber(style.paddingTop);
    var pr = boxNumber(style.paddingRight);
    var pb = boxNumber(style.paddingBottom);
    var pl = boxNumber(style.paddingLeft);
    var outerWidth = rect.width + ml + mr;
    var outerHeight = rect.height + mt + mb;
    var contentWidth = Math.max(0, rect.width - bl - br - pl - pr);
    var contentHeight = Math.max(0, rect.height - bt - bb - pt - pb);
    var root = document.createElement("div");
    root.style.position = "fixed";
    root.style.left = (rect.left - ml) + "px";
    root.style.top = (rect.top - mt) + "px";
    root.style.width = outerWidth + "px";
    root.style.height = outerHeight + "px";
    root.style.zIndex = "2147483647";
    root.style.pointerEvents = "none";
    root.setAttribute("data-spartan-box-model-overlay", "");
    boxPart(root, 0, 0, outerWidth, mt, "rgba(245, 190, 70, 0.35)", "M " + mt + "px");
    boxPart(root, 0, mt, ml, rect.height, "rgba(245, 190, 70, 0.35)", "M " + ml + "px");
    boxPart(root, outerWidth - mr, mt, mr, rect.height, "rgba(245, 190, 70, 0.35)", "M " + mr + "px");
    boxPart(root, 0, outerHeight - mb, outerWidth, mb, "rgba(245, 190, 70, 0.35)", "M " + mb + "px");
    boxPart(root, ml, mt, rect.width, bt, "rgba(245, 105, 105, 0.45)", "B " + bt + "px");
    boxPart(root, ml, mt + bt, bl, Math.max(0, rect.height - bt - bb), "rgba(245, 105, 105, 0.45)", "B " + bl + "px");
    boxPart(root, ml + rect.width - br, mt + bt, br, Math.max(0, rect.height - bt - bb), "rgba(245, 105, 105, 0.45)", "B " + br + "px");
    boxPart(root, ml, mt + rect.height - bb, rect.width, bb, "rgba(245, 105, 105, 0.45)", "B " + bb + "px");
    boxPart(root, ml + bl, mt + bt, Math.max(0, rect.width - bl - br), pt, "rgba(100, 190, 255, 0.38)", "P " + pt + "px");
    boxPart(root, ml + bl, mt + rect.height - bb - pb, Math.max(0, rect.width - bl - br), pb, "rgba(100, 190, 255, 0.38)", "P " + pb + "px");
    boxPart(root, ml + bl, mt + bt + pt, pl, contentHeight, "rgba(100, 190, 255, 0.38)", "P " + pl + "px");
    boxPart(root, ml + rect.width - br - pr, mt + bt + pt, pr, contentHeight, "rgba(100, 190, 255, 0.38)", "P " + pr + "px");
    boxPart(root, ml + bl + pl, mt + bt + pt, contentWidth, contentHeight, "rgba(120, 235, 160, 0.22)", "content");
    document.body.appendChild(root);
    boxModelOverlay = root;
  }

  function refreshBoxModel() {
    if (boxModelNode) renderBoxModel(boxModelNode.getAttribute("data-spartan-id"));
  }

  window.addEventListener("scroll", refreshBoxModel, true);
  window.addEventListener("resize", refreshBoxModel);

  // Real authored interaction-state previewing. CSS pseudo-classes cannot be
  // forced by dispatching synthetic mouse events, so the sandbox clones the
  // user's own rules with :hover/:active replaced by a temporary attribute
  // selector on the selected element. The temporary sheet and attribute are
  // always removed before another state is applied or the selection changes.
  function clearPreviewState() {
    if (previewStateEl) {
      previewStateEl.removeAttribute("data-spartan-preview-hover");
      previewStateEl.removeAttribute("data-spartan-preview-active");
    }
    if (previewStateStyle) {
      previewStateStyle.remove();
      previewStateStyle = null;
    }
    previewStateEl = null;
  }

  function previewInteractionState(nodeId, state) {
    clearPreviewState();
    if (!nodeId || (state !== "hover" && state !== "active")) return;
    var candidate = document.querySelector('[data-spartan-id="' + nodeId + '"]');
    if (!candidate) return;
    var pseudo = state === "hover" ? ":hover" : ":active";
    var marker = "[data-spartan-preview-" + state + "]";
    var clonedRules = "";
    Array.prototype.forEach.call(document.styleSheets, function (sheet) {
      var rules;
      try {
        rules = sheet.cssRules;
      } catch (error) {
        return;
      }
      if (!rules) return;
      Array.prototype.forEach.call(rules, function (rule) {
        var cssText = rule.cssText || "";
        if (cssText.indexOf(pseudo) !== -1) clonedRules += cssText.split(pseudo).join(marker);
      });
    });
    candidate.setAttribute("data-spartan-preview-" + state, "");
    if (clonedRules) {
      previewStateStyle = document.createElement("style");
      previewStateStyle.setAttribute("data-spartan-preview-state", state);
      previewStateStyle.textContent = clonedRules;
      document.head.appendChild(previewStateStyle);
    }
    previewStateEl = candidate;
  }

  function inspectSelection(nodeId) {
    var candidate = nodeId
      ? document.querySelector('[data-spartan-id="' + nodeId + '"]')
      : null;
    if (!candidate) return;
    var style = window.getComputedStyle(candidate);
    var rect = candidate.getBoundingClientRect();
    window.parent.postMessage({
      type: "spartan-canvas-inspect-result",
      nodeId: nodeId,
      rect: { width: rect.width, height: rect.height },
      styles: {
        display: style.display,
        position: style.position,
        color: style.color,
        backgroundColor: style.backgroundColor,
        fontSize: style.fontSize,
        padding: style.padding,
        margin: style.margin,
        marginTop: style.marginTop,
        marginRight: style.marginRight,
        marginBottom: style.marginBottom,
        marginLeft: style.marginLeft,
        borderTopWidth: style.borderTopWidth,
        borderRightWidth: style.borderRightWidth,
        borderBottomWidth: style.borderBottomWidth,
        borderLeftWidth: style.borderLeftWidth,
        paddingTop: style.paddingTop,
        paddingRight: style.paddingRight,
        paddingBottom: style.paddingBottom,
        paddingLeft: style.paddingLeft,
        flexDirection: style.flexDirection,
        flexWrap: style.flexWrap,
        justifyContent: style.justifyContent,
        alignItems: style.alignItems,
        gap: style.gap,
        gridTemplateColumns: style.gridTemplateColumns,
        gridTemplateRows: style.gridTemplateRows,
        overflowX: style.overflowX,
        overflowY: style.overflowY,
        zIndex: style.zIndex,
      },
    }, "*");
  }

  function blurSelection() {
    if (!focusedEl) return;
    if (document.activeElement === focusedEl) focusedEl.blur();
    if (focusedHadTabIndex) focusedEl.setAttribute("tabindex", focusedPreviousTabIndex);
    else focusedEl.removeAttribute("tabindex");
    focusedEl = null;
    focusedPreviousTabIndex = null;
  }

  function focusSelection(nodeId) {
    blurSelection();
    var candidate = nodeId
      ? document.querySelector('[data-spartan-id="' + nodeId + '"]')
      : null;
    if (!candidate) return;
    focusedEl = candidate;
    focusedHadTabIndex = candidate.hasAttribute("tabindex");
    focusedPreviousTabIndex = candidate.getAttribute("tabindex");
    if (!focusedHadTabIndex) candidate.setAttribute("tabindex", "-1");
    candidate.focus();
  }

  // The parent Design screen uses this same message when a tree row is
  // selected. The message event is the correct cross-origin channel for the
  // sandboxed iframe; no same-origin escape hatch is assumed.
  window.addEventListener("message", function (event) {
    if (event.data && event.data.type === "spartan-canvas-select") {
      highlightSelection(event.data.nodeIds || event.data.nodeId || null);
    } else if (event.data && event.data.type === "spartan-canvas-state") {
      previewInteractionState(event.data.nodeId || null, event.data.state || null);
    } else if (event.data && event.data.type === "spartan-canvas-box-model") {
      if (event.data.visible) renderBoxModel(event.data.nodeId || null);
      else clearBoxModel();
    } else if (event.data && event.data.type === "spartan-canvas-inspect") {
      inspectSelection(event.data.nodeId || null);
    } else if (event.data && event.data.type === "spartan-canvas-focus") {
      focusSelection(event.data.nodeId || null);
    } else if (event.data && event.data.type === "spartan-canvas-blur") {
      blurSelection();
    }
  });

  // Real §75.53 click-to-select relay: this iframe is sandboxed
  // ("allow-scripts" only, no "allow-same-origin"), so it has an opaque
  // origin and can't reach the parent page's own JS directly -- postMessage
  // is the one real, correct channel across that boundary. Delegated on
  // the document root rather than per-element, so it keeps working after
  // React re-renders replace the underlying DOM nodes.
  document.addEventListener("click", function (event) {
    // A drag that ended on this element also fires a click; swallow it
    // so a reparent never doubles as a stray selection change.
    if (suppressNextClick) {
      suppressNextClick = false;
      return;
    }
    const target = event.target && event.target.closest
      ? event.target.closest("[data-spartan-id]")
      : null;
    if (target) {
      highlightSelection(target.getAttribute("data-spartan-id"));
      window.parent.postMessage(
        { type: "spartan-canvas-click", nodeId: target.getAttribute("data-spartan-id"), shiftKey: event.shiftKey },
        "*"
      );
    }
  });

  // Real drag-to-reparent relay (task #279). Deliberately pointer-based
  // rather than HTML5 drag-and-drop: the elements here are rendered by
  // the user's own React component and re-created on every re-render, so
  // a \`draggable="true"\` attribute would have to be continually
  // re-applied, and HTML5 DnD additionally can't be driven by ordinary
  // synthetic mouse input. Plain mousedown/mousemove/mouseup delegated
  // on the document survives every re-render for the same reason the
  // click relay above does.
  // A real pixel threshold, not "did the ids differ" -- on a freeform
  // canvas a plain click routinely lands and releases on the same
  // element, so only genuine movement should count as a drag.
  var DRAG_THRESHOLD_PX = 5;

  function clearHover() {
    if (hoverEl) {
      hoverEl.style.outline = hoverEl.__spartanPrevOutline || "";
      hoverEl = null;
    }
  }

  document.addEventListener("mousedown", function (event) {
    const el = event.target && event.target.closest
      ? event.target.closest("[data-spartan-id]")
      : null;
    if (!el) return;
    dragFromId = el.getAttribute("data-spartan-id");
    dragStartX = event.clientX;
    dragStartY = event.clientY;
    dragging = false;
  });

  document.addEventListener("mousemove", function (event) {
    if (dragFromId === null) return;
    if (!dragging) {
      const dx = event.clientX - dragStartX;
      const dy = event.clientY - dragStartY;
      if (Math.sqrt(dx * dx + dy * dy) < DRAG_THRESHOLD_PX) return;
      dragging = true;
    }
    const over = event.target && event.target.closest
      ? event.target.closest("[data-spartan-id]")
      : null;
    if (over === hoverEl) return;
    clearHover();
    if (over && over.getAttribute("data-spartan-id") !== dragFromId) {
      hoverEl = over;
      hoverEl.__spartanPrevOutline = hoverEl.style.outline;
      hoverEl.style.outline = "2px solid #2E7DFF";
    }
  });

  document.addEventListener("mouseup", function (event) {
    const from = dragFromId;
    const wasDragging = dragging;
    dragFromId = null;
    dragging = false;
    clearHover();
    if (!wasDragging || from === null) return;
    const over = event.target && event.target.closest
      ? event.target.closest("[data-spartan-id]")
      : null;
    if (!over) return;
    const to = over.getAttribute("data-spartan-id");
    if (to === from) return;
    suppressNextClick = true;
    window.parent.postMessage(
      { type: "spartan-canvas-drop", nodeId: from, newParentId: to },
      "*"
    );
  });
} catch (e) {
  rootEl.innerHTML =
    '<div style="color:#e06c75;font-family:monospace;padding:1em;white-space:pre-wrap;">' +
    'Render error: ' + String((e && e.message) || e) +
    '</div>';
}
`;
}

function annotateTargetFilePlugin(absPath: string, sourceOverride?: string): esbuild.Plugin {
  return {
    name: "spartan-annotate-target-file",
    setup(build) {
      build.onLoad({ filter: new RegExp(`^${absPath.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`) }, () => {
        const original = sourceOverride ?? readFileSync(absPath, "utf8");
        let contents = original;
        try {
          contents = injectNodeIds(original);
        } catch {
          // Real, deliberate degrade: a real annotation-step parse
          // failure falls back to the real, unannotated source rather
          // than failing the whole live preview over a click-to-select-
          // only concern -- the component still renders, it just isn't
          // clickable.
        }
        const extension = extname(absPath).toLowerCase();
        const loader = extension === ".tsx" ? "tsx" : extension === ".ts" ? "ts" : "jsx";
        return { contents, loader };
      });
    },
  };
}

async function bundleComponentInternal(
  filePath: string,
  sourceOverride?: string,
): Promise<BundleResult | BundleError> {
  const absPath = resolve(filePath);
  try {
    const extension = extname(absPath).toLowerCase();
    const loader = extension === ".tsx" ? "tsx" : extension === ".ts" ? "ts" : "jsx";
    const result = await esbuild.build({
      stdin: {
        contents: buildEntrySource(absPath),
        resolveDir: dirname(absPath),
        loader: "js",
      },
      bundle: true,
      write: false,
      format: "iife",
      platform: "browser",
      jsx: "automatic",
      logLevel: "silent",
      plugins: [annotateTargetFilePlugin(absPath, sourceOverride)],
    });
    const output = result.outputFiles?.[0];
    if (!output) {
      return { error: "esbuild produced no output file" };
    }
    return { code: output.text };
  } catch (e) {
    return { error: (e as Error).message };
  }
}

export async function bundleComponent(filePath: string): Promise<BundleResult | BundleError> {
  return bundleComponentInternal(filePath);
}

/** Bundles an unsaved in-memory component while resolving imports relative to
 * the real file path. This keeps the preview synchronized with the editor's
 * live document rather than silently falling back to stale disk contents. */
export async function bundleComponentSource(
  filePath: string,
  source: string,
): Promise<BundleResult | BundleError> {
  return bundleComponentInternal(filePath, source);
}
