export const INTERACTION_PRESET_STATES = ["normal", "focus", "hover", "active"] as const;
export const PREVIEW_DATA_STATES = ["normal", "loading", "empty", "error", "populated", "long"] as const;
export const INTERACTION_CLIPBOARD_KIND = "spartan.gui-builder.interaction";
export const INTERACTION_CLIPBOARD_VERSION = 1;

export type PreviewInteractionState = (typeof INTERACTION_PRESET_STATES)[number];
export type PreviewDataState = (typeof PREVIEW_DATA_STATES)[number];

export interface InteractionPreset {
  name: string;
  state: PreviewInteractionState;
  dataState: PreviewDataState;
  updatedAt: number;
}

export interface InteractionClipboard extends InteractionPreset {
  sourcePath: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function isInteractionState(value: unknown): value is PreviewInteractionState {
  return typeof value === "string" && (INTERACTION_PRESET_STATES as readonly string[]).includes(value);
}

function isDataState(value: unknown): value is PreviewDataState {
  return typeof value === "string" && (PREVIEW_DATA_STATES as readonly string[]).includes(value);
}

/** Normalizes saved presets, retaining backward compatibility with the old
 * format that did not persist a data state. */
export function normalizeInteractionPreset(value: unknown): InteractionPreset | null {
  if (!isRecord(value) || typeof value.name !== "string" || !value.name.trim() || !isInteractionState(value.state)) return null;
  return {
    name: value.name.trim(),
    state: value.state,
    dataState: isDataState(value.dataState) ? value.dataState : "normal",
    updatedAt: typeof value.updatedAt === "number" && Number.isFinite(value.updatedAt) ? value.updatedAt : Date.now(),
  };
}

export function normalizeInteractionPresets(value: unknown): InteractionPreset[] {
  return Array.isArray(value)
    ? value.map(normalizeInteractionPreset).filter((preset): preset is InteractionPreset => preset !== null)
    : [];
}

export function buildInteractionClipboard(sourcePath: string, preset: InteractionPreset): InteractionClipboard {
  return { sourcePath, ...preset };
}

export function parseInteractionClipboard(raw: string): InteractionClipboard | null {
  try {
    const value = JSON.parse(raw) as unknown;
    if (!isRecord(value) || value.kind !== INTERACTION_CLIPBOARD_KIND || value.version !== INTERACTION_CLIPBOARD_VERSION || typeof value.sourcePath !== "string") return null;
    const preset = normalizeInteractionPreset(value);
    return preset ? { sourcePath: value.sourcePath, ...preset } : null;
  } catch {
    return null;
  }
}

