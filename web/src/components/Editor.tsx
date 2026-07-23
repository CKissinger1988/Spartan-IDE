import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { highlightSource, languageForPath } from "../syntax";
import {
  adjustSnippetStops,
  expandSnippet,
  findSnippet,
  type SnippetSession,
} from "../snippets";
import { writeFileText } from "../fsAccess";
import type { WasmDocument } from "../buffer";

/** Real auto-closing bracket/quote pairs, ported verbatim from
 * `desktop/src/components/Editor.tsx` (task #193) -- see that file's
 * own doc comment for the full real reasoning. */
const OPEN_TO_CLOSE: Record<string, string> = {
  "(": ")",
  "[": "]",
  "{": "}",
  '"': '"',
  "'": "'",
  "`": "`",
};
const CLOSE_CHARS = new Set(Object.values(OPEN_TO_CLOSE));

function offsetToLineChar(content: string, offset: number): { line: number; character: number } {
  const before = content.slice(0, offset);
  const lines = before.split("\n");
  return { line: lines.length - 1, character: lines[lines.length - 1].length };
}

const BRACKET_PAIRS: Record<string, string> = { "(": ")", "[": "]", "{": "}" };
const CLOSE_TO_OPEN: Record<string, string> = { ")": "(", "]": "[", "}": "{" };

function matchBracketForward(content: string, openOffset: number): [number, number] | null {
  const open = content[openOffset];
  const close = BRACKET_PAIRS[open];
  let depth = 0;
  for (let i = openOffset; i < content.length; i++) {
    if (content[i] === open) depth++;
    else if (content[i] === close && --depth === 0) return [openOffset, i];
  }
  return null;
}

function matchBracketBackward(content: string, closeOffset: number): [number, number] | null {
  const close = content[closeOffset];
  const open = CLOSE_TO_OPEN[close];
  let depth = 0;
  for (let i = closeOffset; i >= 0; i--) {
    if (content[i] === close) depth++;
    else if (content[i] === open && --depth === 0) return [closeOffset, i];
  }
  return null;
}

/** Real, pure bracket-pair matcher, ported verbatim from `desktop/`'s own
 * identical wiring -- see that file's own doc comment for the full real
 * reasoning, including the deliberate v1 scope cut (no string/comment
 * awareness) and the real "before cursor is an opener" case a live test
 * caught missing here too. */
function findMatchingBracket(content: string, offset: number): [number, number] | null {
  const atCursor = content[offset];
  if (atCursor && BRACKET_PAIRS[atCursor]) return matchBracketForward(content, offset);
  if (atCursor && CLOSE_TO_OPEN[atCursor]) return matchBracketBackward(content, offset);
  const beforeCursor = content[offset - 1];
  if (beforeCursor && BRACKET_PAIRS[beforeCursor]) return matchBracketForward(content, offset - 1);
  if (beforeCursor && CLOSE_TO_OPEN[beforeCursor]) return matchBracketBackward(content, offset - 1);
  return null;
}

/** Real, per-language line-comment tokens + toggle logic, ported verbatim
 * from `desktop/Editor.tsx`'s own identical wiring -- see that file's own
 * doc comment for the full real reasoning (comment-wins-over-uncomment on
 * a mixed selection, blank lines left untouched, no known token for
 * JSON/CSS/XML/Markdown is a real, honest no-op rather than a guess). */
const LINE_COMMENT_PREFIXES: Record<string, string> = {
  rust: "// ",
  typescript: "// ",
  javascript: "// ",
  kotlin: "// ",
  java: "// ",
  go: "// ",
  csharp: "// ",
  python: "# ",
  bash: "# ",
};

function toggleLineComment(
  content: string,
  selStart: number,
  selEnd: number,
  prefix: string
): { content: string; selectionStart: number; selectionEnd: number } | null {
  if (!prefix) return null;
  const token = prefix.trimEnd();
  const lines = content.split("\n");
  const lineStarts: number[] = new Array(lines.length);
  {
    let acc = 0;
    for (let i = 0; i < lines.length; i++) {
      lineStarts[i] = acc;
      acc += lines[i].length + 1;
    }
  }
  const lineIndexAt = (off: number): number => {
    let idx = 0;
    for (let i = 0; i < lines.length; i++) {
      if (lineStarts[i] <= off) idx = i;
      else break;
    }
    return idx;
  };
  const firstLine = lineIndexAt(selStart);
  let lastLine = lineIndexAt(selEnd);
  if (selEnd > selStart && lastLine > firstLine && lineStarts[lastLine] === selEnd) {
    lastLine -= 1;
  }

  const touchedLines = lines.slice(firstLine, lastLine + 1);
  const nonBlank = touchedLines.filter((l) => l.trim().length > 0);
  const relevant = nonBlank.length > 0 ? nonBlank : touchedLines;
  const allCommented = relevant.every((l) => l.trimStart().startsWith(token));

  const startCol = selStart - lineStarts[firstLine];
  const endCol = selEnd - lineStarts[lastLine];

  const newLines = lines.slice();
  let newStartCol = startCol;
  let newEndCol = endCol;

  for (let i = firstLine; i <= lastLine; i++) {
    const line = lines[i];
    const trimmed = line.trimStart();
    const leadingLen = line.length - trimmed.length;
    let delta = 0;
    if (allCommented) {
      if (trimmed.startsWith(prefix)) {
        newLines[i] = line.slice(0, leadingLen) + trimmed.slice(prefix.length);
        delta = -prefix.length;
      } else if (trimmed.startsWith(token)) {
        newLines[i] = line.slice(0, leadingLen) + trimmed.slice(token.length);
        delta = -token.length;
      }
    } else if (line.trim().length > 0) {
      newLines[i] = line.slice(0, leadingLen) + prefix + trimmed;
      delta = prefix.length;
    }
    if (delta === 0) continue;
    if (i === firstLine && startCol > leadingLen) newStartCol = startCol + delta;
    if (i === lastLine && endCol > leadingLen) newEndCol = endCol + delta;
  }

  const newContent = newLines.join("\n");
  let newFirstLineStart = 0;
  for (let i = 0; i < firstLine; i++) newFirstLineStart += newLines[i].length + 1;
  let newLastLineStart = newFirstLineStart;
  for (let i = firstLine; i < lastLine; i++) newLastLineStart += newLines[i].length + 1;

  const clampedNewStartCol = Math.max(0, Math.min(newStartCol, newLines[firstLine].length));
  const clampedNewEndCol = Math.max(0, Math.min(newEndCol, newLines[lastLine].length));

  return {
    content: newContent,
    selectionStart: newFirstLineStart + clampedNewStartCol,
    selectionEnd: newLastLineStart + clampedNewEndCol,
  };
}

