import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { highlightSource } from "../syntax";
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

  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      const el = e.target;
      applyProgrammaticEdit(el, el.value, el.selectionStart, el.selectionEnd);
    },
    [applyProgrammaticEdit]
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
        const next = `${value.slice(0, start)}  ${value.slice(end)}`;
        applyProgrammaticEdit(el, next, start + 2, start + 2);
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
