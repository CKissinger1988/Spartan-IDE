import type { CanvasEdit } from "./types.js";

export interface AccessibilityFixInput {
  nodeId: string;
  tagName: string;
  props: Record<string, string | null>;
  inspection?: {
    width: number;
    height: number;
  } | null;
}

export interface AccessibilityFix {
  id: string;
  label: string;
  edit: CanvasEdit;
}

const NATIVE_INTERACTIVE = new Set(["a", "button", "input", "select", "textarea"]);
const CUSTOM_INTERACTIVE_ROLES = new Set(["button", "checkbox", "combobox", "link", "menuitem", "option", "radio", "switch", "tab", "textbox"]);

/** Returns only deterministic, user-triggered fixes; ambiguous content stays manual. */
export function suggestAccessibilityFixes(input: AccessibilityFixInput): AccessibilityFix[] {
  const fixes: AccessibilityFix[] = [];
  const tagName = input.tagName.toLowerCase();
  const role = input.props.role?.toLowerCase() ?? "";
  if (tagName === "img" && input.props.alt === null) {
    fixes.push({
      id: "decorative-alt",
      label: "Mark image decorative (alt=\"\")",
      edit: { kind: "PropChange", nodeId: input.nodeId, prop: "alt", value: "", valueType: "string" },
    });
  }
  if (!NATIVE_INTERACTIVE.has(tagName) && CUSTOM_INTERACTIVE_ROLES.has(role)
    && (input.props.tabIndex === null || input.props.tabIndex === "-1")) {
    fixes.push({
      id: "keyboard-focus",
      label: "Make custom role keyboard focusable (tabIndex=0)",
      edit: { kind: "PropChange", nodeId: input.nodeId, prop: "tabIndex", value: "0", valueType: "number" },
    });
  }
  if (CUSTOM_INTERACTIVE_ROLES.has(role) || NATIVE_INTERACTIVE.has(tagName)) {
    if (input.inspection && input.inspection.width < 44) {
      fixes.push({
        id: "minimum-width",
        label: "Set minimum interactive width to 44px",
        edit: { kind: "StyleChange", nodeId: input.nodeId, property: "minWidth", value: "44px" },
      });
    }
    if (input.inspection && input.inspection.height < 44) {
      fixes.push({
        id: "minimum-height",
        label: "Set minimum interactive height to 44px",
        edit: { kind: "StyleChange", nodeId: input.nodeId, property: "minHeight", value: "44px" },
      });
    }
  }
  return fixes;
}
