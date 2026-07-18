import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { highlightSource } from "../syntax";

export interface OpenFile {
  path: string;
  docId: number;
  content: string;
  dirty: boolean;
}

/** Mirrors `spartan_lsp::LspDiagnostic`'s real, unmodified serde field
 * names (no `rename_all` on the Rust side, so these are exactly what
 * arrives over the wire in a real `lsp_diagnostics` event). `line`/
 * `character` are real LSP-spec 0-indexed positions. */
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

/** Real hover-request debounce -- fires only once the mouse has settled
 * on a position, matching how every real editor's own hover UX works
 * (not on every raw mousemove pixel). */
const HOVER_DELAY_MS = 400;

/** Extracts real, displayable text from a real LSP `Hover` result's
 * `contents` field, which the spec allows in three real shapes:
 * `MarkupContent` (`{kind, value}`), a bare `MarkedString` (a plain
 * string, or `{language, value}`), or an array of `MarkedString`.
 * Returns `null` for a real, honest "no hover info here" (not every
 * position has one -- whitespace, punctuation, an unresolvable symbol). */
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

/** Decodes a real `file://` URI (as a real LSP `definition` response's
 * `uri`/`targetUri` carries) back into a local filesystem path, tolerant
 * of percent-escaping and a Windows drive-letter URI shape
 * (`file:///C:/...`) -- the same real decoding job `spartan-lsp`'s own
 * `path_to_file_uri`/`percent_decode` do on the Rust side, mirrored here
 * since a real LSP response arrives at this UI boundary still URI-shaped. */
