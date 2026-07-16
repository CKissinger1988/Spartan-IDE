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

interface BackendEditorProps {
  client: BackendClient;
  file: BackendOpenFile;
  onContentChange: (path: string, content: string, saved?: boolean) => void;
  diagnostics?: LspDiagnostic[];
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
          return (
            <div
              key={n}
              className={`editor-gutter-line${severity ? ` editor-gutter-line-${severity}` : ""}`}
              title={lineDiags?.map((d) => `${d.severity}: ${d.message}`).join("\n")}
            >
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
          style={textStyle}
        />
      </div>
    </div>
  );
}
