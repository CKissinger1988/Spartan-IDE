import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { highlightSource } from "../syntax";
import type { BackendClient } from "../backendClient";

export interface BackendOpenFile {
  path: string;
  docId: number;
  content: string;
  dirty: boolean;
}

/** Mirrors `spartan_lsp::LspDiagnostic`'s real, unmodified serde field
 * names -- the same type `desktop/src/components/Editor.tsx` already
 * defines, duplicated here rather than imported since these are two
 * separate npm projects with no shared package between them. */
export interface LspDiagnostic {
  severity: "error" | "warning" | "info" | "hint" | "diagnostic";
  line: number;
  character: number;
  end_line: number;
  end_character: number;
  message: string;
}

const SEVERITY_RANK: Record<string, number> = {
  error: 0,
  warning: 1,
  info: 2,
  hint: 3,
  diagnostic: 4,
};

function worstSeverity(diags: LspDiagnostic[]): string {
  return diags.reduce(
    (worst, d) => (SEVERITY_RANK[d.severity] < SEVERITY_RANK[worst] ? d.severity : worst),
    diags[0].severity
  );
}

/** Real hover-request debounce, ported verbatim from
 * `desktop/src/components/Editor.tsx` (task #134's own follow-up, task
 * #135) -- matches how every real editor's own hover UX settles before
 * firing a request, not on every raw mousemove pixel. */
const HOVER_DELAY_MS = 400;

/** Extracts real, displayable text from a real LSP `Hover` result's
 * `contents` field, which the spec allows in three real shapes:
 * `MarkupContent` (`{kind, value}`), a bare `MarkedString` (a plain
 * string, or `{language, value}`), or an array of `MarkedString`. Returns
 * `null` for a real, honest "no hover info here" (not every position has
 * one -- whitespace, punctuation, an unresolvable symbol). Duplicated
 * from `desktop/`'s own copy rather than imported -- two separate npm
 * projects with no shared package between them, matching this file's
 * own existing precedent for `LspDiagnostic`/`worstSeverity` above. */
function extractHoverText(result: unknown): string | null {
  if (!result || typeof result !== "object") return null;
  const contents = (result as { contents?: unknown }).contents;
  if (contents === null || contents === undefined) return null;
  if (typeof contents === "string") return contents || null;
  if (Array.isArray(contents)) {
    const parts = contents
      .map((c) => (typeof c === "string" ? c : ((c as { value?: string })?.value ?? "")))
      .filter(Boolean);
    return parts.length > 0 ? parts.join("\n\n") : null;
  }
  if (typeof contents === "object" && "value" in (contents as Record<string, unknown>)) {
    const value = (contents as { value?: string }).value;
    return value || null;
  }
  return null;
}

interface HoverState {
  /** Viewport-relative coordinates (from the real triggering mouse
   * event) -- paired with `position: fixed` CSS so this renders next to
   * the cursor regardless of scroll position or DOM nesting. */
  x: number;
  y: number;
  line: number;
  character: number;
  /** `null` while the real request is still in flight -- the tooltip
   * itself only renders once real text has arrived, avoiding a flash of
   * an empty box for the common "no hover info at this position" case. */
  text: string | null;
}

interface BackendEditorProps {
  client: BackendClient;
  file: BackendOpenFile;
  onContentChange: (path: string, content: string, saved?: boolean) => void;
  diagnostics?: LspDiagnostic[];
  /** Real, 1-indexed breakpoint line numbers for this file -- matches
   * `desktop/src/components/Editor.tsx`'s own convention exactly (the
   * gutter's own displayed line numbers, and the real DAP `break_lines`
   * param `App.tsx` sends to `dap_launch` directly, no translation). */
  breakpoints?: number[];
  /** Real click-to-toggle -- `App.tsx` owns the actual breakpoint set. */
  onToggleBreakpoint?: (line: number) => void;
  /** Real, 1-indexed line the active DAP session is currently stopped
   * at for this file, or `null`/`undefined` when nothing is stopped. */
  stoppedLine?: number | null;
}