/** Real multi-line indent/outdent, ported verbatim from `desktop/`'s own
 * identical wiring -- see that file's own doc comment for the full real
 * reasoning. */
function reindentLines(
  content: string,
  selStart: number,
  selEnd: number,
  indent: string,
  direction: 1 | -1
): { content: string; selectionStart: number; selectionEnd: number } {
  const lines = content.split("\n");
  const lineStarts: number[] = new Array(lines.length);
  {
    let acc = 0;
    for (let i = 0; i < lines.length; i++) {
      lineStarts[i] = acc;
      acc += lines[i].length + 1;
    }
  }
  const lineIndexAt = (off: number): number => {
    let idx = 0;
    for (let i = 0; i < lines.length; i++) {
      if (lineStarts[i] <= off) idx = i;
      else break;
    }
    return idx;
  };
  const firstLine = lineIndexAt(selStart);
  let lastLine = lineIndexAt(selEnd);
  if (selEnd > selStart && lastLine > firstLine && lineStarts[lastLine] === selEnd) {
    lastLine -= 1;
  }

  const startCol = selStart - lineStarts[firstLine];
  const endCol = selEnd - lineStarts[lastLine];

  const newLines = lines.slice();
  let newStartCol = startCol;
  let newEndCol = endCol;

  for (let i = firstLine; i <= lastLine; i++) {
    const line = lines[i];
    let delta = 0;
    if (direction === 1) {
      newLines[i] = indent + line;
      delta = indent.length;
    } else {
      const trimmed = line.replace(/^[ \t]+/, "");
      const leadingLen = line.length - trimmed.length;
      const stripLen = Math.min(indent.length, leadingLen);
      if (stripLen > 0) {
        newLines[i] = line.slice(stripLen);
        delta = -stripLen;
      }
    }
    if (delta === 0) continue;
    if (i === firstLine) newStartCol = startCol + delta;
    if (i === lastLine) newEndCol = endCol + delta;
  }

  const newContent = newLines.join("\n");
  let newFirstLineStart = 0;
  for (let i = 0; i < firstLine; i++) newFirstLineStart += newLines[i].length + 1;
  let newLastLineStart = newFirstLineStart;
  for (let i = firstLine; i < lastLine; i++) newLastLineStart += newLines[i].length + 1;

  const clampedNewStartCol = Math.max(0, Math.min(newStartCol, newLines[firstLine].length));
  const clampedNewEndCol = Math.max(0, Math.min(newEndCol, newLines[lastLine].length));

  return {
    content: newContent,
    selectionStart: newFirstLineStart + clampedNewStartCol,
    selectionEnd: newLastLineStart + clampedNewEndCol,
  };
}

/** Real line/block duplication (Ctrl+Shift+D), ported verbatim from
 * `desktop/`'s own identical wiring -- see that file's own doc comment
 * for the full real reasoning. */
function duplicateLines(
  content: string,
  selStart: number,
  selEnd: number
): { content: string; selectionStart: number; selectionEnd: number } {
  const lines = content.split("\n");
  const lineStarts: number[] = new Array(lines.length);
  {
    let acc = 0;
    for (let i = 0; i < lines.length; i++) {
      lineStarts[i] = acc;
      acc += lines[i].length + 1;
    }
  }
  const lineIndexAt = (off: number): number => {
    let idx = 0;
    for (let i = 0; i < lines.length; i++) {
      if (lineStarts[i] <= off) idx = i;
      else break;
    }
    return idx;
  };
  const firstLine = lineIndexAt(selStart);
  let lastLine = lineIndexAt(selEnd);
  if (selEnd > selStart && lastLine > firstLine && lineStarts[lastLine] === selEnd) {
    lastLine -= 1;
  }

  const startCol = selStart - lineStarts[firstLine];
  const endCol = selEnd - lineStarts[lastLine];

  const touched = lines.slice(firstLine, lastLine + 1);
  const newLines = [...lines.slice(0, lastLine + 1), ...touched, ...lines.slice(lastLine + 1)];

  const shift = touched.length;
  const newFirstLine = firstLine + shift;
  const newLastLine = lastLine + shift;

  let newDupFirstLineStart = 0;
  for (let i = 0; i < newFirstLine; i++) newDupFirstLineStart += newLines[i].length + 1;
  let newDupLastLineStart = newDupFirstLineStart;
  for (let i = newFirstLine; i < newLastLine; i++) newDupLastLineStart += newLines[i].length + 1;

  return {
    content: newLines.join("\n"),
    selectionStart: newDupFirstLineStart + startCol,
    selectionEnd: newDupLastLineStart + endCol,
  };
}

