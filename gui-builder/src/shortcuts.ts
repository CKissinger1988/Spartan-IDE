export type DesignClipboardShortcut = "copySubtree" | "pasteSubtree" | null;

/** Resolves the Design subtree clipboard chord without depending on DOM APIs. */
export function designClipboardShortcut(
  key: string,
  ctrlOrMeta: boolean,
  shift: boolean,
  alt: boolean,
): DesignClipboardShortcut {
  if (!ctrlOrMeta || shift || !alt) return null;
  const normalized = key.toLowerCase();
  if (normalized === "b") return "copySubtree";
  if (normalized === "p") return "pasteSubtree";
  return null;
}