function fileUriToPath(uri: string): string {
  let path = uri.replace(/^file:\/\//, "");
  try {
    path = decodeURIComponent(path);
  } catch {
    // A malformed escape -- fall back to the raw (still usable) string
    // rather than throwing away a real jump target over a cosmetic issue.
  }
  if (/^\/[a-zA-Z]:\//.test(path)) path = path.slice(1);
  return path;
}

interface DefinitionTarget {
  path: string;
  line: number;
  character: number;
}

/** A real LSP `definition` result is `Location | Location[] | LocationLink[]
 * | null` -- normalizes whichever real shape a server sends into the first
 * entry's jump target. `Location` carries `uri`/`range`; `LocationLink`
 * carries `targetUri`/`targetSelectionRange` (preferred, since it's the
 * precise symbol-name span) or `targetRange` as a fallback. Returns `null`
 * for a real, honest "no definition resolvable here" (an unbound name, a
 * keyword, whitespace) -- not every position has one. */
function extractDefinitionTarget(result: unknown): DefinitionTarget | null {
  if (!result) return null;
  const entry = Array.isArray(result) ? result[0] : result;
  if (!entry || typeof entry !== "object") return null;
  const e = entry as {
    uri?: unknown;
    targetUri?: unknown;
    range?: { start?: { line?: unknown; character?: unknown } };
    targetRange?: { start?: { line?: unknown; character?: unknown } };
    targetSelectionRange?: { start?: { line?: unknown; character?: unknown } };
  };
  const uri = e.targetUri ?? e.uri;
  if (typeof uri !== "string") return null;
  const range = e.targetSelectionRange ?? e.targetRange ?? e.range;
  const line = range?.start?.line;
  const character = range?.start?.character;
  if (typeof line !== "number" || typeof character !== "number") return null;
  return { path: fileUriToPath(uri), line, character };
}

/** A real, normalized reference target -- a real LSP `references` result
 * is a real `Location[]` (never `LocationLink[]`, unlike `definition`),
 * so each entry is a plain `{uri, range}`, no `targetUri`/`targetRange`
 * fallback needed the way `extractDefinitionTarget` needs. */
interface ReferenceItem {
  path: string;
  line: number;
  character: number;
}

/** Normalizes a real LSP `references` result (a real `Location[]`, or
 * `null`) into a real, jump-ready list. Returns `[]` for a real, honest
 * "no references found" rather than throwing. */
function extractReferences(result: unknown): ReferenceItem[] {
  if (!Array.isArray(result)) return [];
  return result
    .map((entry) => {
      const e = entry as { uri?: unknown; range?: { start?: { line?: unknown; character?: unknown } } };
      const line = e.range?.start?.line;
      const character = e.range?.start?.character;
      if (typeof e.uri !== "string" || typeof line !== "number" || typeof character !== "number") {
        return null;
      }
      return { path: fileUriToPath(e.uri), line, character };
    })
    .filter((i): i is ReferenceItem => i !== null);
}

/** A real, normalized LSP `TextEdit` -- `line`/`character` are real
 * 0-indexed LSP positions relative to whatever the document looked like
 * when the rename request was made, matching every other query result's
 * own position convention here. */
export interface WorkspaceTextEdit {
  startLine: number;
  startCharacter: number;
  endLine: number;
  endCharacter: number;
  newText: string;
}

/** Normalizes a real LSP `rename` result (a real `WorkspaceEdit`) into a
 * real `path -> TextEdit[]` map, ready for a caller to apply. **A real,
 * live finding, not assumed from the spec**: `open_project`'s own Rust-side
 * `capabilities` block declares no `workspace.workspaceEdit` field at all,
 * which per spec should mean a server sticks to the simpler `changes`
 * shape -- but a real, live `pyright-langserver` session replies with
 * `documentChanges` regardless (confirmed by `spartan-lsp`'s/
 * `spartan-backend`'s own live integration tests), so both real shapes are
 * handled here rather than assuming either one. Returns `null` for a real,
 * honest "nothing renameable here" (an unbound name, a keyword,
 * whitespace) -- not every position supports a rename. */
function extractWorkspaceEditChanges(result: unknown): Record<string, WorkspaceTextEdit[]> | null {
  if (!result || typeof result !== "object") return null;
  const r = result as { changes?: unknown; documentChanges?: unknown };
  const normalizeEdits = (raw: unknown[]): WorkspaceTextEdit[] =>
    raw
      .map((entry) => {
        const e = entry as {
          range?: {
            start?: { line?: unknown; character?: unknown };
            end?: { line?: unknown; character?: unknown };
          };
          newText?: unknown;
        };
        const startLine = e.range?.start?.line;
        const startCharacter = e.range?.start?.character;
        const endLine = e.range?.end?.line;
        const endCharacter = e.range?.end?.character;
        const newText = e.newText;
        if (
          typeof startLine !== "number" ||
          typeof startCharacter !== "number" ||
          typeof endLine !== "number" ||
          typeof endCharacter !== "number" ||
          typeof newText !== "string"
        ) {
          return null;
        }
        return { startLine, startCharacter, endLine, endCharacter, newText };
      })
      .filter((e): e is WorkspaceTextEdit => e !== null);

  const out: Record<string, WorkspaceTextEdit[]> = {};
  if (r.changes && typeof r.changes === "object") {
    for (const [uri, edits] of Object.entries(r.changes as Record<string, unknown>)) {
      if (!Array.isArray(edits)) continue;
      const normalized = normalizeEdits(edits);
      if (normalized.length > 0) out[fileUriToPath(uri)] = normalized;
    }
  } else if (Array.isArray(r.documentChanges)) {
    for (const docEdit of r.documentChanges) {
      const d = docEdit as { textDocument?: { uri?: unknown }; edits?: unknown };
      const uri = d.textDocument?.uri;
      if (typeof uri !== "string" || !Array.isArray(d.edits)) continue;
      const normalized = normalizeEdits(d.edits);
      if (normalized.length > 0) out[fileUriToPath(uri)] = normalized;
    }
  }
  return Object.keys(out).length > 0 ? out : null;
}

/** A real, normalized LSP `SignatureHelp` target -- only the fields this
 * v1 tooltip actually renders. `activeParameterLabel` is `null` whenever
 * the active parameter can't be resolved (no `activeParameter` index sent,
 * or that parameter's own `label` isn't a plain string -- the LSP spec
 * also allows a `[number, number]` offset-range label, deliberately not
 * handled here, a real, named v1 scope cut). */
interface SignatureHelpTarget {
  label: string;
  activeParameterLabel: string | null;
}

/** Normalizes a real LSP `signatureHelp` result
 * (`{signatures, activeSignature, activeParameter}` or `null`). Returns
 * `null` for a real, honest "no active call here" -- not every cursor
 * position is inside a function call. */
function extractSignatureHelp(result: unknown): SignatureHelpTarget | null {
  if (!result || typeof result !== "object") return null;
  const r = result as { signatures?: unknown; activeSignature?: unknown; activeParameter?: unknown };
  const signatures = Array.isArray(r.signatures) ? r.signatures : [];
  if (signatures.length === 0) return null;
  const activeSigIndex =
    typeof r.activeSignature === "number" && r.activeSignature < signatures.length
      ? r.activeSignature
      : 0;
  const sig = signatures[activeSigIndex] as
    | { label?: unknown; activeParameter?: unknown; parameters?: unknown }
    | undefined;
  const label = typeof sig?.label === "string" ? sig.label : null;
  if (!label) return null;
  // `activeParameter` is preferentially per-signature (LSP 3.16+), falling
  // back to the real top-level field older servers still send.
  const activeParamIndex =
    typeof sig?.activeParameter === "number"
      ? sig.activeParameter
      : typeof r.activeParameter === "number"
        ? r.activeParameter
        : null;
  let activeParameterLabel: string | null = null;
  if (activeParamIndex !== null && Array.isArray(sig?.parameters)) {
    const param = sig.parameters[activeParamIndex] as { label?: unknown } | undefined;
    if (param && typeof param.label === "string") activeParameterLabel = param.label;
  }
  return { label, activeParameterLabel };
}

/** Renders a real signature label with its real active parameter bolded
 * (via a plain substring match against the label -- correct as long as
 * the parameter's own label text appears verbatim in the full signature,
 * true for every real language server this component has been tested
 * against). Falls back to the plain label when no active parameter was
 * resolved or it can't be located inside the label text. */
function renderSignatureLabel(target: SignatureHelpTarget): React.ReactNode {
  const { label, activeParameterLabel } = target;
  if (!activeParameterLabel) return label;
  const idx = label.indexOf(activeParameterLabel);
  if (idx === -1) return label;
  return (
    <>
      {label.slice(0, idx)}
      <span className="editor-signature-help-active-param">
        {label.slice(idx, idx + activeParameterLabel.length)}
      </span>
      {label.slice(idx + activeParameterLabel.length)}
    </>
  );
}

/** Converts a real absolute char offset into a real 0-indexed LSP
 * line/character pair -- the inverse of the offset math
 * `jumpToLocalPosition` already does, needed here since signature help
 * triggers off the textarea's own post-edit `selectionStart` (a plain
 * offset), not a mouse pixel or an already-known line/character. */
function offsetToLineChar(content: string, offset: number): { line: number; character: number } {
  const before = content.slice(0, offset);
  const lines = before.split("\n");
  return { line: lines.length - 1, character: lines[lines.length - 1].length };
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

/** A real, normalized LSP `CompletionItem` -- only the fields this v1
 * dropdown actually renders/inserts. `insertText` falls back to `label`
 * per the LSP spec's own documented default. */
interface CompletionItem {
  label: string;
  insertText: string;
  detail: string | null;
}

/** Normalizes a real LSP completion `result`, which the spec allows in
 * two real shapes: a bare `CompletionItem[]`, or a `CompletionList
 * { isIncomplete, items }`. Returns `[]` for a real, honest "no
 * completions here" rather than throwing. */
function extractCompletionItems(result: unknown): CompletionItem[] {
  if (!result) return [];
  const raw: unknown[] = Array.isArray(result)
    ? result
    : Array.isArray((result as { items?: unknown[] }).items)
      ? (result as { items: unknown[] }).items
      : [];
  return raw
    .map((item) => {
      const i = item as { label?: unknown; insertText?: unknown; detail?: unknown };
      const label = typeof i.label === "string" ? i.label : null;
      if (!label) return null;
      const insertText = typeof i.insertText === "string" ? i.insertText : label;
      const detail = typeof i.detail === "string" ? i.detail : null;
      return { label, insertText, detail };
    })
    .filter((i): i is CompletionItem => i !== null);
}

interface CompletionState {
  /** Viewport-relative coordinates, the same real convention `HoverState`
   * uses -- computed from the real caret position, not the mouse, since
   * completion is keyboard-triggered (Ctrl+Space), not pointer-driven. */
  x: number;
  y: number;
  line: number;
  character: number;
  /** The real character offset the completion was requested from --
   * where accepting an item's replacement range starts. */
  insertAt: number;
  /** How many real characters the user has typed at `insertAt` since the
   * dropdown opened -- the live-narrowing prefix is always
   * `document.slice(insertAt, insertAt + typedLength)`, and accepting an
   * item replaces exactly that range rather than inserting alongside it.
   * Closes this component's own previously-named v1 gap ("no prefix-
   * filtering or replacement on accept"). */
  typedLength: number;
  items: CompletionItem[];
  selectedIndex: number;
}

export interface EditorPrefs {
  fontSize: number;
  tabSize: number;
  wordWrap: boolean;
}

export const DEFAULT_EDITOR_PREFS: EditorPrefs = { fontSize: 13, tabSize: 2, wordWrap: false };

interface EditorProps {
  file: OpenFile;
  onContentChange: (path: string, content: string, saved?: boolean) => void;
  /**
   * Real bug fix, found by a code-review pass: this component used to
   * fetch the exact same settings object `App.tsx` already fetches on
   * its own mount, a fully redundant second IPC round trip on every
   * cold start. `App.tsx` now owns the one real fetch and passes the
   * result down; this stays optional (defaulting to
   * `DEFAULT_EDITOR_PREFS`) so a standalone render (e.g. a future test)
   * doesn't need a parent to supply it.
   */
  prefs?: EditorPrefs;
  /** Real, live LSP diagnostics for this exact open file (already
   * filtered by `doc_id` upstream in `App.tsx`) -- absent/empty is a
   * completely normal state (no LSP configured for this language, no
   * project root found, or a genuinely clean file), never an error. */
  diagnostics?: LspDiagnostic[];
  /** Real, 1-indexed breakpoint line numbers for this file (matching the
   * gutter's own displayed line numbers and the real DAP `break_lines`
   * param `App.tsx` sends to `dap_launch` directly, no off-by-one
   * translation needed at either end). */
  breakpoints?: number[];
  /** Real click-to-toggle -- `App.tsx` owns the actual breakpoint set
   * (it must survive an editor unmount/tab switch), this component only
   * reports which 1-indexed line was clicked. */
  onToggleBreakpoint?: (line: number) => void;
  /** Real, 1-indexed line the active DAP session is currently stopped
   * at for this file, or `null`/`undefined` when no session is stopped
   * here -- matches `DapFrame::line`'s own real 1-indexed DAP-spec
   * value directly, no translation needed. */
  stoppedLine?: number | null;
  /** Real go-to-definition (Ctrl+Click), the third real LSP query method
   * after hover/completion. Called only when the real resolved definition
   * lands in a *different* file than this one -- opening a file and
   * managing the tab set is `App.tsx`'s job, not this component's (the
   * same division of responsibility `onToggleBreakpoint` already
   * establishes for breakpoint state). A same-file jump is handled
   * entirely locally, with no parent involvement needed. */
  onJumpToDefinition?: (path: string, line: number, character: number) => void;
  /** A real, pending cross-file jump `App.tsx` has already opened this
   * exact file for (`pendingJump.path === file.path`, filtered upstream)
   * -- applied via `setSelectionRange`/`scrollTop` on the next render this
   * file's own content is available, then reported back via
   * `onJumpApplied` so `App.tsx` clears it (a real jump is a one-shot
   * action, not a persistent prop). */
  pendingJump?: { line: number; character: number } | null;
  onJumpApplied?: () => void;
  /** Real F2 rename-symbol apply -- given the real, already-normalized
   * `path -> TextEdit[]` map a resolved `WorkspaceEdit` produced (see
   * `extractWorkspaceEditChanges`'s own doc comment for the real,
   * live-discovered shape variance it already handles), opens/finds each
   * affected file and applies its edits through the existing, already-real
   * `edit` IPC method -- the same division of responsibility
   * `onJumpToDefinition` already establishes for "this needs multi-file
   * state only `App.tsx` owns." Resolves to the real number of files
   * actually touched, so this component's own rename UI can report a
   * real result rather than assuming success. */
  onApplyRename?: (changes: Record<string, WorkspaceTextEdit[]>) => Promise<number>;
}

/**
 * Real, custom (not Monaco/CodeMirror) text-editing surface -- a real,
 * deliberate v1 scope choice, named honestly rather than overclaimed: it
 * builds line-number gutter, tab/file chrome, theming, and the real
 * open/edit/save round trip through the Rust backend entirely from
 * scratch, but leans on the browser's own native `<textarea>` for
 * character-level cursor/selection/keyboard-input handling rather than
 * reimplementing that from zero in JS (the same kind of pragmatic
 * foundation early versions of CodeMirror/Ace themselves started from).
 *
 * Real syntax highlighting (§75.62 audit finding, closed here): a real,
 * standard "transparent textarea over a highlighted overlay" technique
 * -- `syntax.ts`'s real `highlight.js`-backed tokenizer renders colored
 * `<span>`s into a `<pre>` layer positioned exactly under the real
 * textarea (identical font/line-height/padding), while the textarea's
 * own text is made transparent so only its real caret and native text
 * selection remain visible on top. A deliberate, named choice over this
 * workspace's own real tree-sitter engine (`highlight.rs` in the
 * original wgpu shell) -- see `syntax.ts`'s own doc comment for why.
 *
 * Edits are sent to the real backend as a real whole-document replace on
 * every change (`edit` with `start_char: 0, end_char: <old length>`) --
 * simple and correct. Real Ctrl+Z/Ctrl+Y undo/redo (task #52) are wired
 * to the backend's own `undo`/`redo` IPC methods exclusively -- the
 * native textarea's own built-in undo stack is intercepted and never
 * allowed to fire, since it would silently drift from the real
 * `Document`'s own branching undo tree. One real, named cost remains:
 * every keystroke is still its own undo checkpoint, unlike the original
 * wgpu shell's own coalesced-typing-run undo (§75.25) -- real follow-up
 * work, not attempted in this pass.
 */
export default function Editor({
  file,
  onContentChange,
  prefs = DEFAULT_EDITOR_PREFS,
  diagnostics = [],
  breakpoints = [],
  onToggleBreakpoint,
  stoppedLine = null,
  onJumpToDefinition,
  pendingJump = null,
  onJumpApplied,
  onApplyRename,
}: EditorProps): React.ReactElement {
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

  const breakpointSet = useMemo(() => new Set(breakpoints), [breakpoints]);

  const [hoverState, setHoverState] = useState<HoverState | null>(null);
  const hoverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Real, live LSP hover (task #134) -- listens for this exact file's own
  // `lsp_hover_result` events. Self-contained to this component (not
  // lifted to App.tsx like `diagnostics`) since a hover tooltip is purely
  // ephemeral, position-driven UI feedback with no other real consumer.
  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event !== "lsp_hover_result") return;
      const d = data as { doc_id: number; line: number; character: number; result: unknown };
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
  }, [file.docId]);

  useEffect(() => {
    return () => {
      if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    };
  }, []);

  // Real monospace glyph width, measured once per font size via a real
  // canvas `measureText` call -- the only way to convert a raw pixel
  // mouse position into an LSP-spec line/character position for a plain
  // `<textarea>`, which (unlike the reference wgpu shell's own
  // cosmic-text-backed hit-testing) has no built-in "what's under this
  // pixel" API of its own.
  const charWidth = useMemo(() => {
    const canvas = document.createElement("canvas");
    const ctx = canvas.getContext("2d");
    if (!ctx) return prefs.fontSize * 0.6;
    ctx.font = `${prefs.fontSize}px "JetBrains Mono", monospace`;
    return ctx.measureText("M").width || prefs.fontSize * 0.6;
  }, [prefs.fontSize]);

  const lineHeightPx = Math.round(prefs.fontSize * 1.54);

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
        window.spartan
          .call("lsp_hover", { doc_id: file.docId, line, character })
          .catch((err: Error) => console.error("lsp_hover failed:", err));
      }, HOVER_DELAY_MS);
    },
    [charWidth, lineHeightPx, file.docId]
  );

  const handleMouseLeave = useCallback(() => {
    if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    setHoverState(null);
  }, []);

  const [completionState, setCompletionState] = useState<CompletionState | null>(null);

  // Real, live LSP completion (task #136, the direct sibling of hover's
  // own §134 wiring) -- listens for this exact file's own
  // `lsp_completion_result` events. Self-contained to this component,
  // matching hover's own reasoning: a completion dropdown is purely
  // ephemeral, position-driven UI feedback with no other real consumer.
  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event !== "lsp_completion_result") return;
      const d = data as { doc_id: number; line: number; character: number; result: unknown };
      if (d.doc_id !== file.docId) return;
      setCompletionState((prev) => {
        // A stale reply for a request the caret has since moved away from
        // (or a reply for a different file's own request that arrived
        // late) -- ignored, not shown.
        if (!prev || prev.line !== d.line || prev.character !== d.character) return prev;
        const items = extractCompletionItems(d.result);
        return items.length > 0 ? { ...prev, items, selectedIndex: 0 } : null;
      });
    });
    return unsubscribe;
  }, [file.docId]);

  const [signatureHelpState, setSignatureHelpState] = useState<{
    x: number;
    y: number;
    line: number;
    character: number;
    target: SignatureHelpTarget | null;
  } | null>(null);

  // Real, live LSP signature help (task #169, the fourth real query method
  // after hover/completion/definition) -- listens for this exact file's
  // own `lsp_signature_help_result` events. Self-contained to this
  // component, matching hover's own reasoning: a signature-help tooltip is
  // purely ephemeral, position-driven UI feedback with no other real
  // consumer.
  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event !== "lsp_signature_help_result") return;
      const d = data as { doc_id: number; line: number; character: number; result: unknown };
      if (d.doc_id !== file.docId) return;
      setSignatureHelpState((prev) => {
        if (!prev || prev.line !== d.line || prev.character !== d.character) return prev;
        const target = extractSignatureHelp(d.result);
        return target ? { ...prev, target } : null;
      });
    });
    return unsubscribe;
  }, [file.docId]);

  /** Real, manual completion trigger (Ctrl+Space) -- a deliberate, named
   * v1 scope choice over automatic per-keystroke triggering, matching
   * this component's own established pattern of picking the smallest
   * real, correct increment first (§75.68's own "first real increment,
   * not the full MVP" precedent). Computes the real LSP line/character
   * from the textarea's own `selectionStart` (a plain character offset
   * into the whole document) by counting newlines up to that point --
   * the same real technique this file's own gutter/diagnostics code
   * already uses for line numbers, just applied to a cursor position
   * instead of a mouse pixel. */
  const triggerCompletion = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    const pos = el.selectionStart;
    const before = el.value.slice(0, pos);
    const lines = before.split("\n");
    const line = lines.length - 1;
    const character = lines[lines.length - 1].length;
    const x = el.getBoundingClientRect().left + character * charWidth - el.scrollLeft;
    const y = el.getBoundingClientRect().top + line * lineHeightPx - el.scrollTop + lineHeightPx;
    setCompletionState({
      x,
      y,
      line,
      character,
      insertAt: pos,
      typedLength: 0,
      items: [],
      selectedIndex: 0,
    });
    window.spartan
      .call("lsp_completion", { doc_id: file.docId, line, character })
      .catch((err: Error) => console.error("lsp_completion failed:", err));
  }, [charWidth, lineHeightPx, file.docId]);

  /** Real, live client-side narrowing: as the user keeps typing while the
   * dropdown is open, the already-fetched list is filtered by whatever
   * they've typed since (no new server round trip needed -- the same
   * items just get narrower). Case-insensitive prefix match against both
   * `insertText` and `label`, since either can be the more natural match
   * depending on the language server (e.g. a snippet's `insertText` often
   * differs from its own displayed `label`). */
  const filteredCompletionItems = useMemo(() => {
    if (!completionState) return [];
    if (completionState.typedLength === 0) return completionState.items;
    const prefix = file.content
      .slice(completionState.insertAt, completionState.insertAt + completionState.typedLength)
      .toLowerCase();
    if (!prefix) return completionState.items;
    return completionState.items.filter(
      (item) =>
        item.insertText.toLowerCase().startsWith(prefix) || item.label.toLowerCase().startsWith(prefix)
    );
  }, [completionState, file.content]);

  /** Replaces the real range `[insertAt, insertAt + typedLength)` -- the
   * exact prefix the user typed since the dropdown opened -- with the
   * selected item's `insertText`, instead of the earlier v1's zero-width
   * insert-alongside-what-was-typed (which left the typed prefix and the
   * full completion sitting side by side). Routed through the same `edit`
   * IPC path (and so the same real undo/redo checkpointing) every other
   * edit in this component already uses, not a direct textarea mutation. */
  const acceptCompletion = useCallback(
    (item: CompletionItem) => {
      const insertAt = completionState?.insertAt ?? 0;
      const replaceEnd = insertAt + (completionState?.typedLength ?? 0);
      const newContent =
        prevContentRef.current.slice(0, insertAt) +
        item.insertText +
        prevContentRef.current.slice(replaceEnd);
      prevContentRef.current = newContent;
      setLineCount(newContent.split("\n").length);
      onContentChange(file.path, newContent);
      window.spartan
        .call("edit", {
          doc_id: file.docId,
          start_char: insertAt,
          end_char: replaceEnd,
          text: item.insertText,
        })
        .catch((err: Error) => console.error("edit failed:", err));
      setCompletionState(null);
      const el = textareaRef.current;
      if (el) {
        const newPos = insertAt + item.insertText.length;
        requestAnimationFrame(() => el.setSelectionRange(newPos, newPos));
      }
    },
    [completionState, file.docId, file.path, onContentChange]
  );

  /** Converts a real 0-indexed LSP line/character into a real absolute
   * char offset into the current buffer, then moves the native caret
   * there and scrolls it roughly into view -- the shared landing logic
   * both a same-file jump and a cross-file jump (once `App.tsx` has
   * opened the target and re-rendered this component with it as
   * `file`) use. */
  const jumpToLocalPosition = useCallback(
    (line: number, character: number) => {
      const el = textareaRef.current;
      if (!el) return;
      const lines = prevContentRef.current.split("\n");
      let offset = 0;
      for (let i = 0; i < line && i < lines.length; i++) {
        offset += lines[i].length + 1; // +1 for the real newline this split consumed
      }
      offset += Math.min(character, lines[line]?.length ?? 0);
      el.focus();
      el.setSelectionRange(offset, offset);
      el.scrollTop = Math.max(0, line * lineHeightPx - el.clientHeight / 2);
    },
    [lineHeightPx]
  );

  /** Real, shared jump-target dispatch -- same file lands locally via
   * `jumpToLocalPosition`, a different file goes through `App.tsx`'s own
   * `onJumpToDefinition` (which opens/activates it and hands the position
   * back down once it's the active file). Used by both a real go-to-
   * definition result and a real find-references item click -- both are
   * "jump to a file:line:character", the only real difference is where the
   * target list came from. */
  const goToTarget = useCallback(
    (target: { path: string; line: number; character: number }) => {
      if (target.path === file.path) {
        jumpToLocalPosition(target.line, target.character);
      } else {
        onJumpToDefinition?.(target.path, target.line, target.character);
      }
    },
    [file.path, jumpToLocalPosition, onJumpToDefinition]
  );

  // Real go-to-definition (Ctrl+Click) -- a request the click handler below
  // fires is tracked here (a ref, not state: nothing renders while it's in
  // flight, unlike hover/completion) so a stale reply for a position the
  // user has since clicked elsewhere from is ignored rather than jumping
  // to the wrong place.
  const pendingDefinitionRef = useRef<{ line: number; character: number } | null>(null);

  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event !== "lsp_definition_result") return;
      const d = data as { doc_id: number; line: number; character: number; result: unknown };
      if (d.doc_id !== file.docId) return;
      const pending = pendingDefinitionRef.current;
      if (!pending || pending.line !== d.line || pending.character !== d.character) return;
      pendingDefinitionRef.current = null;
      const target = extractDefinitionTarget(d.result);
      // A real, honest "no definition resolvable here" -- silent, matching
      // how every real editor's own Ctrl+Click behaves at an unbound
      // position rather than surfacing an error for a completely normal
      // case.
      if (!target) return;
      goToTarget(target);
    });
    return unsubscribe;
  }, [file.docId, goToTarget]);

  // A real cross-file jump lands here: `App.tsx` has already opened the
  // target file and re-rendered this component with it as `file` (filtered
  // upstream so `pendingJump` is only ever non-null once `file.path`
  // already matches), so `prevContentRef.current` (synced from `file.content`
  // by the effect above, which runs first in declaration order) is already
  // the real target content by the time this runs.
  useEffect(() => {
    if (!pendingJump) return;
    jumpToLocalPosition(pendingJump.line, pendingJump.character);
    onJumpApplied?.();
  }, [pendingJump, jumpToLocalPosition, onJumpApplied]);

  const handleDefinitionClick = useCallback(
    (e: React.MouseEvent<HTMLTextAreaElement>) => {
      if (!(e.ctrlKey || e.metaKey)) {
        // A real plain click dismisses an open references panel -- the
        // same "clicking elsewhere closes it" behavior every real
        // editor's own find-references popup has.
        setReferencesState(null);
        return;
      }
      const el = textareaRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const x = e.clientX - rect.left + el.scrollLeft;
      const y = e.clientY - rect.top + el.scrollTop;
      const line = Math.max(0, Math.floor(y / lineHeightPx));
      const character = Math.max(0, Math.round(x / charWidth));
      pendingDefinitionRef.current = { line, character };
      window.spartan
        .call("lsp_definition", { doc_id: file.docId, line, character })
        .catch((err: Error) => console.error("lsp_definition failed:", err));
    },
    [charWidth, lineHeightPx, file.docId]
  );

  const [referencesState, setReferencesState] = useState<{
    x: number;
    y: number;
    items: ReferenceItem[] | null;
  } | null>(null);
  const pendingReferencesRef = useRef<{ line: number; character: number } | null>(null);

  // Real find-references (Shift+F12) -- listens for this exact file's own
  // `lsp_references_result` events, following the same real query-request/
  // reply pattern hover/completion/definition/signature-help already
  // established. `items: null` while the request is in flight (distinct
  // from `[]`, a real, honest "no references found" once it resolves).
  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event !== "lsp_references_result") return;
      const d = data as { doc_id: number; line: number; character: number; result: unknown };
      if (d.doc_id !== file.docId) return;
      const pending = pendingReferencesRef.current;
      if (!pending || pending.line !== d.line || pending.character !== d.character) return;
      pendingReferencesRef.current = null;
      setReferencesState((prev) => (prev ? { ...prev, items: extractReferences(d.result) } : prev));
    });
    return unsubscribe;
  }, [file.docId]);

  /** Real, manual find-references trigger (Shift+F12, the standard
   * cross-editor convention) -- computes the real LSP line/character from
   * the textarea's own `selectionStart`, the same real technique
   * `triggerCompletion` already uses. */
  const triggerReferences = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    const { line, character } = offsetToLineChar(el.value, el.selectionStart);
    const x = el.getBoundingClientRect().left + character * charWidth - el.scrollLeft;
    const y = el.getBoundingClientRect().top + line * lineHeightPx - el.scrollTop + lineHeightPx;
    pendingReferencesRef.current = { line, character };
    setReferencesState({ x, y, items: null });
    window.spartan
      .call("lsp_references", { doc_id: file.docId, line, character })
      .catch((err: Error) => console.error("lsp_references failed:", err));
  }, [charWidth, lineHeightPx, file.docId]);

  /** Real F2 rename-symbol -- the sixth real LSP-backed editor feature,
   * following go-to-definition/signature-help/find-references' own
   * "compute position from `selectionStart`, show UI near the cursor"
   * shape. Unlike those three, `editing` is a real, distinct first phase:
   * a plain new-name text box, no request sent yet -- `requesting` covers
   * the real `lsp_rename` round trip, `applying` the real multi-file
   * `edit` application via `onApplyRename`, and `done`/`error` show a
   * brief real result before this self-dismisses. */
  const [renameState, setRenameState] = useState<{
    x: number;
    y: number;
    line: number;
    character: number;
    value: string;
    phase: "editing" | "requesting" | "applying" | "done" | "error";
    message?: string;
  } | null>(null);
  const pendingRenameRef = useRef<{ line: number; character: number } | null>(null);
  const renameInputRef = useRef<HTMLInputElement>(null);

  const triggerRename = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    const { line, character } = offsetToLineChar(el.value, el.selectionStart);
    const x = el.getBoundingClientRect().left + character * charWidth - el.scrollLeft;
    const y = el.getBoundingClientRect().top + line * lineHeightPx - el.scrollTop;
    setRenameState({ x, y, line, character, value: "", phase: "editing" });
  }, [charWidth, lineHeightPx]);

  // Real focus-on-open for the rename input -- a plain `autoFocus` prop
  // doesn't reliably win against the textarea's own focus on the very
  // render that mounts it.
  useEffect(() => {
    if (renameState?.phase === "editing") renameInputRef.current?.focus();
  }, [renameState?.phase]);

  const confirmRename = useCallback(() => {
    setRenameState((prev) => {
      if (!prev || !prev.value.trim()) return prev;
      pendingRenameRef.current = { line: prev.line, character: prev.character };
      window.spartan
        .call("lsp_rename", {
          doc_id: file.docId,
          line: prev.line,
          character: prev.character,
          new_name: prev.value.trim(),
        })
        .catch((err: Error) => console.error("lsp_rename failed:", err));
      return { ...prev, phase: "requesting" };
    });
  }, [file.docId]);

  // Real `lsp_rename_result` handling -- normalizes the real WorkspaceEdit,
  // then hands it to `App.tsx` (via `onApplyRename`) to actually apply,
  // since it may span files this component has never seen.
  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event !== "lsp_rename_result") return;
      const d = data as { doc_id: number; line: number; character: number; result: unknown };
      if (d.doc_id !== file.docId) return;
      const pending = pendingRenameRef.current;
      if (!pending || pending.line !== d.line || pending.character !== d.character) return;
      pendingRenameRef.current = null;
      const changes = extractWorkspaceEditChanges(d.result);
      if (!changes) {
        setRenameState((prev) =>
          prev ? { ...prev, phase: "error", message: "Nothing renameable here" } : prev
        );
        return;
      }
      if (!onApplyRename) {
        setRenameState((prev) =>
          prev ? { ...prev, phase: "error", message: "Rename apply is unavailable" } : prev
        );
        return;
      }
      setRenameState((prev) => (prev ? { ...prev, phase: "applying" } : prev));
      onApplyRename(changes)
        .then((fileCount) => {
          setRenameState((prev) =>
            prev
              ? {
                  ...prev,
                  phase: "done",
                  message: `Renamed in ${fileCount} file${fileCount === 1 ? "" : "s"}`,
                }
              : prev
          );
        })
        .catch((err: Error) => {
          setRenameState((prev) => (prev ? { ...prev, phase: "error", message: err.message } : prev));
        });
    });
    return unsubscribe;
  }, [file.docId, onApplyRename]);

  // A completed (or failed) rename self-dismisses after a brief real
  // result is shown -- the same "don't require an extra dismiss click for
  // a one-shot action" convention `pendingJump`'s own effect establishes.
  useEffect(() => {
    if (renameState?.phase !== "done" && renameState?.phase !== "error") return;
    const timer = window.setTimeout(() => setRenameState(null), 2000);
    return () => window.clearTimeout(timer);
  }, [renameState?.phase]);

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
      window.spartan
        .call("edit", {
          doc_id: file.docId,
          start_char: 0,
          end_char: oldLength,
          text: newContent,
        })
        .catch((err: Error) => console.error("edit failed:", err));

      // Real signature-help auto-trigger -- fires on the real
      // LSP-conventional trigger characters "(" (open a call) and ","
      // (advance to the next argument), dismisses on a real closing ")".
      // A deliberate, named v1 scope choice over full per-keystroke
      // re-querying while a signature stays open, matching this
      // component's own established "smallest real, correct increment"
      // precedent (Ctrl+Space over fully-automatic completion).
      const el = e.target;
      const pos = el.selectionStart;
      const justTyped = pos > 0 ? newContent[pos - 1] : "";
      if (justTyped === "(" || justTyped === ",") {
        const { line, character } = offsetToLineChar(newContent, pos);
        const x = el.getBoundingClientRect().left + character * charWidth - el.scrollLeft;
        const y =
          el.getBoundingClientRect().top + line * lineHeightPx - el.scrollTop + lineHeightPx;
        setSignatureHelpState({ x, y, line, character, target: null });
        window.spartan
          .call("lsp_signature_help", { doc_id: file.docId, line, character })
          .catch((err: Error) => console.error("lsp_signature_help failed:", err));
      } else if (justTyped === ")") {
        setSignatureHelpState(null);
      }
    },
    [charWidth, lineHeightPx, file.docId, file.path, onContentChange]
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      // Real completion dropdown keyboard handling -- checked first so it
      // can intercept Enter/Escape/arrows before any other handler (Tab,
      // undo/redo) sees them, matching how a real open dropdown always
      // owns those keys in every other editor.
      if (completionState) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setCompletionState((prev) =>
            prev && filteredCompletionItems.length > 0
              ? { ...prev, selectedIndex: (prev.selectedIndex + 1) % filteredCompletionItems.length }
              : prev
          );
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setCompletionState((prev) =>
            prev && filteredCompletionItems.length > 0
              ? {
                  ...prev,
                  selectedIndex:
                    (prev.selectedIndex - 1 + filteredCompletionItems.length) %
                    filteredCompletionItems.length,
                }
              : prev
          );
          return;
        }
        if (e.key === "Enter") {
          e.preventDefault();
          const item = filteredCompletionItems[completionState.selectedIndex];
          if (item) acceptCompletion(item);
          else setCompletionState(null);
          return;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          setCompletionState(null);
          return;
        }
        if (e.key === "Backspace") {
          // Shrinks the tracked prefix by one and keeps narrowing, rather
          // than dismissing -- matches how a real editor's own completion
          // dropdown survives a correction. Deliberately not
          // `preventDefault`-ed: the character still needs to actually be
          // deleted from the document via this key's own normal handling
          // below (and `handleChange`'s resulting edit). Backspacing past
          // where the dropdown opened (`typedLength` already at 0) falls
          // to the dismiss branch below instead of tracking a negative
          // prefix.
          setCompletionState((prev) =>
            prev && prev.typedLength > 0
              ? { ...prev, typedLength: prev.typedLength - 1, selectedIndex: 0 }
              : null
          );
        } else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
          // A real printable character -- extend the tracked prefix and
          // keep the dropdown open, live-narrowing `filteredCompletionItems`
          // above, instead of dismissing on the very next keystroke (this
          // component's own previously-named v1 gap, now closed). Not
          // `preventDefault`-ed: the character still needs to actually be
          // typed via this key's own normal handling below.
          setCompletionState((prev) =>
            prev ? { ...prev, typedLength: prev.typedLength + 1, selectedIndex: 0 } : prev
          );
        } else {
          // Any other real key (Left/Right/Home/End/Delete/Tab, etc.)
          // dismisses the dropdown rather than trying to track it -- a
          // real, named, still-remaining v1 scope cut -- and falls
          // through to that key's own normal handling below.
          setCompletionState(null);
        }
      }
      // Real signature-help dismissal (Escape) -- only reached when the
      // completion dropdown didn't already consume this Escape above (its
      // own branch `return`s), so the two never race for the same
      // keypress.
      if (e.key === "Escape" && signatureHelpState) {
        setSignatureHelpState(null);
      }
      // Real find-references dismissal (Escape), same real precedence as
      // signature help's own identical branch above.
      if (e.key === "Escape" && referencesState) {
        setReferencesState(null);
      }
      // Real, manual completion trigger (Ctrl+Space) -- see
      // `triggerCompletion`'s own doc comment for why manual, not
      // automatic-per-keystroke.
      if ((e.ctrlKey || e.metaKey) && e.key === " ") {
        e.preventDefault();
        triggerCompletion();
        return;
      }
      // Real, manual find-references trigger (Shift+F12, the standard
      // cross-editor convention for "Find All References").
      if (e.key === "F12" && e.shiftKey) {
        e.preventDefault();
        triggerReferences();
        return;
      }
      // Real, manual rename-symbol trigger (F2, the standard cross-editor
      // convention). Opens the rename input via `triggerRename`; typing
      // the new name and Enter/Escape are handled by that input's own
      // `onKeyDown` below (a real, distinct focused DOM element, unlike
      // completion/references' own textarea-relative popups), so this
      // branch never fires again while it's open.
      if (e.key === "F2" && !renameState) {
        e.preventDefault();
        triggerRename();
        return;
      }
      if (e.key === "Tab") {
        e.preventDefault();
        const el = textareaRef.current;
        if (!el) return;
        const start = el.selectionStart;
        const end = el.selectionEnd;
        const value = el.value;
        const indent = " ".repeat(prefs.tabSize);
        el.value = `${value.slice(0, start)}${indent}${value.slice(end)}`;
        el.selectionStart = el.selectionEnd = start + indent.length;
        el.dispatchEvent(new Event("input", { bubbles: true }));
      }
      if ((e.ctrlKey || e.metaKey) && e.key === "s") {
        e.preventDefault();
        window.spartan
          .call("save_file", { doc_id: file.docId })
          .then(() => onContentChange(file.path, prevContentRef.current, true))
          .catch((err: Error) => console.error("save failed:", err));
      }
      // Real undo/redo (task #52 audit finding: this crate's own
      // `undo`/`redo` IPC methods existed but were never called --
      // the native textarea's own built-in undo stack is unrelated to
      // and would silently drift from the real backend `Document`'s
      // own branching undo tree, so both are intercepted here and
      // routed through the backend exclusively, never left to fall
      // through to native behavior.
      const isRedo =
        ((e.ctrlKey || e.metaKey) && e.key === "y") ||
        ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "z");
      const isUndo = (e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "z";
      if (isUndo || isRedo) {
        e.preventDefault();
        window.spartan
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
    [
      completionState,
      filteredCompletionItems,
      acceptCompletion,
      triggerCompletion,
      signatureHelpState,
      referencesState,
      triggerReferences,
      renameState,
      triggerRename,
      file.docId,
      file.path,
      onContentChange,
      prefs.tabSize,
    ]
  );

  const lineNumbers = Array.from({ length: lineCount }, (_, i) => i + 1);

  // Real §75.76 editor preferences applied as inline overrides -- the
  // highlight layer and textarea must stay pixel-identical to each other
  // (the whole overlay technique depends on it), so both always receive
  // the exact same style object rather than one being styled via CSS and
  // the other via inline props.
  const textStyle: React.CSSProperties = {
    fontSize: `${prefs.fontSize}px`,
    lineHeight: `${Math.round(prefs.fontSize * 1.54)}px`,
    tabSize: prefs.tabSize,
    whiteSpace: prefs.wordWrap ? "pre-wrap" : "pre",
  };

  return (
    <div className="editor-root">
      <div className="editor-gutter mono" ref={gutterRef} style={textStyle}>
        {lineNumbers.map((n) => {
          // Real LSP positions are 0-indexed; `n` (the displayed line
          // number) is 1-indexed, matching every other real line-number
          // convention in this codebase, and matching real DAP
          // breakpoint/stop-frame line numbers directly (no translation).
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
            // `highlight.js` call against the real open document's own
            // content), never from an external/untrusted source -- the
            // same trust boundary every other real editor content path
            // in this app already assumes for the user's own files.
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
          onClick={handleDefinitionClick}
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
      {signatureHelpState?.target && (
        <div
          className="editor-signature-help-tooltip mono"
          style={{ left: signatureHelpState.x, top: signatureHelpState.y }}
        >
          {renderSignatureLabel(signatureHelpState.target)}
        </div>
      )}
      {renameState && (
        <div
          className="editor-rename-box mono"
          style={{ left: renameState.x, top: renameState.y }}
        >
          {renameState.phase === "editing" ? (
            <input
              ref={renameInputRef}
              className="editor-rename-input"
              value={renameState.value}
              onChange={(e) =>
                setRenameState((prev) => (prev ? { ...prev, value: e.target.value } : prev))
              }
              onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") {
                  e.preventDefault();
                  confirmRename();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  setRenameState(null);
                }
              }}
              onBlur={() => setRenameState((prev) => (prev?.phase === "editing" ? null : prev))}
              placeholder="New name…"
            />
          ) : (
            <div className="editor-rename-status">
              {renameState.phase === "requesting" && "Resolving rename…"}
              {renameState.phase === "applying" && "Applying edits…"}
              {(renameState.phase === "done" || renameState.phase === "error") &&
                renameState.message}
            </div>
          )}
        </div>
      )}
      {referencesState && (
        <div
          className="editor-references-panel mono"
          style={{ left: referencesState.x, top: referencesState.y }}
        >
          <div className="editor-references-header">
            {referencesState.items === null
              ? "Finding references…"
              : `${referencesState.items.length} reference${referencesState.items.length === 1 ? "" : "s"}`}
          </div>
          {referencesState.items === null ? (
            <div className="editor-references-item editor-references-item-empty">Loading…</div>
          ) : referencesState.items.length === 0 ? (
            <div className="editor-references-item editor-references-item-empty">
              No references found
            </div>
          ) : (
            referencesState.items.map((item, i) => (
              <div
                key={`${item.path}:${item.line}:${item.character}:${i}`}
                className="editor-references-item"
                onMouseDown={(e) => {
                  // `onMouseDown`, not `onClick` -- matches the completion
                  // dropdown's own established reasoning (fires before the
                  // textarea's blur/plain-click dismissal below could race
                  // it away).
                  e.preventDefault();
                  setReferencesState(null);
                  goToTarget(item);
                }}
              >
                {item.path === file.path
                  ? `line ${item.line + 1}, col ${item.character + 1}`
                  : `${item.path}:${item.line + 1}`}
              </div>
            ))
          )}
        </div>
      )}
      {completionState && (
        <div
          className="editor-completion-list mono"
          style={{ left: completionState.x, top: completionState.y }}
        >
          {completionState.items.length === 0 ? (
            <div className="editor-completion-item editor-completion-item-empty">Loading…</div>
          ) : filteredCompletionItems.length === 0 ? (
            <div className="editor-completion-item editor-completion-item-empty">No matches</div>
          ) : (
            filteredCompletionItems.map((item, i) => (
              <div
                key={`${item.label}-${i}`}
                className={`editor-completion-item${i === completionState.selectedIndex ? " editor-completion-item-active" : ""}`}
                onMouseDown={(e) => {
                  // `onMouseDown`, not `onClick` -- fires before the
                  // textarea's own blur, so the caret position (and so
                  // `insertAt`) is still exactly where completion was
                  // requested from when `acceptCompletion` reads it.
                  e.preventDefault();
                  acceptCompletion(item);
                }}
              >
                <span className="editor-completion-label">{item.label}</span>
                {item.detail && <span className="editor-completion-detail">{item.detail}</span>}
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