/** Real move-line-up/down (Alt+Up/Alt+Down), ported verbatim from
 * `desktop/`'s own identical wiring -- see that file's own doc comment
 * for the full real reasoning. */
function moveLines(
  content: string,
  selStart: number,
  selEnd: number,
  direction: 1 | -1
): { content: string; selectionStart: number; selectionEnd: number } | null {
  const lines = content.split("\n");
  const lineStarts: number[] = new Array(lines.length);
  {
    let acc = 0;
    for (let i = 0; i < lines.length; i++) {
      lineStarts[i] = acc;
      acc += lines[i].length + 1;
    }
  }
  const lineIndexAt = (off: number): number => {
    let idx = 0;
    for (let i = 0; i < lines.length; i++) {
      if (lineStarts[i] <= off) idx = i;
      else break;
    }
    return idx;
  };
  const firstLine = lineIndexAt(selStart);
  let lastLine = lineIndexAt(selEnd);
  if (selEnd > selStart && lastLine > firstLine && lineStarts[lastLine] === selEnd) {
    lastLine -= 1;
  }

  if (direction === -1 && firstLine === 0) return null;
  if (direction === 1 && lastLine === lines.length - 1) return null;

  const startCol = selStart - lineStarts[firstLine];
  const endCol = selEnd - lineStarts[lastLine];

  const newLines = lines.slice();
  if (direction === -1) {
    const above = newLines[firstLine - 1];
    newLines.splice(firstLine - 1, 1);
    newLines.splice(lastLine, 0, above);
  } else {
    const below = newLines[lastLine + 1];
    newLines.splice(lastLine + 1, 1);
    newLines.splice(firstLine, 0, below);
  }

  const newFirstLine = firstLine + direction;
  const newLastLine = lastLine + direction;

  let newMoveFirstLineStart = 0;
  for (let i = 0; i < newFirstLine; i++) newMoveFirstLineStart += newLines[i].length + 1;
  let newMoveLastLineStart = newMoveFirstLineStart;
  for (let i = newFirstLine; i < newLastLine; i++) newMoveLastLineStart += newLines[i].length + 1;

  return {
    content: newLines.join("\n"),
    selectionStart: newMoveFirstLineStart + startCol,
    selectionEnd: newMoveLastLineStart + endCol,
  };
}

/** Real "Delete Line" (Ctrl+Shift+K), ported verbatim from `desktop/`'s
 * own identical wiring -- see that file's own doc comment for the full
 * real reasoning. */
function deleteLines(
  content: string,
  selStart: number,
  selEnd: number
): { content: string; selectionStart: number; selectionEnd: number } {
  const lines = content.split("\n");
  const lineStarts: number[] = new Array(lines.length);
  {
    let acc = 0;
    for (let i = 0; i < lines.length; i++) {
      lineStarts[i] = acc;
      acc += lines[i].length + 1;
    }
  }
  const lineIndexAt = (off: number): number => {
    let idx = 0;
    for (let i = 0; i < lines.length; i++) {
      if (lineStarts[i] <= off) idx = i;
      else break;
    }
    return idx;
  };
  const firstLine = lineIndexAt(selStart);
  let lastLine = lineIndexAt(selEnd);
  if (selEnd > selStart && lastLine > firstLine && lineStarts[lastLine] === selEnd) {
    lastLine -= 1;
  }

  const newLines = [...lines.slice(0, firstLine), ...lines.slice(lastLine + 1)];
  const newContent = newLines.join("\n");

  const clampedLine = Math.min(firstLine, Math.max(0, newLines.length - 1));
  let newOffset = 0;
  for (let i = 0; i < clampedLine; i++) newOffset += newLines[i].length + 1;

  return { content: newContent, selectionStart: newOffset, selectionEnd: newOffset };
}

interface FindMatch {
  start: number;
  end: number;
}

/** Real, pure, plain-substring "find," ported verbatim from `desktop/`'s
 * own identical wiring -- see that file's own doc comment for the full
 * real reasoning. */
function findAllMatches(content: string, query: string, caseSensitive: boolean): FindMatch[] {
  if (!query) return [];
  const haystack = caseSensitive ? content : content.toLowerCase();
  const needle = caseSensitive ? query : query.toLowerCase();
  const matches: FindMatch[] = [];
  let idx = haystack.indexOf(needle);
  while (idx !== -1) {
    matches.push({ start: idx, end: idx + needle.length });
    idx = haystack.indexOf(needle, idx + needle.length);
  }
  return matches;
}

/** Real "Replace All," ported verbatim from `desktop/`'s own identical
 * wiring. */
function replaceAllMatches(content: string, matches: FindMatch[], replacement: string): string {
  let result = "";
  let cursor = 0;
  for (const m of matches) {
    result += content.slice(cursor, m.start) + replacement;
    cursor = m.end;
  }
  result += content.slice(cursor);
  return result;
}

export interface OpenFile {
  path: string;
  handle: FileSystemFileHandle;
  doc: WasmDocument;
  content: string;
  dirty: boolean;
}

interface EditorProps {
  file: OpenFile;
  onContentChange: (path: string, content: string, saved?: boolean) => void;
}

