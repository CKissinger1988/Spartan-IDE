// Real rope-anchored breakpoint line-shifting -- computes how each real
// breakpoint's 1-indexed line number should move in response to a real
// text edit, closing the gap named in this project's own history since
// §75.8 ("line-number breakpoints instead of rope-anchored persistence").
//
// Deliberately a plain line-array diff (`oldText.split("\n")` vs.
// `newText.split("\n")`, common-prefix/common-suffix), not the
// char-based `computeEdit` in treeSitter.ts -- a line-level diff
// directly answers "which old lines survived unchanged," which is
// exactly what breakpoint-shifting needs, without the extra ambiguity
// a char-based diff would carry about whether an edit split an
// existing line's own content mid-line.
//
// A breakpoint whose line was genuinely touched by a multi-line edit
// (anything beyond a single line's content changing in place) is
// dropped rather than guessed at -- the same honest "don't guess where
// the code went" choice this codebase already makes for git merge
// conflicts (manual resolution, no auto-merge heuristic) and Leo's own
// bounded diff previews.

export interface ShiftableBreakpoint {
  line: number; // 1-indexed
}

interface LineDiff {
  editStartLine0: number;
  /** Inclusive; < editStartLine0 means a pure insertion (nothing old removed). */
  oldChangedEndLine0: number;
  removedCount: number;
  insertedCount: number;
  lineDelta: number;
}

function computeLineDiff(oldText: string, newText: string): LineDiff {
  const oldLines = oldText.split("\n");
  const newLines = newText.split("\n");

  let prefixCount = 0;
  const maxPrefix = Math.min(oldLines.length, newLines.length);
  while (prefixCount < maxPrefix && oldLines[prefixCount] === newLines[prefixCount]) {
    prefixCount++;
  }

  let suffixCount = 0;
  const maxSuffix = Math.min(oldLines.length - prefixCount, newLines.length - prefixCount);
  while (
    suffixCount < maxSuffix &&
    oldLines[oldLines.length - 1 - suffixCount] === newLines[newLines.length - 1 - suffixCount]
  ) {
    suffixCount++;
  }

  const editStartLine0 = prefixCount;
  const oldChangedEndLine0 = oldLines.length - suffixCount - 1;
  const newChangedEndLine0 = newLines.length - suffixCount - 1;
  const removedCount = Math.max(0, oldChangedEndLine0 - editStartLine0 + 1);
  const insertedCount = Math.max(0, newChangedEndLine0 - editStartLine0 + 1);

  return {
    editStartLine0,
    oldChangedEndLine0,
    removedCount,
    insertedCount,
    lineDelta: newLines.length - oldLines.length,
  };
}

/**
 * Real per-breakpoint shift/drop decision for a single 1-indexed line,
 * given an already-computed line diff. Returns the new 1-indexed line,
 * or `null` when the edit genuinely touched (and so invalidated) that
 * exact line.
 */
function shiftBreakpointLine(line1: number, diff: LineDiff): number | null {
  const bpLine0 = line1 - 1;
  if (bpLine0 < diff.editStartLine0) return line1;
  // A plain single-line content edit (typing, no line-count change)
  // leaves the touched line itself in place -- the overwhelmingly
  // common case, and the one that must never silently drop a
  // breakpoint just because the user typed on that exact line.
  if (diff.removedCount === 1 && diff.insertedCount === 1 && bpLine0 === diff.editStartLine0) {
    return line1;
  }
  if (diff.removedCount === 0) {
    // Pure insertion: every old line at/after the insertion point is
    // still present, just shifted down.
    return line1 + diff.lineDelta;
  }
  if (bpLine0 <= diff.oldChangedEndLine0) {
    return null;
  }
  return line1 + diff.lineDelta;
}

/**
 * Real bulk breakpoint-shift entry point. Returns a new array with
 * every breakpoint's line remapped for the given edit, dropping any
 * breakpoint whose exact line was genuinely touched by it. Returns the
 * exact same array reference when nothing shifted or dropped, so a
 * caller can skip a state update on the common no-op case.
 */
export function shiftBreakpointsForEdit<T extends ShiftableBreakpoint>(
  breakpoints: T[],
  oldText: string,
  newText: string
): T[] {
  if (breakpoints.length === 0 || oldText === newText) return breakpoints;
  const diff = computeLineDiff(oldText, newText);
  let changed = false;
  const shifted: T[] = [];
  for (const bp of breakpoints) {
    const newLine = shiftBreakpointLine(bp.line, diff);
    if (newLine === null) {
      changed = true;
      continue;
    }
    if (newLine !== bp.line) {
      changed = true;
      shifted.push({ ...bp, line: newLine });
    } else {
      shifted.push(bp);
    }
  }
  return changed ? shifted : breakpoints;
}
