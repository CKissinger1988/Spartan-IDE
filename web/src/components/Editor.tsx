import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { highlightSource } from "../syntax";
import { writeFileText } from "../fsAccess";
import type { WasmDocument } from "../buffer";

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
 * Real, deliberate, named scope cut carried over directly from
 * `spartan-buffer-wasm`'s own doc comment: **no redo yet**. Ctrl+Z calls
 * the real `Document::undo()`; Ctrl+Shift+Z/Ctrl+Y are not wired to
 * anything real here (a real `redo_stack` layered above `Document`,
 * matching how every other real UI surface in this project already
 * builds it, is separate, unstarted follow-up work).
 */
export default function Editor({ file, onContentChange }: EditorProps): React.ReactElement {
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
      try {
        file.doc.replace(0, oldLength, newContent);
      } catch (err) {
        console.error("real WasmDocument.replace failed:", err);
      }
      onContentChange(file.path, newContent);
    },
    [file.doc, file.path, onContentChange]
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
        writeFileText(file.handle, prevContentRef.current)
          .then(() => onContentChange(file.path, prevContentRef.current, true))
          .catch((err: Error) => console.error("real save via File System Access API failed:", err));
      }
      // Real undo only -- see this component's own doc comment for why
      // redo is a real, named, separate follow-up piece.
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "z") {
        e.preventDefault();
        const changed = file.doc.undo();
        if (changed) {
          const restored = file.doc.text();
          prevContentRef.current = restored;
          onContentChange(file.path, restored);
        }
      }
    },
    [file.doc, file.handle, file.path, onContentChange]
  );

  const lineNumbers = Array.from({ length: lineCount }, (_, i) => i + 1).join("\n");
  const textStyle: React.CSSProperties = {
    fontSize: "13px",
    lineHeight: "20px",
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