/**
 * Real backend-mode editing surface -- the counterpart to `Editor.tsx`
 * (File System Access + WASM) for when a devserver is connected. A
 * direct port of `desktop/src/components/Editor.tsx` onto
 * `BackendClient.call`, since `desktop/`'s own Editor already routes
 * every operation through `spartan-backend`'s IPC methods unconditionally
 * -- the same real edit/undo/redo/save/diagnostics wiring applies here
 * verbatim, just reached over a WebSocket instead of Electron IPC.
 *
 * This is what makes real, live LSP diagnostics (§75.6-class backend
 * wiring) usable in this app for the first time -- `Editor.tsx`'s own
 * File System Access + WASM path has no `doc_id` the backend's
 * `lsp_diagnostics` events could ever key off of. Real, named scope cut
 * matching `Editor.tsx`'s own existing precedent: whole-document replace
 * on every keystroke, no syntax-aware editing beyond `syntax.ts`'s
 * lexical highlighter, one file at a time (no tabs).
 */
export default function BackendEditor({
  client,
  file,
  onContentChange,
  diagnostics = [],
  breakpoints = [],
  onToggleBreakpoint,
  stoppedLine = null,
}: BackendEditorProps): React.ReactElement {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const gutterRef = useRef<HTMLDivElement>(null);
  const highlightRef = useRef<HTMLPreElement>(null);
  const [lineCount, setLineCount] = useState(1);
  const prevContentRef = useRef(file.content);

  useEffect(() => {
    prevContentRef.current = file.content;
    setLineCount(file.content.split("\n").length);
  }, [file.content]);

  const highlightedHtml = useMemo(
    () => highlightSource(file.content, file.path),
    [file.content, file.path]
  );

  const diagnosticsByLine = useMemo(() => {
    const map = new Map<number, LspDiagnostic[]>();
    for (const d of diagnostics) {
      const list = map.get(d.line) ?? [];
      list.push(d);
      map.set(d.line, list);
    }
    return map;
  }, [diagnostics]);

  const breakpointSet = useMemo(() => new Set(breakpoints), [breakpoints]);

  const [hoverState, setHoverState] = useState<HoverState | null>(null);
  const hoverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Real, live LSP hover (task #135, the web/ half of task #134's own
  // desktop-then-web follow-up) -- listens for this exact file's own
  // `lsp_hover_result` events over the real `client.onEvent` subscription
  // (the WebSocket-transport counterpart of `desktop/`'s
  // `window.spartan.onEvent`). Self-contained to this component, matching
  // `desktop/`'s own reasoning: a hover tooltip is purely ephemeral,
  // position-driven UI feedback with no other real consumer.
  useEffect(() => {
    const unsubscribe = client.onEvent((e) => {
      if (e.event !== "lsp_hover_result") return;
      const d = e.data as { doc_id: number; line: number; character: number; result: unknown };
      if (d.doc_id !== file.docId) return;
      setHoverState((prev) => {
        // A stale reply for a position the mouse has since moved away
        // from (or a reply for a different file's own request that
        // arrived late) -- ignored, not shown.
        if (!prev || prev.line !== d.line || prev.character !== d.character) return prev;
        const text = extractHoverText(d.result);
        return text ? { ...prev, text } : null;
      });
    });
    return unsubscribe;
  }, [client, file.docId]);

  useEffect(() => {
    return () => {
      if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    };
  }, []);

  // Real monospace glyph width, measured once via a real canvas
  // `measureText` call -- the only way to convert a raw pixel mouse
  // position into an LSP-spec line/character position for a plain
  // `<textarea>`, matching `desktop/`'s own identical technique (this
  // file uses a fixed 13px font, unlike `desktop/`'s own configurable
  // `prefs.fontSize`, since this component has no equivalent settings
  // wiring yet -- `textStyle` below is likewise hardcoded).
  const charWidth = useMemo(() => {
    const canvas = document.createElement("canvas");
    const ctx = canvas.getContext("2d");
    if (!ctx) return 13 * 0.6;
    ctx.font = `13px "JetBrains Mono", monospace`;
    return ctx.measureText("M").width || 13 * 0.6;
  }, []);

  const lineHeightPx = 20;

  const handleMouseMove = useCallback(
    (e: React.MouseEvent<HTMLTextAreaElement>) => {
      if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
      setHoverState(null);
      const el = textareaRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const x = e.clientX - rect.left + el.scrollLeft;
      const y = e.clientY - rect.top + el.scrollTop;
      const line = Math.max(0, Math.floor(y / lineHeightPx));
      const character = Math.max(0, Math.round(x / charWidth));
      const clientX = e.clientX;
      const clientY = e.clientY;
      hoverTimerRef.current = setTimeout(() => {
        setHoverState({ x: clientX, y: clientY, line, character, text: null });
        client
          .call("lsp_hover", { doc_id: file.docId, line, character })
          .catch((err: Error) => console.error("lsp_hover failed:", err));
      }, HOVER_DELAY_MS);
    },
    [client, charWidth, file.docId]
  );

  const handleMouseLeave = useCallback(() => {
    if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    setHoverState(null);
  }, []);

  const syncScroll = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    if (gutterRef.current) gutterRef.current.scrollTop = el.scrollTop;
    if (highlightRef.current) {
      highlightRef.current.scrollTop = el.scrollTop;
      highlightRef.current.scrollLeft = el.scrollLeft;
    }
  }, []);

  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      const newContent = e.target.value;
      const oldLength = [...prevContentRef.current].length;
      prevContentRef.current = newContent;
      setLineCount(newContent.split("\n").length);
      onContentChange(file.path, newContent);
      client
        .call("edit", { doc_id: file.docId, start_char: 0, end_char: oldLength, text: newContent })
        .catch((err: Error) => console.error("edit failed:", err));
    },
    [client, file.docId, file.path, onContentChange]
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Tab") {
        e.preventDefault();
        const el = textareaRef.current;
        if (!el) return;
        const start = el.selectionStart;
        const end = el.selectionEnd;
        const value = el.value;
        el.value = `${value.slice(0, start)}  ${value.slice(end)}`;
        el.selectionStart = el.selectionEnd = start + 2;
        el.dispatchEvent(new Event("input", { bubbles: true }));
      }
      if ((e.ctrlKey || e.metaKey) && e.key === "s") {
        e.preventDefault();
        client
          .call("save_file", { doc_id: file.docId })
          .then(() => onContentChange(file.path, prevContentRef.current, true))
          .catch((err: Error) => console.error("save failed:", err));
      }
      const isRedo =
        ((e.ctrlKey || e.metaKey) && e.key === "y") ||
        ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "z");
      const isUndo = (e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "z";
      if (isUndo || isRedo) {
        e.preventDefault();
        client
          .call(isRedo ? "redo" : "undo", { doc_id: file.docId })
          .then((result) => {
            const r = result as { changed: boolean; content: string };
            if (r.changed) {
              prevContentRef.current = r.content;
              onContentChange(file.path, r.content);
            }
          })
          .catch((err: Error) => console.error(`${isRedo ? "redo" : "undo"} failed:`, err));
      }
    },
    [client, file.docId, file.path, onContentChange]
  );

  const lineNumbers = Array.from({ length: lineCount }, (_, i) => i + 1);
  const textStyle: React.CSSProperties = {
    fontSize: "13px",
    lineHeight: "20px",
    tabSize: 2,
    whiteSpace: "pre",
  };

  return (
    <div className="editor-root">
      <div className="editor-gutter mono" ref={gutterRef} style={textStyle}>
        {lineNumbers.map((n) => {
          const lineDiags = diagnosticsByLine.get(n - 1);
          const severity = lineDiags ? worstSeverity(lineDiags) : null;
          const hasBreakpoint = breakpointSet.has(n);
          const isStopped = stoppedLine === n;
          return (
            <div
              key={n}
              className={`editor-gutter-line${severity ? ` editor-gutter-line-${severity}` : ""}${isStopped ? " editor-gutter-line-stopped" : ""}`}
              title={lineDiags?.map((d) => `${d.severity}: ${d.message}`).join("\n")}
              onClick={() => onToggleBreakpoint?.(n)}
            >
              {onToggleBreakpoint && (
                <span
                  className={`editor-gutter-breakpoint-dot${hasBreakpoint ? " editor-gutter-breakpoint-dot-active" : ""}`}
                />
              )}
              {n}
            </div>
          );
        })}
      </div>
      <div className="editor-text-wrap">
        <pre className="editor-highlight-layer mono" ref={highlightRef} aria-hidden="true" style={textStyle}>
          <code
            className="hljs"
            dangerouslySetInnerHTML={{ __html: `${highlightedHtml}\n` }}
          />
        </pre>
        <textarea
          ref={textareaRef}
          className="editor-textarea editor-textarea-overlay mono"
          value={file.content}
          spellCheck={false}
          onChange={handleChange}
          onKeyDown={handleKeyDown}
          onScroll={syncScroll}
          onMouseMove={handleMouseMove}
          onMouseLeave={handleMouseLeave}
          style={textStyle}
        />
      </div>
      {hoverState?.text && (
        <div
          className="editor-hover-tooltip mono"
          style={{ left: hoverState.x + 12, top: hoverState.y + 16 }}
        >
          {hoverState.text}
        </div>
      )}
    </div>
  );
}