/**
 * Real, custom (not Monaco/CodeMirror) text-editing surface for the web
 * app -- the same real "transparent textarea over a highlighted overlay"
 * technique `desktop/src/components/Editor.tsx` already established
 * (§75.62/§75.63), adapted here to edit through the real local
 * `WasmDocument` (client-side, in-browser, no server round trip) instead
 * of `spartan-backend`'s IPC methods, and to save via the real File
 * System Access API instead of a real local `std::fs::write` behind an
 * IPC call.
 *
 * Real undo (Ctrl+Z) and redo (Ctrl+Shift+Z / Ctrl+Y), both backed by
 * `WasmDocument`'s own real `undo`/`redo` -- the crate now builds redo as a
 * thin `redo_stack` layer above `Document`, matching how every other real
 * UI surface in this project already does it (§75.19/§75.62). The former
 * "no redo yet" scope cut is closed.
 */
export default function Editor({ file, onContentChange }: EditorProps): React.ReactElement {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const gutterRef = useRef<HTMLDivElement>(null);
  const highlightRef = useRef<HTMLPreElement>(null);
  const symbolHighlightRef = useRef<HTMLDivElement>(null);
  const [lineCount, setLineCount] = useState(1);
  const prevContentRef = useRef(file.content);
  // Real snippet expansion (P1 backlog), mirroring the two backend-editor
  // surfaces -- Tab navigates a just-expanded snippet's tab stops.
  const snippetSessionRef = useRef<SnippetSession | null>(null);

  /** Real matching-bracket highlighting, ported verbatim from
   * `desktop/`'s own identical wiring -- see that file's own doc comment
   * for the full real reasoning. */
  const [bracketMatch, setBracketMatch] = useState<[number, number] | null>(null);

  /** Real, live font-size zoom (Ctrl+=/Ctrl+-/Ctrl+0), ported verbatim
   * from `desktop/`'s own identical wiring. */
  const [fontSize, setFontSize] = useState(13);
  const zoomFontSize = useCallback((step: number) => {
    setFontSize((prev) => Math.min(32, Math.max(8, prev + step)));
  }, []);

  const charWidth = useMemo(() => {
    const canvas = document.createElement("canvas");
    const ctx = canvas.getContext("2d");
    if (!ctx) return fontSize * 0.6;
    ctx.font = `${fontSize}px "JetBrains Mono", monospace`;
    return ctx.measureText("M").width || fontSize * 0.6;
  }, [fontSize]);
  const lineHeightPx = Math.round(fontSize * 1.54);

  useEffect(() => {
    prevContentRef.current = file.content;
    setLineCount(file.content.split("\n").length);
  }, [file.content]);

  const highlightedHtml = useMemo(
    () => highlightSource(file.content, file.path),
    [file.content, file.path]
  );

  const syncScroll = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    if (gutterRef.current) gutterRef.current.scrollTop = el.scrollTop;
    if (highlightRef.current) {
      highlightRef.current.scrollTop = el.scrollTop;
      highlightRef.current.scrollLeft = el.scrollLeft;
    }
    if (symbolHighlightRef.current) {
      symbolHighlightRef.current.scrollTop = el.scrollTop;
      symbolHighlightRef.current.scrollLeft = el.scrollLeft;
    }
  }, []);

  const handleSelectionChange = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    if (el.selectionStart !== el.selectionEnd) {
      setBracketMatch(null);
      return;
    }
    setBracketMatch(findMatchingBracket(el.value, el.selectionStart));
  }, []);

  /** The real, shared core of applying an edit -- extracted from
   * `handleChange` (below) so keyboard-triggered mutations (Tab-indent,
   * auto-closing brackets/quotes) can call it directly instead of
   * manually mutating `el.value` and dispatching a synthetic native
   * "input" event.
   *
   * **A real, serious bug this refactor fixes, found only by live
   * testing -- not by inspection**: the previous "set `el.value`, then
   * `el.dispatchEvent(new Event('input', {bubbles:true}))`" technique
   * (originally established by the Tab-indent handler, then reused for
   * auto-closing brackets) does NOT reliably reach React's `onChange` in
   * this component. A live Playwright round trip against `web/`'s
   * sibling `BackendEditor.tsx` proved it (same technique, same bug,
   * fixed here too before it could ship broken in every real editing
   * surface this project has): typing Tab or an auto-pairing bracket
   * visibly updated the textarea's raw DOM `.value` (so it *looked*
   * correct on screen), but the real `WasmDocument` was never told
   * about the change (`handleChange` never fired), and a subsequent
   * real Ctrl+S wrote the file via the File System Access API *without*
   * the Tab indent or the auto-paired closing character at all -- a
   * real, silent data-loss bug with nothing to do with auto-closing
   * brackets specifically; Tab alone reproduced it identically. Rather
   * than keep relying on an event-dispatch technique proven unreliable
   * here, this function makes every programmatic mutation call the
   * *same* real update path a genuine native input event already goes
   * through, sidestepping the question of whether React chooses to
   * recognize the synthetic dispatch at all. */
  const applyProgrammaticEdit = useCallback(
    (el: HTMLTextAreaElement, newContent: string, selStart: number, selEnd: number) => {
      el.value = newContent;
      el.setSelectionRange(selStart, selEnd);
      const oldLength = [...prevContentRef.current].length;
      if (snippetSessionRef.current) {
        adjustSnippetStops(snippetSessionRef.current, prevContentRef.current, newContent);
      }
      prevContentRef.current = newContent;
      setLineCount(newContent.split("\n").length);
      try {
        file.doc.replace(0, oldLength, newContent);
      } catch (err) {
        console.error("real WasmDocument.replace failed:", err);
      }
      onContentChange(file.path, newContent);
      setBracketMatch(findMatchingBracket(newContent, selStart));
    },
    [file.doc, file.path, onContentChange]
  );

  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      const el = e.target;
      applyProgrammaticEdit(el, el.value, el.selectionStart, el.selectionEnd);
    },
    [applyProgrammaticEdit]
  );

  /** Real "Go to Line" (Ctrl+G), ported from `desktop/`'s own identical
   * wiring -- see that file's own doc comment for the full real
   * reasoning. This component has no LSP-driven jump helper to reuse (no
   * language server exists in the pure client-side path), so the real
   * line/character-to-offset conversion is inlined here directly. */
  const [gotoLineState, setGotoLineState] = useState<{ value: string } | null>(null);
  const gotoLineInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (gotoLineState) gotoLineInputRef.current?.focus();
  }, [gotoLineState]);

  const confirmGotoLine = useCallback(() => {
    setGotoLineState((prev) => {
      if (!prev) return prev;
      const match = prev.value.trim().match(/^(\d+)(?::(\d+))?$/);
      if (!match) return null;
      const el = textareaRef.current;
      if (!el) return null;
      const lines = prevContentRef.current.split("\n");
      const totalLines = lines.length;
      const requestedLine = Math.max(1, parseInt(match[1], 10));
      const line = Math.min(requestedLine, totalLines) - 1;
      const character = match[2]
        ? Math.min(Math.max(0, parseInt(match[2], 10) - 1), lines[line]?.length ?? 0)
        : 0;
      let offset = 0;
      for (let i = 0; i < line; i++) offset += lines[i].length + 1;
      offset += character;
      el.focus();
      el.setSelectionRange(offset, offset);
      el.scrollTop = Math.max(0, line * 20 - el.clientHeight / 2);
      return null;
    });
  }, []);

  /** Real "Find & Replace" (Ctrl+F / Ctrl+H), ported from `desktop/`'s
   * own identical wiring -- see that file's own doc comment for the full
   * real reasoning, including the two real bugs found and fixed there by
   * live testing (a focus-stealing effect and an Escape-swallowed-by-a-
   * disabled-button case), both fixed identically here from the start
   * rather than rediscovered. This component has no LSP-driven jump
   * helper to reuse (matching `confirmGotoLine`'s own precedent just
   * above), so the real selection/scroll logic is inlined directly. */
  const [findState, setFindState] = useState<{
    query: string;
    replaceQuery: string;
    showReplace: boolean;
    caseSensitive: boolean;
    currentIndex: number;
  } | null>(null);
  const findQueryInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (findState) findQueryInputRef.current?.focus();
  }, [Boolean(findState)]);

  useEffect(() => {
    if (!findState) return;
    const handleGlobalEscape = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setFindState(null);
        textareaRef.current?.focus();
      }
    };
    window.addEventListener("keydown", handleGlobalEscape);
    return () => window.removeEventListener("keydown", handleGlobalEscape);
  }, [Boolean(findState)]);

  const findMatches = useMemo(() => {
    if (!findState || !findState.query) return [];
    return findAllMatches(prevContentRef.current, findState.query, findState.caseSensitive);
  }, [findState?.query, findState?.caseSensitive, file.content]);

  const selectMatch = useCallback(
    (match: FindMatch) => {
      const el = textareaRef.current;
      if (!el) return;
      el.setSelectionRange(match.start, match.end);
      const { line } = offsetToLineChar(prevContentRef.current, match.start);
      el.scrollTop = Math.max(0, line * lineHeightPx - el.clientHeight / 2);
    },
    [lineHeightPx]
  );

  useEffect(() => {
    if (!findState || findMatches.length === 0) return;
    const clamped = Math.min(findState.currentIndex, findMatches.length - 1);
    selectMatch(findMatches[clamped]);
  }, [findMatches, findState?.currentIndex]);

  const findNext = useCallback(
    (direction: 1 | -1) => {
      setFindState((prev) => {
        if (!prev) return prev;
        const count = findMatches.length;
        if (count === 0) return prev;
        const next = (((prev.currentIndex + direction) % count) + count) % count;
        return { ...prev, currentIndex: next };
      });
    },
    [findMatches]
  );

  const replaceCurrentMatch = useCallback(() => {
    setFindState((prev) => {
      if (!prev || findMatches.length === 0) return prev;
      const el = textareaRef.current;
      if (!el) return prev;
      const match = findMatches[Math.min(prev.currentIndex, findMatches.length - 1)];
      const value = el.value;
      const next = value.slice(0, match.start) + prev.replaceQuery + value.slice(match.end);
      const caretPos = match.start + prev.replaceQuery.length;
      applyProgrammaticEdit(el, next, caretPos, caretPos);
      return prev;
    });
  }, [findMatches, applyProgrammaticEdit]);

  const replaceAll = useCallback(() => {
    setFindState((prev) => {
      if (!prev || findMatches.length === 0) return prev;
      const el = textareaRef.current;
      if (!el) return prev;
      const next = replaceAllMatches(el.value, findMatches, prev.replaceQuery);
      applyProgrammaticEdit(el, next, 0, 0);
      return { ...prev, currentIndex: 0 };
    });
  }, [findMatches, applyProgrammaticEdit]);

  const findMatchMarks = useMemo(() => {
    if (!findState || findMatches.length === 0) return [];
    const content = prevContentRef.current;
    const currentIdx = Math.min(findState.currentIndex, findMatches.length - 1);
    const marks: { line: number; startChar: number; endChar: number; isCurrent: boolean }[] = [];
    findMatches.forEach((m, i) => {
      const startPos = offsetToLineChar(content, m.start);
      const endPos = offsetToLineChar(content, m.end);
      if (startPos.line !== endPos.line) return;
      marks.push({
        line: startPos.line,
        startChar: startPos.character,
        endChar: endPos.character,
        isCurrent: i === currentIdx,
      });
    });
    return marks;
  }, [findState, findMatches]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      // Real "Duplicate Line" (Ctrl+Shift+D), ported verbatim from
      // `desktop/`'s own identical wiring.
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "d") {
        e.preventDefault();
        const el = textareaRef.current;
        if (el) {
          const result = duplicateLines(el.value, el.selectionStart, el.selectionEnd);
          applyProgrammaticEdit(el, result.content, result.selectionStart, result.selectionEnd);
        }
        return;
      }
      // Real "Move Line Up/Down" (Alt+Up/Alt+Down), ported verbatim from
      // `desktop/`'s own identical wiring.
      if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
        e.preventDefault();
        const el = textareaRef.current;
        if (el) {
          const result = moveLines(
            el.value,
            el.selectionStart,
            el.selectionEnd,
            e.key === "ArrowUp" ? -1 : 1
          );
          if (result) {
            applyProgrammaticEdit(el, result.content, result.selectionStart, result.selectionEnd);
          }
        }
        return;
      }
      // Real "Delete Line" (Ctrl+Shift+K), ported verbatim from
      // `desktop/`'s own identical wiring.
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        const el = textareaRef.current;
        if (el) {
          const result = deleteLines(el.value, el.selectionStart, el.selectionEnd);
          applyProgrammaticEdit(el, result.content, result.selectionStart, result.selectionEnd);
        }
        return;
      }
      // Real font-size zoom (Ctrl+=/Ctrl++/Ctrl+-/Ctrl+0), ported
      // verbatim from `desktop/`'s own identical wiring.
      if ((e.ctrlKey || e.metaKey) && (e.key === "=" || e.key === "+")) {
        e.preventDefault();
        zoomFontSize(1);
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.key === "-") {
        e.preventDefault();
        zoomFontSize(-1);
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.key === "0") {
        e.preventDefault();
        setFontSize(13);
        return;
      }
      if (
        snippetSessionRef.current &&
        (e.key === "Escape" ||
          e.key === "Home" ||
          e.key === "End" ||
          e.key.startsWith("Arrow"))
      ) {
        snippetSessionRef.current = null;
      }
      if (e.key === "Tab") {
        e.preventDefault();
        const el = textareaRef.current;
        if (!el) return;
        const start = el.selectionStart;
        const end = el.selectionEnd;
        const value = el.value;
        const indent = "  ";
        // Real snippet tab-stop navigation (plain Tab while a session is
        // active): jump to the next placeholder, selecting its text.
        if (!e.shiftKey && snippetSessionRef.current) {
          const session = snippetSessionRef.current;
          session.index += 1;
          const stop = session.stops[session.index];
          if (stop) {
            const s = Math.max(0, Math.min(stop.start, el.value.length));
            const en = Math.max(s, Math.min(stop.end, el.value.length));
            el.setSelectionRange(s, en);
          }
          if (session.index >= session.stops.length - 1) {
            snippetSessionRef.current = null;
          }
          return;
        }
        // Real snippet expansion (plain Tab, collapsed caret, a prefix word
        // matching a snippet for this language).
        if (!e.shiftKey && start === end) {
          const m = /([A-Za-z_]\w*)$/.exec(value.slice(0, start));
          const snip = m ? findSnippet(languageForPath(file.path), m[1]) : null;
          if (snip && m) {
            const expanded = expandSnippet(snip.body);
            const prefixStart = start - m[1].length;
            const next = value.slice(0, prefixStart) + expanded.text + value.slice(end);
            const abs = expanded.stops.map((st) => ({
              start: prefixStart + st.start,
              end: prefixStart + st.end,
            }));
            const first = abs[0];
            applyProgrammaticEdit(el, next, first.start, first.end);
            snippetSessionRef.current = abs.length > 1 ? { stops: abs, index: 0 } : null;
            return;
          }
        }
        // Real multi-line indent/outdent, ported verbatim from
        // `desktop/`'s own identical wiring.
        if (e.shiftKey || start !== end) {
          const result = reindentLines(value, start, end, indent, e.shiftKey ? -1 : 1);
          applyProgrammaticEdit(el, result.content, result.selectionStart, result.selectionEnd);
          return;
        }
        const next = `${value.slice(0, start)}${indent}${value.slice(end)}`;
        applyProgrammaticEdit(el, next, start + indent.length, start + indent.length);
      }
      // Real auto-closing brackets/quotes, ported verbatim from
      // `desktop/`'s own identical wiring.
      if (
        !e.ctrlKey &&
        !e.metaKey &&
        !e.altKey &&
        Object.prototype.hasOwnProperty.call(OPEN_TO_CLOSE, e.key)
      ) {
        const el = textareaRef.current;
        if (el) {
          const start = el.selectionStart;
          const end = el.selectionEnd;
          const value = el.value;
          const closeChar = OPEN_TO_CLOSE[e.key];
          if (start !== end) {
            e.preventDefault();
            const selected = value.slice(start, end);
            const next = `${value.slice(0, start)}${e.key}${selected}${closeChar}${value.slice(end)}`;
            applyProgrammaticEdit(el, next, start + 1, start + 1 + selected.length);
            return;
          }
          const isQuote = e.key === '"' || e.key === "'" || e.key === "`";
          const nextChar = value[start] ?? "";
          const shouldPair = !isQuote || nextChar === "" || /[\s)\]},;]/.test(nextChar);
          if (shouldPair) {
            e.preventDefault();
            const next = `${value.slice(0, start)}${e.key}${closeChar}${value.slice(start)}`;
            applyProgrammaticEdit(el, next, start + 1, start + 1);
            return;
          }
        }
      }
      if (!e.ctrlKey && !e.metaKey && !e.altKey && CLOSE_CHARS.has(e.key)) {
        const el = textareaRef.current;
        if (el && el.selectionStart === el.selectionEnd && el.value[el.selectionStart] === e.key) {
          e.preventDefault();
          el.selectionStart = el.selectionEnd = el.selectionStart + 1;
          return;
        }
      }
      // Real "Go to Line" trigger (Ctrl+G), ported from `desktop/`'s own
      // identical branch.
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "g" && !gotoLineState) {
        e.preventDefault();
        setGotoLineState({ value: "" });
        return;
      }
      // Real Find & Replace dismissal (Escape) reached only if the user
      // clicked back into the main textarea while the find bar stayed
      // open, ported from `desktop/`'s own identical branch.
      if (e.key === "Escape" && findState) {
        setFindState(null);
      }
      // Real "Find" (Ctrl+F) and "Find & Replace" (Ctrl+H) triggers,
      // ported from `desktop/`'s own identical branches.
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "f" && !findState) {
        e.preventDefault();
        const el = textareaRef.current;
        const selected =
          el && el.selectionStart !== el.selectionEnd
            ? el.value.slice(el.selectionStart, el.selectionEnd)
            : "";
        setFindState({
          query: selected,
          replaceQuery: "",
          showReplace: false,
          caseSensitive: false,
          currentIndex: 0,
        });
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "h" && !findState) {
        e.preventDefault();
        const el = textareaRef.current;
        const selected =
          el && el.selectionStart !== el.selectionEnd
            ? el.value.slice(el.selectionStart, el.selectionEnd)
            : "";
        setFindState({
          query: selected,
          replaceQuery: "",
          showReplace: true,
          caseSensitive: false,
          currentIndex: 0,
        });
        return;
      }
      // Real "Toggle Line Comment" (Ctrl+/), ported verbatim from
      // `desktop/Editor.tsx`'s own identical wiring.
      if ((e.ctrlKey || e.metaKey) && e.key === "/") {
        e.preventDefault();
        const el = textareaRef.current;
        const prefix = LINE_COMMENT_PREFIXES[languageForPath(file.path) ?? ""];
        if (el && prefix) {
          const result = toggleLineComment(el.value, el.selectionStart, el.selectionEnd, prefix);
          if (result) {
            applyProgrammaticEdit(el, result.content, result.selectionStart, result.selectionEnd);
          }
        }
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.key === "s") {
        e.preventDefault();
        writeFileText(file.handle, prevContentRef.current)
          .then(() => onContentChange(file.path, prevContentRef.current, true))
          .catch((err: Error) => console.error("real save via File System Access API failed:", err));
      }
      // Real undo (Ctrl+Z) and redo (Ctrl+Shift+Z / Ctrl+Y), both backed by
      // `WasmDocument`'s own real `undo`/`redo` (§75.89's own named "no redo
      // yet" gap is now closed at the crate level).
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "z") {
        e.preventDefault();
        const changed = file.doc.undo();
        if (changed) {
          const restored = file.doc.text();
          prevContentRef.current = restored;
          onContentChange(file.path, restored);
        }
      } else if (
        (e.ctrlKey || e.metaKey) &&
        ((e.shiftKey && e.key.toLowerCase() === "z") || (!e.shiftKey && e.key.toLowerCase() === "y"))
      ) {
        e.preventDefault();
        const changed = file.doc.redo();
        if (changed) {
          const restored = file.doc.text();
          prevContentRef.current = restored;
          onContentChange(file.path, restored);
        }
      }
    },
    [file.doc, file.handle, file.path, onContentChange, gotoLineState, findState, applyProgrammaticEdit]
  );

  const lineNumbers = Array.from({ length: lineCount }, (_, i) => i + 1).join("\n");
  const textStyle: React.CSSProperties = {
    fontSize: `${fontSize}px`,
    lineHeight: `${lineHeightPx}px`,
    tabSize: 2,
    whiteSpace: "pre",
  };

  return (
    <div className="editor-root">
      <div className="editor-gutter mono" ref={gutterRef} style={textStyle}>
        {lineNumbers}
      </div>
      <div className="editor-text-wrap">
        <pre
          className="editor-highlight-layer mono"
          ref={highlightRef}
          aria-hidden="true"
          style={textStyle}
        >
          <code
            className="hljs"
            // Real, deliberate use of dangerouslySetInnerHTML: the HTML
            // comes from this file's own `highlightSource` (a real
            // highlight.js call against the real open document's own
            // content), never from an external/untrusted source -- the
            // same trust boundary desktop/'s own Editor.tsx already
            // assumes for the user's own files.
            dangerouslySetInnerHTML={{ __html: `${highlightedHtml}\n` }}
          />
        </pre>
        <div
          className="editor-symbol-highlight-layer"
          ref={symbolHighlightRef}
          aria-hidden="true"
          style={textStyle}
        >
          {bracketMatch?.map((offset) => {
            const { line, character } = offsetToLineChar(prevContentRef.current, offset);
            return (
              <div
                key={`bracket:${offset}`}
                className="editor-bracket-match-mark"
                style={{
                  top: line * lineHeightPx,
                  left: character * charWidth,
                  width: charWidth,
                  height: lineHeightPx,
                }}
              />
            );
          })}
          {findMatchMarks.map((m, i) => (
            <div
              key={`find:${m.line}:${m.startChar}:${i}`}
              className={`editor-find-match-mark${m.isCurrent ? " editor-find-match-mark-current" : ""}`}
              style={{
                top: m.line * lineHeightPx,
                left: m.startChar * charWidth,
                width: Math.max(2, (m.endChar - m.startChar) * charWidth),
                height: lineHeightPx,
              }}
            />
          ))}
        </div>
        <textarea
          ref={textareaRef}
          className="editor-textarea editor-textarea-overlay mono"
          value={file.content}
          spellCheck={false}
          onChange={handleChange}
          onKeyDown={handleKeyDown}
          onScroll={syncScroll}
          onSelect={handleSelectionChange}
          onClick={handleSelectionChange}
          style={textStyle}
        />
      </div>
      {findState && (
        <div className="editor-find-box mono">
          <div className="editor-find-row">
            <input
              ref={findQueryInputRef}
              className="editor-find-input"
              value={findState.query}
              onChange={(e) =>
                setFindState((prev) =>
                  prev ? { ...prev, query: e.target.value, currentIndex: 0 } : prev
                )
              }
              onKeyDown={(e) => {
                if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "h") {
                  e.preventDefault();
                  setFindState((prev) => (prev ? { ...prev, showReplace: true } : prev));
                  return;
                }
                if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
                  e.preventDefault();
                  findQueryInputRef.current?.select();
                  return;
                }
                e.stopPropagation();
                if (e.key === "Enter") {
                  e.preventDefault();
                  findNext(e.shiftKey ? -1 : 1);
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  setFindState(null);
                  textareaRef.current?.focus();
                }
              }}
              placeholder="Find…"
            />
            <span className="editor-find-count">
              {findState.query
                ? findMatches.length > 0
                  ? `${Math.min(findState.currentIndex, findMatches.length - 1) + 1}/${findMatches.length}`
                  : "No results"
                : ""}
            </span>
            <button
              type="button"
              className={`editor-find-btn${findState.caseSensitive ? " editor-find-btn-active" : ""}`}
              onClick={() =>
                setFindState((prev) =>
                  prev ? { ...prev, caseSensitive: !prev.caseSensitive } : prev
                )
              }
              title="Match case"
            >
              Aa
            </button>
            <button
              type="button"
              className="editor-find-btn"
              onClick={() => findNext(-1)}
              disabled={findMatches.length === 0}
              title="Previous match (Shift+Enter)"
            >
              ↑
            </button>
            <button
              type="button"
              className="editor-find-btn"
              onClick={() => findNext(1)}
              disabled={findMatches.length === 0}
              title="Next match (Enter)"
            >
              ↓
            </button>
            <button
              type="button"
              className={`editor-find-btn${findState.showReplace ? " editor-find-btn-active" : ""}`}
              onClick={() =>
                setFindState((prev) => (prev ? { ...prev, showReplace: !prev.showReplace } : prev))
              }
              title="Toggle Replace (Ctrl+H)"
            >
              ⇄
            </button>
            <button
              type="button"
              className="editor-find-btn"
              onClick={() => {
                setFindState(null);
                textareaRef.current?.focus();
              }}
              title="Close (Escape)"
            >
              ×
            </button>
          </div>
          {findState.showReplace && (
            <div className="editor-find-row">
              <input
                className="editor-find-input"
                value={findState.replaceQuery}
                onChange={(e) =>
                  setFindState((prev) =>
                    prev ? { ...prev, replaceQuery: e.target.value } : prev
                  )
                }
                onKeyDown={(e) => {
                  e.stopPropagation();
                  if (e.key === "Enter") {
                    e.preventDefault();
                    replaceCurrentMatch();
                  } else if (e.key === "Escape") {
                    e.preventDefault();
                    setFindState(null);
                    textareaRef.current?.focus();
                  }
                }}
                placeholder="Replace…"
              />
              <button
                type="button"
                className="editor-find-btn"
                onClick={replaceCurrentMatch}
                disabled={findMatches.length === 0}
              >
                Replace
              </button>
              <button
                type="button"
                className="editor-find-btn"
                onClick={replaceAll}
                disabled={findMatches.length === 0}
              >
                Replace All
              </button>
            </div>
          )}
        </div>
      )}
      {gotoLineState && (
        <div className="editor-gotoline-box mono">
          <input
            ref={gotoLineInputRef}
            className="editor-rename-input"
            value={gotoLineState.value}
            onChange={(e) =>
              setGotoLineState((prev) => (prev ? { ...prev, value: e.target.value } : prev))
            }
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter") {
                e.preventDefault();
                confirmGotoLine();
              } else if (e.key === "Escape") {
                e.preventDefault();
                setGotoLineState(null);
                // Real, deliberate refocus: an Escape-closed overlay
                // otherwise leaves keyboard focus nowhere useful, found by
                // live testing a third Ctrl+G press right after an
                // Escape silently doing nothing -- see `desktop/`'s own
                // identical fix for the full real reasoning.
                textareaRef.current?.focus();
              }
            }}
            onBlur={() => setGotoLineState(null)}
            placeholder={`Go to line (1-${lineCount})…`}
          />
        </div>
      )}
    </div>
  );
}
