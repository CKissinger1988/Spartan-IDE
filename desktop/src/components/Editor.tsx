import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { highlightSource, languageForPath } from "../syntax";
import { ensureGrammar, grammarReady } from "../treeSitter";
import {
  adjustSnippetStops,
  expandSnippet,
  findSnippet,
  type SnippetSession,
  type UserSnippet,
} from "../snippets";
import { computeBracketPairMarks } from "../bracketPairs";
import { shiftBreakpointsForEdit } from "../breakpointShift";

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

/** One real source breakpoint, 1-indexed `line` (matching the gutter's own
 * displayed line numbers and the real DAP `frame.line` value directly).
 * `condition` is a real DAP conditional-breakpoint expression (the adapter
 * only stops when it evaluates truthy) and `logMessage` turns it into a
 * real *logpoint* (the adapter logs the interpolated message and does not
 * stop) -- both optional; a bare `{ line }` is an ordinary line breakpoint.
 * Serialized straight into `dap_launch`'s real `breakpoints:
 * [{line, condition?, logMessage?}]` param, which `spartan-backend`'s
 * `parse_breakpoints` reads verbatim. */
export interface BreakpointSpec {
  line: number;
  condition?: string;
  logMessage?: string;
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

/** Real auto-closing bracket/quote pairs (task #193) -- the single most
 * noticeable "doesn't feel like a real editor" gap a plain textarea has.
 * Deliberately v1-scoped, named in `handleKeyDown`'s own comments: no
 * Backspace-deletes-both-of-a-pair behavior, matching this whole
 * session's own "smallest real, correct increment" precedent. */
const OPEN_TO_CLOSE: Record<string, string> = {
  "(": ")",
  "[": "]",
  "{": "}",
  '"': '"',
  "'": "'",
  "`": "`",
};
const CLOSE_CHARS = new Set(Object.values(OPEN_TO_CLOSE));

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

/** A real, normalized call-hierarchy entry -- for the incoming direction
 * each is one `CallHierarchyIncomingCall` whose `from` is the caller; for
 * the outgoing direction each is one `CallHierarchyOutgoingCall` whose `to`
 * is the callee. The name is shown; the `selectionRange.start` is the jump
 * target. */
interface CallerItem {
  name: string;
  path: string;
  line: number;
  character: number;
}

type CallDirection = "incoming" | "outgoing";

/** Normalizes a real `callHierarchy/incomingCalls` or `outgoingCalls`
 * result into a real, jump-ready list. Returns `[]` for a real, honest
 * "none found" rather than throwing. The relevant `CallHierarchyItem` is in
 * `from` (incoming) or `to` (outgoing). */
function extractCallers(result: unknown, direction: CallDirection): CallerItem[] {
  if (!Array.isArray(result)) return [];
  const key = direction === "outgoing" ? "to" : "from";
  return result
    .map((entry) => {
      const item = (entry as Record<string, unknown>)[key] as
        | {
            name?: unknown;
            uri?: unknown;
            selectionRange?: { start?: { line?: unknown; character?: unknown } };
          }
        | undefined;
      const line = item?.selectionRange?.start?.line;
      const character = item?.selectionRange?.start?.character;
      if (
        !item ||
        typeof item.name !== "string" ||
        typeof item.uri !== "string" ||
        typeof line !== "number" ||
        typeof character !== "number"
      ) {
        return null;
      }
      return { name: item.name, path: fileUriToPath(item.uri), line, character };
    })
    .filter((i): i is CallerItem => i !== null);
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

/** A real, raw LSP `CodeAction` as `lsp_code_action_result` delivers it --
 * deliberately typed minimally since the full object (data and all) must
 * be forwarded verbatim to `codeAction/resolve`, never reconstructed. */
export interface CodeActionEnvelope {
  title?: string;
  kind?: string;
}

/** The real display title of a raw code action. */
function codeActionTitle(action: unknown): string {
  return (action as CodeActionEnvelope)?.title ?? "Code action";
}

/** A short, human label for a code action's real LSP `kind` (which arrives
 * as a dotted string like `"quickfix"` or `"source.organizeImports"`),
 * shown next to its title in the quick-fix popup. */
function codeActionKindLabel(action: unknown): string {
  const kind = (action as CodeActionEnvelope)?.kind;
  if (!kind) return "Quick Fix";
  if (kind.startsWith("quickfix")) return "Quick Fix";
  if (kind.startsWith("source")) return "Source";
  if (kind.startsWith("refactor")) return "Refactor";
  return kind;
}

/** A real, normalized, flattened LSP document symbol -- `depth` (0 for a
 * top-level symbol) is the only structural information kept from a real
 * hierarchical result's own `children` nesting, enough for the panel below
 * to indent without needing a recursive render. `kind` is the real LSP
 * `SymbolKind` integer, looked up against `SYMBOL_KIND_LABELS` at render
 * time rather than resolved here, matching this file's own "normalize the
 * data, format at the UI boundary" split. */
interface DocumentSymbolItem {
  name: string;
  kind: number;
  line: number;
  character: number;
  depth: number;
}

/** The real LSP `SymbolKind` enum's own integer values (spec §3.17.4) --
 * only used to render a short, human-readable label next to each real
 * symbol name. */
const SYMBOL_KIND_LABELS: Record<number, string> = {
  1: "File",
  2: "Module",
  3: "Namespace",
  4: "Package",
  5: "Class",
  6: "Method",
  7: "Property",
  8: "Field",
  9: "Constructor",
  10: "Enum",
  11: "Interface",
  12: "Function",
  13: "Variable",
  14: "Constant",
  15: "String",
  16: "Number",
  17: "Boolean",
  18: "Array",
  19: "Object",
  20: "Key",
  21: "Null",
  22: "EnumMember",
  23: "Struct",
  24: "Event",
  25: "Operator",
  26: "TypeParameter",
};

/** Normalizes a real LSP `documentSymbol` result into a real, flat,
 * jump-ready list. Handles both real response shapes the spec allows --
 * a nested `DocumentSymbol[]` (`selectionRange`/`range` directly on each
 * node, real `children`) or a flat `SymbolInformation[]` (its position
 * nested one level deeper, under `location.range`, and never any real
 * `children`) -- **a real, live finding, not assumed from the spec**: see
 * `open_project`'s own Rust-side doc comment for why this crate declares
 * `hierarchicalDocumentSymbolSupport`, which makes every real server this
 * crate has been tested against reply with the nested shape; this
 * normalizer still handles the flat one too, for any real server that
 * doesn't honor that declared capability. Returns `[]` for a real, honest
 * "no symbols in this file" rather than throwing. */
function extractDocumentSymbols(result: unknown): DocumentSymbolItem[] {
  if (!Array.isArray(result)) return [];
  const out: DocumentSymbolItem[] = [];
  const walk = (nodes: unknown[], depth: number) => {
    for (const node of nodes) {
      const n = node as {
        name?: unknown;
        kind?: unknown;
        range?: { start?: { line?: unknown; character?: unknown } };
        selectionRange?: { start?: { line?: unknown; character?: unknown } };
        location?: { range?: { start?: { line?: unknown; character?: unknown } } };
        children?: unknown;
      };
      const name = typeof n.name === "string" ? n.name : null;
      const kind = typeof n.kind === "number" ? n.kind : 0;
      const posSource = n.selectionRange ?? n.range ?? n.location?.range;
      const line = posSource?.start?.line;
      const character = posSource?.start?.character;
      if (name && typeof line === "number" && typeof character === "number") {
        out.push({ name, kind, line, character, depth });
      }
      if (Array.isArray(n.children)) walk(n.children, depth + 1);
    }
  };
  walk(result, 0);
  return out;
}

/** A real, normalized, flattened LSP workspace symbol -- the workspace-wide
 * sibling of `DocumentSymbolItem` above, already flattened into the exact
 * "name + kind + one jump target" shape `goToTarget`'s own callers use.
 * `path` is the real local filesystem path, decoded from the backend's
 * `uri` via `fileUriToPath` at normalization time (the backend already
 * decoded the LSP 3.17 `location`-can-be-bare-`{uri}` wire shape, so the
 * frontend only ever sees a plain `uri` string). `containerName` is the
 * enclosing module/class from the server's own `containerName` when it
 * sent one (rust-analyzer does), rendered as a disambiguating suffix.
 * `kind` is the real LSP `SymbolKind` integer, looked up against
 * `SYMBOL_KIND_LABELS` at render time, matching `DocumentSymbolItem`. */
interface WorkspaceSymbolItem {
  name: string;
  kind: number;
  containerName: string | null;
  path: string;
  line: number;
  character: number;
}

/** Normalizes a real `lsp_workspace_symbol_result` payload's `result` --
 * the backend already decoded the raw LSP array into `{name, kind,
 * container_name, uri, line, character}` entries (see `spartan-lsp`'s own
 * `decode_workspace_symbols`), so this only re-flattens into the frontend's
 * jump shape, translating `uri` to a real path and dropping any entry that
 * somehow lacks a name, path, or position (never half-normalized). A `null`
 * result (a genuinely no-match query) normalizes to `[]` -- "nothing
 * matched" is a real, meaningful answer for a symbol search, not an error,
 * matching `extractDocumentSymbols`' own empty-list contract. */
function extractWorkspaceSymbols(result: unknown): WorkspaceSymbolItem[] {
  if (!Array.isArray(result)) return [];
  const out: WorkspaceSymbolItem[] = [];
  for (const entry of result) {
    const e = entry as {
      name?: unknown;
      kind?: unknown;
      container_name?: unknown;
      uri?: unknown;
      line?: unknown;
      character?: unknown;
    };
    const name = typeof e.name === "string" ? e.name : null;
    const kind = typeof e.kind === "number" ? e.kind : 0;
    const uri = typeof e.uri === "string" ? e.uri : null;
    const line = typeof e.line === "number" ? e.line : null;
    const character = typeof e.character === "number" ? e.character : null;
    if (!name || !uri || line === null || character === null) continue;
    const containerName = typeof e.container_name === "string" ? e.container_name : null;
    out.push({ name, kind, containerName, path: fileUriToPath(uri), line, character });
  }
  return out;
}

/** A real, normalized LSP `DocumentHighlight` -- `kind` is the real spec
 * §3.17.5 value (1 Text, 2 Read, 3 Write), used only to pick a slightly
 * different highlight color at render time, matching `DocumentSymbolItem`'s
 * own "normalize the data, format at the UI boundary" split. Multi-line
 * ranges are real and spec-allowed but, for a real symbol occurrence,
 * essentially never happen in practice -- a real, deliberate v1 scope cut:
 * only `startLine`/`startCharacter`/`endCharacter` are used at render time
 * (assuming `endLine === startLine`), named here rather than silently
 * mis-rendering a genuinely multi-line highlight. */
interface DocumentHighlightItem {
  startLine: number;
  startCharacter: number;
  endLine: number;
  endCharacter: number;
  kind: number;
}

/** Normalizes a real LSP `documentHighlight` result (a real
 * `DocumentHighlight[]`, or `null`). Returns `[]` for a real, honest "no
 * highlightable occurrences here" -- not every cursor position is on a
 * real symbol. */
function extractDocumentHighlights(result: unknown): DocumentHighlightItem[] {
  if (!Array.isArray(result)) return [];
  return result
    .map((entry) => {
      const e = entry as {
        range?: {
          start?: { line?: unknown; character?: unknown };
          end?: { line?: unknown; character?: unknown };
        };
        kind?: unknown;
      };
      const startLine = e.range?.start?.line;
      const startCharacter = e.range?.start?.character;
      const endLine = e.range?.end?.line;
      const endCharacter = e.range?.end?.character;
      if (
        typeof startLine !== "number" ||
        typeof startCharacter !== "number" ||
        typeof endLine !== "number" ||
        typeof endCharacter !== "number"
      ) {
        return null;
      }
      const kind = typeof e.kind === "number" ? e.kind : 1;
      return { startLine, startCharacter, endLine, endCharacter, kind };
    })
    .filter((h): h is DocumentHighlightItem => h !== null);
}

/** One decoded LSP semantic token (the shape `lsp_semantic_tokens_result`
 * carries -- already legend-resolved by the backend, so `token_type` is a
 * real name like `"struct"`/`"function"`, not a server-legend index). */
interface SemanticTokenItem {
  line: number;
  character: number;
  length: number;
  token_type: string;
  modifiers: string[];
}

/** Normalizes a real `lsp_semantic_tokens_result` `result` (a decoded
 * `SemanticToken[]`, or `null` for a genuinely clean file). Returns `[]`
 * for a real, honest "no tokens here". */
function extractSemanticTokens(result: unknown): SemanticTokenItem[] {
  if (!Array.isArray(result)) return [];
  return result.flatMap((entry) => {
    const e = entry as Partial<SemanticTokenItem>;
    if (
      typeof e.line !== "number" ||
      typeof e.character !== "number" ||
      typeof e.length !== "number" ||
      typeof e.token_type !== "string"
    ) {
      return [];
    }
    return [
      {
        line: e.line,
        character: e.character,
        length: Math.max(1, e.length),
        token_type: e.token_type,
        modifiers: Array.isArray(e.modifiers)
          ? e.modifiers.filter((m): m is string => typeof m === "string")
          : [],
      },
    ];
  });
}

/** One decoded LSP inlay hint (the shape `lsp_inlay_hints_result` carries --
 * already decoded by the backend, so `label` is the plain rendered text and
 * `kind` is the real LSP `InlayHintKind` (1 Type, 2 Parameter, 3 Everything
 * else) when the server sent one). */
interface InlayHintItem {
  line: number;
  character: number;
  label: string;
  kind: number | null;
  padding_left: boolean;
  padding_right: boolean;
}

/** Normalizes a real `lsp_inlay_hints_result` `result` (a decoded
 * `InlayHint[]`, or `null` for a genuinely hint-free file). Returns `[]`
 * for a real, honest "no hints here". */
function extractInlayHints(result: unknown): InlayHintItem[] {
  if (!Array.isArray(result)) return [];
  return result.flatMap((entry) => {
    const e = entry as Partial<InlayHintItem>;
    if (
      typeof e.line !== "number" ||
      typeof e.character !== "number" ||
      typeof e.label !== "string"
    ) {
      return [];
    }
    return [
      {
        line: e.line,
        character: e.character,
        label: e.label,
        kind: typeof e.kind === "number" ? e.kind : null,
        padding_left: e.padding_left === true,
        padding_right: e.padding_right === true,
      },
    ];
  });
}

/** Inlay-hint render classes, keyed by the real LSP `InlayHintKind` (1 Type,
 * 2 Parameter, 3 Everything else) -- VS Code's own per-kind coloring,
 * ported: type hints get the type hue, parameter hints the parameter hue,
 * everything else the neutral hint hue. `null` means "no real `kind` sent,
 * use the neutral class". */
function inlayHintKindClass(kind: number | null): string {
  if (kind === 1) return "editor-inlay-hint-type";
  if (kind === 2) return "editor-inlay-hint-parameter";
  return "editor-inlay-hint-other";
}

/** Inlay-hint label with the server's real `paddingLeft`/`paddingRight`
 * flags applied as actual spaces -- the exact convention VS Code's own
 * inlay-hint renderer uses (a `paddingRight` parameter hint like `a:` is
 * spaced "a: 1" rather than "a:1"). */
function renderInlayHintLabel(hint: InlayHintItem): string {
  return `${hint.padding_left ? " " : ""}${hint.label}${hint.padding_right ? " " : ""}`;
}

/** Semantic-token marks deliberately render only for the token types that
 * add real information *on top of* the existing syntax-color layer --
 * structs/classes/types/functions/macros/namespaces/variables/etc., which
 * the local highlighter can't know. Keywords, strings, numbers, comments
 * and operators are already colored by `highlightSource`, so painting marks
 * on them would be pure visual noise; they're filtered out here. */
const SEMANTIC_TOKEN_MARK_CLASSES: Record<string, string> = {
  struct: "editor-semantic-token-struct",
  class: "editor-semantic-token-struct",
  enum: "editor-semantic-token-struct",
  union: "editor-semantic-token-struct",
  typeAlias: "editor-semantic-token-struct",
  builtinType: "editor-semantic-token-struct",
  type: "editor-semantic-token-type",
  interface: "editor-semantic-token-type",
  function: "editor-semantic-token-function",
  method: "editor-semantic-token-function",
  macro: "editor-semantic-token-function",
  namespace: "editor-semantic-token-namespace",
  module: "editor-semantic-token-namespace",
  variable: "editor-semantic-token-variable",
  parameter: "editor-semantic-token-variable",
  property: "editor-semantic-token-variable",
  enumMember: "editor-semantic-token-variable",
  constParameter: "editor-semantic-token-variable",
  typeParameter: "editor-semantic-token-type-param",
};

function semanticTokenMarkClass(tokenType: string): string | null {
  return SEMANTIC_TOKEN_MARK_CLASSES[tokenType] ?? null;
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

/** Real, pure bracket-pair matcher -- a plain forward/backward depth-
 * tracking scan of the whole document, no LSP round trip needed (unlike
 * `documentHighlights`/hover/etc., every one of which needs a real
 * language server). A real, deliberate, named v1 scope cut: no string/
 * comment awareness, so a bracket character inside a string literal or
 * comment is matched like any other -- exactly the same honest
 * simplification most editors' own *first* bracket-matching increment
 * ships with, before layering in tokenization.
 *
 * Checks all four real cursor-adjacency cases, in the same priority order
 * every mainstream editor's own bracket matching uses: the character at
 * `offset` first (cursor sitting just *before* an opener or closer), then
 * the character just before `offset` (cursor sitting just *after* one).
 * **A real bug was caught only by live testing, not by inspection**: an
 * earlier version only checked "before cursor is a closer," never "before
 * cursor is an opener" -- so the single most common real trigger for this
 * whole feature, the cursor landing right after a just-typed or
 * just-auto-paired `(`, silently showed no match at all. A live Playwright
 * script moving the real caret with real arrow-key presses (matching a
 * real user's own keyboard, not a synthetic DOM event) caught it directly:
 * landing right after `f(` in `def f(x, y):` showed zero highlight marks,
 * while landing right before the matching `)` correctly showed two. Fixed
 * by covering all four cases explicitly instead of assuming the two
 * checked so far were symmetric. Returns both real matched offsets, or
 * `null` if the cursor isn't adjacent to any bracket, or its match
 * genuinely isn't found (a real, unbalanced/incomplete document -- not an
 * error, just nothing to show). */
function findMatchingBracket(content: string, offset: number): [number, number] | null {
  const atCursor = content[offset];
  if (atCursor && BRACKET_PAIRS[atCursor]) return matchBracketForward(content, offset);
  if (atCursor && CLOSE_TO_OPEN[atCursor]) return matchBracketBackward(content, offset);
  const beforeCursor = content[offset - 1];
  if (beforeCursor && BRACKET_PAIRS[beforeCursor]) return matchBracketForward(content, offset - 1);
  if (beforeCursor && CLOSE_TO_OPEN[beforeCursor]) return matchBracketBackward(content, offset - 1);
  return null;
}

/** Real, per-language line-comment tokens, keyed by the same hljs
 * language ids `languageForPath` already returns -- one source of truth
 * for "what language is this file," not a second, separately-maintained
 * extension map. Deliberately omits `xml`/`css`/`json`/`markdown`: none
 * of them has a real, unambiguous single-line comment token (JSON has
 * none at all; the others are block-comment-only), so `toggleLineComment`
 * below refuses honestly rather than guessing wrong for those. */
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

/** Real, pure line-comment toggle (Ctrl+/) -- no LSP query needed, just
 * the per-language token above. Comments every line spanned by
 * `[selStart, selEnd]` (or just the caret's own line with no selection)
 * if any non-blank spanned line isn't already commented; uncomments every
 * spanned line otherwise -- the standard "comment wins over uncomment
 * when mixed" convention, so a selection straddling a mix of commented/
 * uncommented lines always converges to fully commented in one press
 * rather than toggling each line independently. Recognizes a line as
 * already commented whether or not it carries the trailing space this
 * function always inserts (`// foo` and `//foo` both toggle off
 * correctly). Blank lines within a touched range are left untouched in
 * both directions, matching every mainstream editor's own convention.
 * Returns `null` for a language with no known comment token (an honest
 * no-op, not a guess) or a caret sitting past the end of the last real
 * line. */
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
  // A full-line drag selection's own end offset sits at the *start* of
  // the line just past the selection -- don't count that line as
  // "touched," matching every mainstream editor's own multi-line-
  // selection convention for this exact command.
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

/** Real multi-line indent/outdent (Tab/Shift+Tab on an active selection,
 * or Shift+Tab alone on a collapsed caret) -- `direction` is `1` to
 * indent (prepend `indent` to every real touched line) or `-1` to outdent
 * (strip up to `indent.length` characters of real leading whitespace from
 * every touched line, matching most editors' own "up to one indent
 * worth, whatever's actually there" outdent behavior rather than
 * requiring an exact match). Blank lines are indented like any other line
 * (an indented blank line is still blank, so there's no reason to skip
 * it, a real, deliberate difference from `toggleLineComment`'s own "leave
 * blank lines alone" rule) but never outdented past column 0. Selection
 * restoration reuses the same real column-shift approach
 * `toggleLineComment` already established: each boundary line's own
 * per-line delta is applied to that boundary's column, then clamped to
 * never go negative or past the new line's own length. */
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

/** Real line/block duplication (Ctrl+Shift+D) -- the standard cross-
 * editor convention: with no selection, duplicates the caret's own line
 * directly below itself and moves the caret onto the new copy at the
 * same column; with an active selection, duplicates every line the
 * selection touches as one block, inserted immediately after the last
 * touched line, and moves the selection down onto the new copy at the
 * same relative start/end columns. Reuses the same touched-line-range
 * computation `toggleLineComment`/`reindentLines` already established
 * (`lineStarts`/`lineIndexAt`/full-line-drag-selection-boundary logic),
 * but needs no per-line delta tracking the way those two do -- every
 * touched line shifts down by the exact same fixed amount (the touched
 * block's own line count), so the new selection is computed directly
 * from that shift rather than accumulated line-by-line. */
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

  let newFirstLineStart = 0;
  for (let i = 0; i < newFirstLine; i++) newFirstLineStart += newLines[i].length + 1;
  let newLastLineStart = newFirstLineStart;
  for (let i = newFirstLine; i < newLastLine; i++) newLastLineStart += newLines[i].length + 1;

  return {
    content: newLines.join("\n"),
    selectionStart: newFirstLineStart + startCol,
    selectionEnd: newLastLineStart + endCol,
  };
}

/** Real move-line-up/down (Alt+Up/Alt+Down) -- swaps the touched line
 * block (the caret's own line with no selection, or every line an active
 * selection touches) with the single adjacent line immediately above
 * (`direction: -1`) or below (`direction: 1`), keeping the selection
 * following the moved block at the same relative start/end columns.
 * Returns `null` at a real document boundary (nothing above line 0 to
 * swap with when moving up; nothing below the last line when moving
 * down) rather than wrapping around or silently doing nothing that still
 * calls `applyProgrammaticEdit` -- the caller only applies a real edit
 * when this returns non-null. Reuses the same touched-line-range
 * computation `toggleLineComment`/`reindentLines`/`duplicateLines` already
 * established, but the actual swap is a real, plain `Array.splice` pair
 * (remove the one adjacent line, reinsert it on the other side of the
 * touched block) rather than a full sort/rebuild -- verified by hand for
 * both a single line and a multi-line block in both directions before
 * being wired up, since an off-by-one here would silently scramble line
 * order rather than throw. */
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

  let newFirstLineStart = 0;
  for (let i = 0; i < newFirstLine; i++) newFirstLineStart += newLines[i].length + 1;
  let newLastLineStart = newFirstLineStart;
  for (let i = newFirstLine; i < newLastLine; i++) newLastLineStart += newLines[i].length + 1;

  return {
    content: newLines.join("\n"),
    selectionStart: newFirstLineStart + startCol,
    selectionEnd: newLastLineStart + endCol,
  };
}

/** Real "Delete Line" (Ctrl+Shift+K) -- removes the caret's own line (or
 * every line an active selection touches, as one block) entirely,
 * including its own trailing newline, landing the caret at column 0 of
 * whatever line now occupies that same index -- clamped to the real new
 * last line if the deleted block ran through the document's own end.
 * Reuses the same touched-line-range computation
 * `toggleLineComment`/`reindentLines`/`duplicateLines`/`moveLines`
 * already established. Unlike `moveLines`, this never refuses -- deleting
 * every remaining line correctly collapses to a real empty document
 * (`newLines` becomes `[]`, `join("\n")` correctly produces `""`) rather
 * than erroring, and the loop that computes the landing offset never
 * dereferences an out-of-range index even in that empty case, since its
 * own bound is clamped to `newLines.length - 1` first. */
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

/** Real, pure "join lines" (Ctrl+J): merges the touched lines (the caret's own
 * line with the next one if there's no selection, or every line an active
 * selection touches) into a single line -- leading whitespace of each joined
 * line is trimmed and a single space inserted at the seam unless the running
 * text already ends in whitespace or the joined segment is empty (matching
 * VS Code's own Join Lines behavior). The caret lands at the first seam.
 * Returns `null` when there is nothing to join (the caret is on the last line
 * with no selection), so the caller can treat it as a no-op. */
function joinLines(
  content: string,
  selStart: number,
  selEnd: number
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
  const joinTo = lastLine > firstLine ? lastLine : firstLine + 1;
  if (joinTo > lines.length - 1) return null;

  let joined = lines[firstLine];
  const seamOffset = joined.length;
  for (let i = firstLine + 1; i <= joinTo; i++) {
    const seg = lines[i].replace(/^\s+/, "");
    if (joined.length > 0 && !/\s$/.test(joined) && seg.length > 0) {
      joined += " " + seg;
    } else {
      joined += seg;
    }
  }

  const newLines = [...lines.slice(0, firstLine), joined, ...lines.slice(joinTo + 1)];
  const newContent = newLines.join("\n");
  const caret = lineStarts[firstLine] + seamOffset;
  return { content: newContent, selectionStart: caret, selectionEnd: caret };
}

/** Real, pure, language-agnostic "trim trailing whitespace" -- strips
 * trailing spaces/tabs from every line, leaving line count and every
 * other character untouched. A real, deliberate, named v1 scope cut:
 * this does *not* also insert a missing final newline (a distinct real
 * editor convention, `insert_final_newline`, not attempted here) -- just
 * the one, narrow, unambiguous transform. Used as `triggerFormatDocument`'s
 * own real fallback when the backend's `format_document` synchronously
 * refuses because no real formatter is configured or wired for this
 * file's language (Java has zero configured formatter at all; Kotlin/C#
 * have one configured but no real stdin/stdout filter-mode invocation,
 * §183) -- a real, honest, universally-applicable improvement over doing
 * nothing, never applied when a real *configured* formatter fails to run
 * or rejects the input (that stays a real, reported failure, not silently
 * papered over). */
function trimTrailingWhitespace(content: string): string {
  return content
    .split("\n")
    .map((line) => line.replace(/[ \t]+$/, ""))
    .join("\n");
}

interface FindMatch {
  start: number;
  end: number;
}

/** Real, pure, plain-substring "find" -- matches `search_project`'s own
 * already-established v1 scope (task #190): plain substring, not regex.
 * An empty query correctly returns zero matches rather than matching
 * every position. Advances by the real match length (not by 1) so
 * matches never overlap, matching every mainstream editor's own "find
 * next" semantics. */
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

/** Real "Replace All" -- rebuilds the document in a single left-to-right
 * pass over the given (already-sorted, non-overlapping -- guaranteed by
 * `findAllMatches`'s own construction) matches, so every replacement
 * applies via exactly one real edit/undo checkpoint rather than N
 * separate ones. */
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
  /** Real "Format Document on Save" (task #187) -- when true, Ctrl+S
   * runs the same real `format_document` a manual Ctrl+Shift+F does
   * before writing to disk. */
  formatOnSave: boolean;
  /** Real user-defined snippets (the follow-up the curated-snippets pass
   * named): `Settings.user_snippets` as loaded by `App.tsx`'s one real
   * settings fetch. Consulted by the Tab-expansion `findSnippet` call on
   * top of the curated `SNIPPETS` table, with a user snippet for the same
   * `(lang_id, prefix)` pair winning over the curated one. Empty (the
   * default) means "no user snippets defined" and is a totally normal
   * state. */
  userSnippets: UserSnippet[];
}

export const DEFAULT_EDITOR_PREFS: EditorPrefs = {
  fontSize: 13,
  tabSize: 2,
  wordWrap: false,
  formatOnSave: false,
  userSnippets: [],
};

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
  /** Real, 1-indexed breakpoint specs for this file (matching the gutter's
   * own displayed line numbers and the real DAP `breakpoints` param
   * `App.tsx` sends to `dap_launch` directly, no off-by-one translation
   * needed at either end). Each carries an optional `condition`/`logMessage`
   * for conditional breakpoints/logpoints. */
  breakpoints?: BreakpointSpec[];
  /** Real click-to-toggle -- `App.tsx` owns the actual breakpoint set
   * (it must survive an editor unmount/tab switch), this component only
   * reports which 1-indexed line was clicked. Toggling always creates a
   * plain (unconditional) breakpoint or removes whatever is there. */
  onToggleBreakpoint?: (line: number) => void;
  /** Real edit of a breakpoint's condition/log message (right-click a
   * gutter line). Passing empty strings for both clears them back to a
   * plain breakpoint; `App.tsx` owns applying it to the real set. A line
   * with no existing breakpoint gains one when a condition/logpoint is
   * set on it. */
  onEditBreakpoint?: (line: number, condition: string, logMessage: string) => void;
  /** Real rope-anchored breakpoint shifting (closes the §75.8-named
   * "line-number only" gap) -- called with the full, already-shifted
   * breakpoint array whenever a real edit moves or invalidates one or
   * more breakpoints' lines (see `breakpointShift.ts`'s own doc
   * comment for the exact rule). Only called when the result actually
   * differs from the current `breakpoints` prop, matching
   * `onToggleBreakpoint`'s own "`App.tsx` owns the real set" division
   * of responsibility. */
  onBreakpointsShift?: (next: BreakpointSpec[]) => void;
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
/** One line's real git blame, as returned by the backend `git_blame`
 * (spartan-git's `blame_file`). */
interface BlameLineInfo {
  oid: string;
  summary: string;
  author: string;
  time: number;
}

/** Coarse relative age for a unix-seconds commit time -- deliberately
 * coarse (a blame gutter, not a timestamp report), matching `GitPanel`'s
 * own `formatAge` convention. */
function formatBlameAge(unixSeconds: number): string {
  if (!unixSeconds) return "";
  const secs = Math.max(0, Math.floor(Date.now() / 1000) - unixSeconds);
  const mins = Math.floor(secs / 60);
  if (mins < 1) return "now";
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months}mo`;
  return `${Math.floor(months / 12)}y`;
}

export default function Editor({
  file,
  onContentChange,
  prefs = DEFAULT_EDITOR_PREFS,
  diagnostics = [],
  breakpoints = [],
  onToggleBreakpoint,
  onEditBreakpoint,
  onBreakpointsShift,
  stoppedLine = null,
  onJumpToDefinition,
  pendingJump = null,
  onJumpApplied,
  onApplyRename,
}: EditorProps): React.ReactElement {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const gutterRef = useRef<HTMLDivElement>(null);
  const highlightRef = useRef<HTMLPreElement>(null);
  const symbolHighlightRef = useRef<HTMLDivElement>(null);
  const [lineCount, setLineCount] = useState(1);
  const prevContentRef = useRef(file.content);

  /** Real, shared breakpoint-shift step (a real, confirmed follow-up gap
   * from task #291's own rope-anchored breakpoints pass): every real
   * content replacement -- not just `applyProgrammaticEdit`'s own typed/
   * programmatic edits, but also completion-accept, a real format-apply
   * result, the no-formatter trim fallback, and undo/redo, none of which
   * route through that function -- needs this exact same "shift or drop
   * each breakpoint against what actually changed" step run against the
   * real pre-replace text before it's overwritten, or a breakpoint set
   * before one of those other paths ran would silently point at the
   * wrong line afterward. One implementation, called from every real
   * content-replacing call site in this component instead of five
   * separately-duplicated copies. */
  const shiftBreakpointsBeforeReplace = useCallback(
    (oldContent: string, newContent: string) => {
      if (breakpoints.length > 0 && onBreakpointsShift) {
        const shifted = shiftBreakpointsForEdit(breakpoints, oldContent, newContent);
        if (shifted !== breakpoints) {
          onBreakpointsShift(shifted);
        }
      }
    },
    [breakpoints, onBreakpointsShift]
  );

  /** Real inline git blame (P1 backlog) -- per-line commit attribution
   * from the real backend `git_blame` (spartan-git). Alt+B toggles it.
   * `blameOn` is the real on/off; `blameLines` is the fetched data
   * (empty is a valid "no blame" state: not a git repo, or an
   * untracked/uncommitted file). Blame is aligned by line index to the
   * *committed* file, so it's exact for an unedited buffer and drifts
   * within edited regions until the next commit -- the same limitation
   * every inline-blame tool has, named in `blame_file`'s own doc comment. */
  const [blameOn, setBlameOn] = useState(false);
  const [blameLines, setBlameLines] = useState<BlameLineInfo[]>([]);
  const blameGutterRef = useRef<HTMLDivElement>(null);
  // Real snippet expansion (P1 backlog): an active session holds the
  // absolute tab-stop offsets for a just-expanded snippet, so plain Tab
  // navigates between placeholders. `null` = no snippet in progress.
  const snippetSessionRef = useRef<SnippetSession | null>(null);

  const fetchBlame = useCallback(() => {
    // project_root: the file's own parent directory -- `git_blame`'s own
    // discover() walks upward to find the repo, and it accepts the file's
    // absolute path directly (resolving it to a repo-relative path).
    const parent = file.path.replace(/[/\\][^/\\]*$/, "") || file.path;
    window.spartan
      .call("git_blame", { project_root: parent, path: file.path })
      .then((r) => {
        const lines = ((r as { lines?: BlameLineInfo[] }).lines ?? []) as BlameLineInfo[];
        setBlameLines(lines);
      })
      .catch(() => setBlameLines([])); // not a git repo / untracked -> no blame
  }, [file.path]);

  // Fetch blame when it's toggled on and whenever the active file changes
  // while it stays on (fetchBlame's identity tracks file.path).
  useEffect(() => {
    if (blameOn) fetchBlame();
    else setBlameLines([]);
  }, [blameOn, fetchBlame]);

  /** Real, live font-size zoom (Ctrl+=/Ctrl+-/Ctrl+0) -- a session-only
   * delta layered on top of `prefs.fontSize` (the real, persisted
   * Settings value), matching the standard "editor zoom is separate from
   * the settings.json font size" convention most editors and browsers
   * already use. `prefs.fontSize` changing externally (a real Settings
   * screen edit) still takes effect immediately -- `effectiveFontSize` is
   * always recomputed from the current `prefs.fontSize` plus this delta,
   * never a frozen snapshot. */
  const [fontSizeDelta, setFontSizeDelta] = useState(0);
  const effectiveFontSize = Math.min(32, Math.max(8, prefs.fontSize + fontSizeDelta));
  const zoomFontSize = useCallback(
    (step: number) => {
      setFontSizeDelta((prev) => Math.min(32, Math.max(8, prefs.fontSize + prev + step)) - prefs.fontSize);
    },
    [prefs.fontSize]
  );

  useEffect(() => {
    prevContentRef.current = file.content;
    setLineCount(file.content.split("\n").length);
  }, [file.content]);

  // Real tree-sitter grammars load asynchronously (a real WASM fetch), so
  // the first paint of a file uses `highlight.js` and this counter forces
  // exactly one re-highlight once the grammar is genuinely ready. Bumping a
  // counter rather than storing the grammar keeps `highlightSource`
  // synchronous -- the render path is unchanged.
  const [grammarGeneration, setGrammarGeneration] = useState(0);
  useEffect(() => {
    const language = languageForPath(file.path);
    if (!language || grammarReady(language)) return;
    let cancelled = false;
    void ensureGrammar(language).then((entry) => {
      if (!cancelled && entry) setGrammarGeneration((n) => n + 1);
    });
    return () => {
      cancelled = true;
    };
  }, [file.path]);

  const highlightedHtml = useMemo(
    () => highlightSource(file.content, file.path),
    // `grammarGeneration` is deliberately a dependency with no direct use in
    // the body: it is the signal that a real grammar just became available,
    // so the memo must recompute even though content/path are unchanged.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [file.content, file.path, grammarGeneration]
  );

  const breakpointMap = useMemo(() => {
    const m = new Map<number, BreakpointSpec>();
    for (const bp of breakpoints) m.set(bp.line, bp);
    return m;
  }, [breakpoints]);

  // Real inline breakpoint-condition editor state (right-click a gutter
  // line). `top` is the pixel y of the clicked gutter row so the popup
  // renders next to it; `condition`/`logMessage` seed from the existing
  // spec (if any) so an edit shows the current values rather than blank.
  const [breakpointEdit, setBreakpointEdit] = useState<{
    line: number;
    condition: string;
    logMessage: string;
    top: number;
  } | null>(null);

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
    if (!ctx) return effectiveFontSize * 0.6;
    ctx.font = `${effectiveFontSize}px "JetBrains Mono", monospace`;
    return ctx.measureText("M").width || effectiveFontSize * 0.6;
  }, [effectiveFontSize]);

  const lineHeightPx = Math.round(effectiveFontSize * 1.54);

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
      shiftBreakpointsBeforeReplace(prevContentRef.current, newContent);
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
    [completionState, file.docId, file.path, onContentChange, shiftBreakpointsBeforeReplace]
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
  // Real go-to-type-definition (Ctrl+Shift+Click) -- "jump to a value's
  // type," not the value itself (e.g. from a variable to its class rather
  // than its assignment). A real, separate pending ref from
  // `pendingDefinitionRef` above so the two requests can never be confused
  // with each other if both happen to be in flight at once. Reuses the
  // identical `Location | Location[] | LocationLink[] | null` response
  // shape `extractDefinitionTarget` already normalizes -- no separate
  // normalizer needed, `textDocument/typeDefinition` returns the same real
  // shape `textDocument/definition` does.
  const pendingTypeDefinitionRef = useRef<{ line: number; character: number } | null>(null);

  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event === "lsp_definition_result") {
        const d = data as { doc_id: number; line: number; character: number; result: unknown };
        if (d.doc_id !== file.docId) return;
        const pending = pendingDefinitionRef.current;
        if (!pending || pending.line !== d.line || pending.character !== d.character) return;
        pendingDefinitionRef.current = null;
        const target = extractDefinitionTarget(d.result);
        // A real, honest "no definition resolvable here" -- silent,
        // matching how every real editor's own Ctrl+Click behaves at an
        // unbound position rather than surfacing an error for a
        // completely normal case.
        if (!target) return;
        goToTarget(target);
      } else if (event === "lsp_type_definition_result") {
        const d = data as { doc_id: number; line: number; character: number; result: unknown };
        if (d.doc_id !== file.docId) return;
        const pending = pendingTypeDefinitionRef.current;
        if (!pending || pending.line !== d.line || pending.character !== d.character) return;
        pendingTypeDefinitionRef.current = null;
        const target = extractDefinitionTarget(d.result);
        if (!target) return;
        goToTarget(target);
      }
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

  /** Real "Go to Symbol in Workspace" (Ctrl+T, VS Code's own standard
   * cross-editor convention for the same action) -- the workspace-wide
   * sibling of the document-symbol outline above, and the eighth real
   * LSP-backed editor feature. Unlike every other overlay in this file
   * it's a genuinely searchable palette: a real focused `<input>` whose
   * free-text query drives the real `workspace/symbol` request (empty
   * query = "list everything", the convention every real editor's symbol
   * search uses), with results arriving asynchronously and rendered as a
   * keyboard-navigable jump list. Debounced (250ms) like hover, with the
   * same stale-reply guard the other async overlays use: the in-flight
   * query is captured in a ref and a result is only applied when it still
   * matches the query currently being searched for. `items: null` means
   * in flight. The backend emits `lsp_workspace_symbol_result` with
   * `{doc_id, query, result}` where `result` is the already-decoded
   * `{name, kind, container_name, uri, line, character}` array. */
  const [wsSymbolsState, setWsSymbolsState] = useState<{
    x: number;
    y: number;
    query: string;
    items: WorkspaceSymbolItem[] | null;
  } | null>(null);
  const wsSymbolsInputRef = useRef<HTMLInputElement>(null);
  const wsSymbolsQueryRef = useRef<string>("");
  const wsSymbolsDebounceRef = useRef<number | null>(null);
  const [wsSymbolsSelected, setWsSymbolsSelected] = useState(0);

  useEffect(() => {
    if (wsSymbolsState) wsSymbolsInputRef.current?.focus();
  }, [wsSymbolsState]);

  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event !== "lsp_workspace_symbol_result") return;
      const d = data as { doc_id: number; query: string; result: unknown };
      if (d.doc_id !== file.docId || d.query !== wsSymbolsQueryRef.current) return;
      setWsSymbolsState((prev) =>
        prev ? { ...prev, items: extractWorkspaceSymbols(d.result) } : prev
      );
    });
    return unsubscribe;
  }, [file.docId]);

  const fireWorkspaceSymbols = useCallback(
    (query: string) => {
      wsSymbolsQueryRef.current = query;
      setWsSymbolsSelected(0);
      setWsSymbolsState((prev) => (prev ? { ...prev, query, items: null } : prev));
      window.spartan
        .call("lsp_workspace_symbol", { doc_id: file.docId, query })
        .catch((err: Error) => console.error("lsp_workspace_symbol failed:", err));
    },
    [file.docId]
  );

  const triggerWorkspaceSymbols = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    const { line, character } = offsetToLineChar(el.value, el.selectionStart);
    const x = el.getBoundingClientRect().left + character * charWidth - el.scrollLeft;
    const y = el.getBoundingClientRect().top + line * lineHeightPx - el.scrollTop + lineHeightPx;
    setWsSymbolsState({ x, y, query: "", items: null });
    fireWorkspaceSymbols("");
  }, [charWidth, lineHeightPx, file.docId, fireWorkspaceSymbols]);

  const handleWsSymbolsKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter") {
        e.preventDefault();
        const items = wsSymbolsState?.items;
        if (items && items.length > 0) {
          const item = items[Math.min(wsSymbolsSelected, items.length - 1)];
          setWsSymbolsState(null);
          goToTarget({ path: item.path, line: item.line, character: item.character });
        }
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setWsSymbolsState(null);
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        const items = wsSymbolsState?.items;
        if (items && items.length > 0) {
          setWsSymbolsSelected((s) => (s + 1) % items.length);
        }
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        const items = wsSymbolsState?.items;
        if (items && items.length > 0) {
          setWsSymbolsSelected((s) => (s - 1 + items.length) % items.length);
        }
      }
    },
    [goToTarget, wsSymbolsSelected, wsSymbolsState]
  );

  const handleWsSymbolsChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const query = e.target.value;
      setWsSymbolsState((prev) => (prev ? { ...prev, query } : prev));
      if (wsSymbolsDebounceRef.current !== null) {
        window.clearTimeout(wsSymbolsDebounceRef.current);
      }
      wsSymbolsDebounceRef.current = window.setTimeout(() => {
        wsSymbolsDebounceRef.current = null;
        fireWorkspaceSymbols(query);
      }, 250);
    },
    [fireWorkspaceSymbols]
  );

  const closeWorkspaceSymbols = useCallback(() => {
    if (wsSymbolsDebounceRef.current !== null) {
      window.clearTimeout(wsSymbolsDebounceRef.current);
      wsSymbolsDebounceRef.current = null;
    }
    setWsSymbolsState(null);
  }, []);

  const handleDefinitionClick = useCallback(
    (e: React.MouseEvent<HTMLTextAreaElement>) => {
      if (!(e.ctrlKey || e.metaKey)) {
        // A real plain click dismisses an open references panel -- the
        // same "clicking elsewhere closes it" behavior every real
        // editor's own find-references popup has. Same for a real open
        // document-symbol outline panel.
        setReferencesState(null);
        setSymbolsState(null);
        closeWorkspaceSymbols();
        setCallHierarchyState(null);
        setQuickFixState(null);
        return;
      }
      const el = textareaRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const x = e.clientX - rect.left + el.scrollLeft;
      const y = e.clientY - rect.top + el.scrollTop;
      const line = Math.max(0, Math.floor(y / lineHeightPx));
      const character = Math.max(0, Math.round(x / charWidth));
      if (e.shiftKey) {
        // Real Ctrl+Shift+Click -- "Go to Type Definition," the real
        // sibling of plain Ctrl+Click below.
        pendingTypeDefinitionRef.current = { line, character };
        window.spartan
          .call("lsp_type_definition", { doc_id: file.docId, line, character })
          .catch((err: Error) => console.error("lsp_type_definition failed:", err));
        return;
      }
      pendingDefinitionRef.current = { line, character };
      window.spartan
        .call("lsp_definition", { doc_id: file.docId, line, character })
        .catch((err: Error) => console.error("lsp_definition failed:", err));
    },
    [charWidth, lineHeightPx, file.docId, closeWorkspaceSymbols]
  );

  const [referencesState, setReferencesState] = useState<{
    x: number;
    y: number;
    items: ReferenceItem[] | null;
  } | null>(null);
  const pendingReferencesRef = useRef<{ line: number; character: number } | null>(null);
  const [callHierarchyState, setCallHierarchyState] = useState<{
    x: number;
    y: number;
    direction: CallDirection;
    items: CallerItem[] | null;
  } | null>(null);
  const pendingCallHierarchyRef = useRef<{
    line: number;
    character: number;
    direction: CallDirection;
  } | null>(null);

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

  // Real call hierarchy (Shift+Alt+H incoming / Shift+Alt+O outgoing) --
  // listens for this exact file's own `lsp_call_hierarchy_result` events, the
  // same real request/reply pattern find-references already established.
  // `items: null` while in flight.
  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event !== "lsp_call_hierarchy_result") return;
      const d = data as {
        doc_id: number;
        line: number;
        character: number;
        direction?: string;
        result: unknown;
      };
      if (d.doc_id !== file.docId) return;
      const pending = pendingCallHierarchyRef.current;
      const dir: CallDirection = d.direction === "outgoing" ? "outgoing" : "incoming";
      if (
        !pending ||
        pending.line !== d.line ||
        pending.character !== d.character ||
        pending.direction !== dir
      ) {
        return;
      }
      pendingCallHierarchyRef.current = null;
      setCallHierarchyState((prev) =>
        prev ? { ...prev, items: extractCallers(d.result, dir) } : prev
      );
    });
    return unsubscribe;
  }, [file.docId]);

  /** Real, manual call-hierarchy trigger (Shift+Alt+H incoming / Shift+Alt+O
   * outgoing) -- shows every real caller of (or callee called by) the symbol
   * under the cursor, each jumpable via the same `goToTarget` machinery
   * find-references already uses. */
  const triggerCallHierarchy = useCallback(
    (direction: CallDirection) => {
      const el = textareaRef.current;
      if (!el) return;
      const { line, character } = offsetToLineChar(el.value, el.selectionStart);
      const x = el.getBoundingClientRect().left + character * charWidth - el.scrollLeft;
      const y = el.getBoundingClientRect().top + line * lineHeightPx - el.scrollTop + lineHeightPx;
      pendingCallHierarchyRef.current = { line, character, direction };
      setCallHierarchyState({ x, y, direction, items: null });
      window.spartan
        .call("lsp_call_hierarchy", { doc_id: file.docId, line, character, direction })
        .catch((err: Error) => console.error("lsp_call_hierarchy failed:", err));
    },
    [charWidth, lineHeightPx, file.docId]
  );

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

  /** Real quick fixes / code actions (Alt+Enter, the standard cross-editor
   * convention) -- the ninth real LSP-backed editor feature, following
   * find-references' own exact "panel near the cursor, `items: null` while
   * in flight" shape. `actions` is the raw `CodeAction[]` the backend's
   * `lsp_code_action` handler merged for this position (it already issues
   * one `textDocument/codeAction` request per diagnostic covering the
   * caret and merges by title -- see that handler's own doc comment);
   * each action is kept verbatim because picking one must forward it
   * (data and all) to the real `codeAction/resolve` round trip. */
  const [quickFixState, setQuickFixState] = useState<{
    x: number;
    y: number;
    actions: unknown[] | null;
  } | null>(null);
  const pendingQuickFixRef = useRef<{ line: number; character: number } | null>(null);
  /** The raw action awaiting its `codeAction/resolve` reply -- set right
   * before `lsp_code_action_resolve` is sent, cleared by the resolve
   * handler. Only one resolve can be in flight per file at a time (the
   * popup closes on selection), matching `pendingRenameRef`'s own
   * single-slot convention. */
  const pendingResolveRef = useRef<{ action: unknown } | null>(null);

  const triggerQuickFix = useCallback(
    (line?: number, character?: number) => {
      const el = textareaRef.current;
      if (!el) return;
      const { line: l, character: c } =
        line === undefined || character === undefined
          ? offsetToLineChar(el.value, el.selectionStart)
          : { line, character };
      const x = el.getBoundingClientRect().left + c * charWidth - el.scrollLeft;
      const y = el.getBoundingClientRect().top + l * lineHeightPx - el.scrollTop + lineHeightPx;
      pendingQuickFixRef.current = { line: l, character: c };
      setQuickFixState({ x, y, actions: null });
      window.spartan
        .call("lsp_code_action", { doc_id: file.docId, line: l, character: c, diagnostics })
        .catch((err: Error) => console.error("lsp_code_action failed:", err));
    },
    [charWidth, lineHeightPx, file.docId, diagnostics]
  );

  /** Applies a resolved code action: its `edit` is a real `WorkspaceEdit`
   * (both real shapes, normalized by `extractWorkspaceEditChanges` and
   * applied multi-file through the same `onApplyRename` path F2 rename
   * uses), otherwise its `command` is run through `lsp_execute_command`. */
  const applyResolvedCodeAction = useCallback(
    (resolved: unknown) => {
      const action = resolved as { edit?: unknown; command?: unknown };
      const changes = extractWorkspaceEditChanges(action.edit);
      if (changes && onApplyRename) {
        onApplyRename(changes).catch((err: Error) =>
          console.error("code action apply failed:", err)
        );
        return;
      }
      const command = action.command as { command?: string; arguments?: unknown[] } | undefined;
      if (command && typeof command === "object") {
        window.spartan
          .call("lsp_execute_command", { doc_id: file.docId, command })
          .catch((err: Error) => console.error("lsp_execute_command failed:", err));
        return;
      }
      console.warn("resolved code action had neither edit nor command:", action);
    },
    [file.docId, onApplyRename]
  );

  // Real quick-fix handling -- both halves of the protocol, mirroring the
  // rename effect above: `lsp_code_action_result` fills the popup,
  // `lsp_code_action_resolve_result` applies the picked action.
  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event === "lsp_code_action_result") {
        const d = data as {
          doc_id: number;
          line: number;
          character: number;
          actions: unknown[];
        };
        if (d.doc_id !== file.docId) return;
        const pending = pendingQuickFixRef.current;
        if (!pending || pending.line !== d.line || pending.character !== d.character) return;
        pendingQuickFixRef.current = null;
        setQuickFixState((prev) => (prev ? { ...prev, actions: d.actions } : prev));
      } else if (event === "lsp_code_action_resolve_result") {
        const d = data as { doc_id: number; action: unknown };
        if (d.doc_id !== file.docId) return;
        if (!pendingResolveRef.current) return;
        pendingResolveRef.current = null;
        setQuickFixState(null);
        applyResolvedCodeAction(d.action);
      }
    });
    return unsubscribe;
  }, [file.docId, applyResolvedCodeAction]);

  /** Real document-symbol outline (Ctrl+Shift+O, the standard cross-editor
   * "Go to Symbol in File" convention) -- the seventh real LSP-backed
   * editor feature, following find-references' own exact "panel near the
   * cursor, items: null while in flight" shape, since a whole-document
   * request has no real per-request position of its own to key a pending
   * ref off of the way the other six do -- a single in-flight boolean is
   * enough here (only one outline request can be open at a time, matching
   * every other panel in this component). */
  const [symbolsState, setSymbolsState] = useState<{
    x: number;
    y: number;
    items: DocumentSymbolItem[] | null;
  } | null>(null);
  const pendingSymbolsRef = useRef(false);

  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event !== "lsp_document_symbol_result") return;
      const d = data as { doc_id: number; result: unknown };
      if (d.doc_id !== file.docId || !pendingSymbolsRef.current) return;
      pendingSymbolsRef.current = false;
      setSymbolsState((prev) =>
        prev ? { ...prev, items: extractDocumentSymbols(d.result) } : prev
      );
    });
    return unsubscribe;
  }, [file.docId]);

  const triggerDocumentSymbols = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    const { line, character } = offsetToLineChar(el.value, el.selectionStart);
    const x = el.getBoundingClientRect().left + character * charWidth - el.scrollLeft;
    const y = el.getBoundingClientRect().top + line * lineHeightPx - el.scrollTop + lineHeightPx;
    pendingSymbolsRef.current = true;
    setSymbolsState({ x, y, items: null });
    window.spartan
      .call("lsp_document_symbol", { doc_id: file.docId })
      .catch((err: Error) => console.error("lsp_document_symbol failed:", err));
  }, [charWidth, lineHeightPx, file.docId]);

  /** Real "Go to Line" (Ctrl+G) -- pure client-side, no LSP/backend query
   * needed: a real line number typed by the user, optionally followed by
   * `:column`, jumps the caret there via the same `jumpToLocalPosition`
   * every other real jump (definition/references/rename/symbols) already
   * shares. Centered rather than caret-anchored (unlike every other real
   * overlay in this file) since there's no real cursor position yet to
   * anchor to -- the user is about to jump *away* from wherever they are. */
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
      const totalLines = prevContentRef.current.split("\n").length;
      const requestedLine = Math.max(1, parseInt(match[1], 10));
      const line = Math.min(requestedLine, totalLines) - 1;
      const character = match[2] ? Math.max(0, parseInt(match[2], 10) - 1) : 0;
      jumpToLocalPosition(line, character);
      return null;
    });
  }, [jumpToLocalPosition]);

  /** Real document-highlight occurrence highlighting -- the ninth real
   * LSP-backed editor feature, and the first genuinely passive/automatic
   * one (every other real query method here is a manual trigger, whether
   * a keybinding or a mouse action): it fires on real cursor movement
   * alone, matching how every real editor's own "highlight all
   * occurrences" behavior works, and renders inline as colored rects
   * behind the real text rather than a popup, reusing the exact
   * scroll-synced layering `highlightRef`'s own syntax-color `<pre>`
   * already established (`syncScroll` now keeps this new layer's own
   * scroll position matched too). Debounced like hover (`HOVER_DELAY_MS`),
   * and only fires for a real plain cursor position (`selectionStart ===
   * selectionEnd`) -- an active text selection means something else is
   * already being done at that position, not "show me its occurrences". */
  const [documentHighlights, setDocumentHighlights] = useState<DocumentHighlightItem[]>([]);
  const pendingHighlightRef = useRef<{ line: number; character: number } | null>(null);
  const highlightDebounceRef = useRef<number | null>(null);

  /** Real matching-bracket highlighting -- unlike `documentHighlights`
   * above, this is pure, synchronous, local computation (`findMatchingBracket`),
   * so it updates on every real cursor move with no debounce and no
   * network round trip at all. */
  const [bracketMatch, setBracketMatch] = useState<[number, number] | null>(null);

  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event !== "lsp_document_highlight_result") return;
      const d = data as { doc_id: number; line: number; character: number; result: unknown };
      if (d.doc_id !== file.docId) return;
      const pending = pendingHighlightRef.current;
      if (!pending || pending.line !== d.line || pending.character !== d.character) return;
      setDocumentHighlights(extractDocumentHighlights(d.result));
    });
    return unsubscribe;
  }, [file.docId]);

  const handleSelectionChange = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    if (highlightDebounceRef.current !== null) {
      window.clearTimeout(highlightDebounceRef.current);
    }
    if (el.selectionStart !== el.selectionEnd) {
      // A real active selection -- clear any stale highlight from before
      // the selection started rather than leaving it visually stuck.
      setDocumentHighlights([]);
      setBracketMatch(null);
      return;
    }
    setBracketMatch(findMatchingBracket(el.value, el.selectionStart));
    highlightDebounceRef.current = window.setTimeout(() => {
      const { line, character } = offsetToLineChar(el.value, el.selectionStart);
      pendingHighlightRef.current = { line, character };
      window.spartan
        .call("lsp_document_highlight", { doc_id: file.docId, line, character })
        .catch((err: Error) => console.error("lsp_document_highlight failed:", err));
    }, HOVER_DELAY_MS);
  }, [file.docId]);

  /** Real, live LSP semantic-token highlighting (semantic highlighting) --
   * the P1 backlog item whose rust-analyzer support was confirmed by a real
   * live probe (rust-analyzer declares `semanticTokensProvider` and answers
   * `textDocument/semanticTokens/full` with real tokens; pyright declares
   * it too but returns `{data: null}` here, so this is genuinely a no-op
   * there). The backend's `lsp_semantic_tokens` returns decoded,
   * legend-resolved `{line, character, length, token_type, modifiers}`
   * spans for the whole document, rendered as colored marks in the same
   * overlay layer `documentHighlights` already uses. Two real, deliberate
   * correctness details: (1) the fetch is debounced on every content change
   * so marks track edits without hammering the LSP server, and (2) a
   * response is applied only when the buffer it was requested against is
   * still the current buffer -- otherwise a stale reply (the user typed
   * while the request was in flight) could paint marks at positions that
   * no longer match the text.
   *
   * The marks deliberately paint only token types the local highlighter
   * can't know (structs/functions/types/namespaces/...), never keywords,
   * strings, numbers, comments or operators, which the syntax layer already
   * colors -- see `SEMANTIC_TOKEN_MARK_CLASSES` for the exact mapping. */
  const [semanticTokens, setSemanticTokens] = useState<SemanticTokenItem[]>([]);
  /** Content snapshot the most recent in-flight request was made against;
   * `null` when no request is in flight. Compared against the live buffer
   * (via `latestSemanticContentRef`) when a response arrives. */
  const semanticRequestContentRef = useRef<string | null>(null);
  /** The most recently rendered buffer content -- assigned during render so
   * the event handler below can compare against the *current* buffer even
   * though its closure's `file.content` is frozen at the last render whose
   * `docId` matched. */
  const latestSemanticContentRef = useRef(file.content);
  latestSemanticContentRef.current = file.content;
  /** Bounded retry count for the first fetch -- a real, live e2e finding: a
   * rust-analyzer still finishing its initial indexing can genuinely answer
   * `textDocument/semanticTokens/full` with `{data: null}` (the first
   * request made right after open came back empty and the editor stayed
   * mark-less until an edit happened to trigger a fresh fetch). Reset to 0
   * on every new content-driven fetch so an idle file never exhausts the
   * budget permanently. */
  const semanticRetryRef = useRef(0);
  const semanticRetryTimerRef = useRef<number | null>(null);

  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event !== "lsp_semantic_tokens_result") return;
      const d = data as { doc_id: number; result: unknown };
      if (d.doc_id !== file.docId) return;
      if (semanticRequestContentRef.current !== latestSemanticContentRef.current) return;
      const tokens = extractSemanticTokens(d.result);
      if (tokens.length === 0 && semanticRetryRef.current < 5) {
        semanticRetryRef.current += 1;
        semanticRequestContentRef.current = latestSemanticContentRef.current;
        semanticRetryTimerRef.current = window.setTimeout(() => {
          window.spartan
            .call("lsp_semantic_tokens", { doc_id: d.doc_id })
            .catch((err: Error) => console.error("lsp_semantic_tokens failed:", err));
        }, 1500);
        return;
      }
      setSemanticTokens(tokens);
    });
    return () => {
      unsubscribe();
      if (semanticRetryTimerRef.current !== null) window.clearTimeout(semanticRetryTimerRef.current);
    };
  }, [file.docId]);

  useEffect(() => {
    setSemanticTokens([]);
    semanticRequestContentRef.current = null;
    semanticRetryRef.current = 0;
  }, [file.docId]);

  useEffect(() => {
    const docId = file.docId;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      if (cancelled) return;
      semanticRetryRef.current = 0;
      semanticRequestContentRef.current = file.content;
      window.spartan
        .call("lsp_semantic_tokens", { doc_id: docId })
        .catch((err: Error) => console.error("lsp_semantic_tokens failed:", err));
    }, 400);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [file.content, file.docId]);

  /** Real, live LSP inlay hints (type hints / parameter hints) -- the P2
   * backlog item whose rust-analyzer support was confirmed by a real live
   * probe: rust-analyzer declares `inlayHintProvider` and answers
   * `textDocument/inlayHint` with real hints (type hints like `: i32` and
   * parameter hints like `a:` with the real `paddingRight` flag set);
   * pyright declares `inlayHintProvider: null` here, so this is genuinely a
   * no-op there -- the LSP client never even asks a server that never
   * offered the capability. The backend's `lsp_inlay_hints` returns
   * already-decoded `{line, character, label, kind, padding_left,
   * padding_right}` hints for the whole document, rendered as text spans in
   * the same overlay layer `documentHighlights` and the semantic-token
   * marks already use. It shares the semantic-token fetch's two real
   * correctness details: debounced on every content change so hints track
   * edits without hammering the server, and a response applied only when
   * the buffer it was requested against is still the current buffer
   * (stale-reply guard). The identical bounded empty-result retry covers
   * the same live finding -- a rust-analyzer still finishing its initial
   * indexing can genuinely answer with an empty list the first time, and
   * without a retry an idle file would stay hint-less until an edit
   * happened to trigger a fresh fetch. */
  const [inlayHints, setInlayHints] = useState<InlayHintItem[]>([]);
  const inlayRequestContentRef = useRef<string | null>(null);
  const latestInlayContentRef = useRef(file.content);
  latestInlayContentRef.current = file.content;
  const inlayRetryRef = useRef(0);
  const inlayRetryTimerRef = useRef<number | null>(null);

  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event !== "lsp_inlay_hints_result") return;
      const d = data as { doc_id: number; result: unknown };
      if (d.doc_id !== file.docId) return;
      if (inlayRequestContentRef.current !== latestInlayContentRef.current) return;
      const hints = extractInlayHints(d.result);
      if (hints.length === 0 && inlayRetryRef.current < 5) {
        inlayRetryRef.current += 1;
        inlayRequestContentRef.current = latestInlayContentRef.current;
        inlayRetryTimerRef.current = window.setTimeout(() => {
          window.spartan
            .call("lsp_inlay_hints", { doc_id: d.doc_id })
            .catch((err: Error) => console.error("lsp_inlay_hints failed:", err));
        }, 1500);
        return;
      }
      setInlayHints(hints);
    });
    return () => {
      unsubscribe();
      if (inlayRetryTimerRef.current !== null) window.clearTimeout(inlayRetryTimerRef.current);
    };
  }, [file.docId]);

  useEffect(() => {
    setInlayHints([]);
    inlayRequestContentRef.current = null;
    inlayRetryRef.current = 0;
  }, [file.docId]);

  useEffect(() => {
    const docId = file.docId;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      if (cancelled) return;
      inlayRetryRef.current = 0;
      inlayRequestContentRef.current = file.content;
      window.spartan
        .call("lsp_inlay_hints", { doc_id: docId })
        .catch((err: Error) => console.error("lsp_inlay_hints failed:", err));
    }, 400);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [file.content, file.docId]);

  /** Real "Format Document" (Ctrl+Shift+F) -- the first real caller of the
   * registry's own `formatter` field (real since §20.1, unwired anywhere
   * until now). The backend runs the language's real formatter binary
   * against the *live buffer* (not disk) and reports the formatted text
   * back via a `format_document_result` event; applying it goes through
   * the same real whole-buffer `edit` IPC path typing already uses, so a
   * format is a single real undo checkpoint like any other edit. The
   * caret is restored to its old offset, clamped -- a formatter can move
   * text arbitrarily, so exact caret preservation is out of scope for a
   * real v1, matching the "smallest real, correct increment" precedent. */
  const [formatStatus, setFormatStatus] = useState<string | null>(null);
  const pendingFormatRef = useRef(false);
  /** Real completion signal for a real in-flight format request -- lets a
   * caller (Format on Save, below) `await` a full format cycle (including
   * the real `edit` call that applies it) before proceeding, without a
   * second, competing event subscription that could double-apply the
   * same real result. Resolved exactly once per cycle, then cleared. */
  const formatCompletionResolverRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event === "format_document_result") {
        const d = data as { doc_id: number; formatted: string };
        if (d.doc_id !== file.docId || !pendingFormatRef.current) return;
        pendingFormatRef.current = false;
        const resolve = formatCompletionResolverRef.current;
        formatCompletionResolverRef.current = null;
        if (d.formatted === prevContentRef.current) {
          setFormatStatus("Already formatted");
          window.setTimeout(() => setFormatStatus(null), 2500);
          resolve?.();
        } else {
          const oldLength = [...prevContentRef.current].length;
          const caret = textareaRef.current?.selectionStart ?? 0;
          shiftBreakpointsBeforeReplace(prevContentRef.current, d.formatted);
          prevContentRef.current = d.formatted;
          setLineCount(d.formatted.split("\n").length);
          onContentChange(file.path, d.formatted);
          setDocumentHighlights([]);
          window.spartan
            .call("edit", {
              doc_id: file.docId,
              start_char: 0,
              end_char: oldLength,
              text: d.formatted,
            })
            .catch((err: Error) => console.error("edit failed:", err))
            .then(() => resolve?.());
          const el = textareaRef.current;
          if (el) {
            const newPos = Math.min(caret, d.formatted.length);
            requestAnimationFrame(() => el.setSelectionRange(newPos, newPos));
          }
          setFormatStatus("Formatted");
          window.setTimeout(() => setFormatStatus(null), 2500);
        }
      } else if (event === "format_document_error") {
        const d = data as { doc_id: number; message: string };
        if (d.doc_id !== file.docId || !pendingFormatRef.current) return;
        pendingFormatRef.current = false;
        setFormatStatus(`Format failed: ${d.message}`);
        window.setTimeout(() => setFormatStatus(null), 5000);
        const resolve = formatCompletionResolverRef.current;
        formatCompletionResolverRef.current = null;
        resolve?.();
      }
    });
    return unsubscribe;
  }, [file.docId, file.path, onContentChange, shiftBreakpointsBeforeReplace]);

  /** Triggers a real format cycle and returns a promise that resolves once
   * it's fully settled (applied, already-formatted, or failed) -- Ctrl+
   * Shift+F fires this without awaiting it (unchanged v1 UX); Format on
   * Save awaits it before calling `save_file`, so the disk write always
   * sees the real formatted content, not a stale pre-format buffer. A
   * real, named safety bound: a wedged formatter can never hang a save
   * indefinitely -- if no real event arrives within 10s, this gives up
   * and lets the caller proceed with whatever the buffer already holds. */
  const triggerFormatDocument = useCallback((): Promise<void> => {
    return new Promise((resolve) => {
      formatCompletionResolverRef.current = resolve;
      pendingFormatRef.current = true;
      setFormatStatus("Formatting…");
      window.spartan.call("format_document", { doc_id: file.docId }).catch((err: Error) => {
        pendingFormatRef.current = false;
        if (formatCompletionResolverRef.current === resolve) {
          formatCompletionResolverRef.current = null;
        }
        // Real "no formatter configured/wired for this language" fallback
        // -- see `trimTrailingWhitespace`'s own doc comment for the full
        // real reasoning. Matched by message substring against the
        // backend's own three real synchronous-rejection shapes
        // (`format_document` in `spartan-backend::lib.rs`); deliberately
        // does NOT match any other real rejection reason (a poisoned
        // backend state, an already-closed document), which stay real,
        // honest failures instead of being silently papered over.
        const isNoFormatterError =
          err.message.includes("no formatter") ||
          err.message.includes("no language profile") ||
          err.message.includes("no supported stdin/stdout formatting mode");
        const el = textareaRef.current;
        if (isNoFormatterError && el) {
          const trimmed = trimTrailingWhitespace(prevContentRef.current);
          if (trimmed !== prevContentRef.current) {
            const oldLength = [...prevContentRef.current].length;
            const caret = el.selectionStart;
            shiftBreakpointsBeforeReplace(prevContentRef.current, trimmed);
            prevContentRef.current = trimmed;
            setLineCount(trimmed.split("\n").length);
            onContentChange(file.path, trimmed);
            setDocumentHighlights([]);
            setBracketMatch(null);
            window.spartan
              .call("edit", {
                doc_id: file.docId,
                start_char: 0,
                end_char: oldLength,
                text: trimmed,
              })
              .catch((editErr: Error) => console.error("edit failed:", editErr));
            const newPos = Math.min(caret, trimmed.length);
            requestAnimationFrame(() => el.setSelectionRange(newPos, newPos));
            setFormatStatus("No formatter configured -- trimmed trailing whitespace");
          } else {
            setFormatStatus("No formatter configured -- nothing to trim");
          }
          window.setTimeout(() => setFormatStatus(null), 3000);
        } else {
          setFormatStatus(`Format failed: ${err.message}`);
          window.setTimeout(() => setFormatStatus(null), 5000);
        }
        resolve();
      });
      window.setTimeout(() => {
        if (formatCompletionResolverRef.current === resolve) {
          formatCompletionResolverRef.current = null;
          resolve();
        }
      }, 10000);
    });
  }, [file.docId, file.path, onContentChange, shiftBreakpointsBeforeReplace]);

  /** Real "Find & Replace" (Ctrl+F / Ctrl+H) -- pure client-side, no LSP/
   * backend query needed, distinct from the already-real, cross-file
   * "Find in Files" panel (task #190-192): this searches only the
   * currently open buffer, with live next/prev navigation and an actual
   * replace/replace-all. `matches` is recomputed on every query/case-
   * sensitivity change *and* on every real edit (`handleChange` re-derives
   * it), so it never goes stale the way a one-shot search would after the
   * user keeps typing. `currentIndex` is clamped, not reset, when the
   * match count shrinks (e.g. the user is mid-Replace-All), so navigating
   * right after a replace doesn't jump to a surprising position. */
  const [findState, setFindState] = useState<{
    query: string;
    replaceQuery: string;
    showReplace: boolean;
    caseSensitive: boolean;
    currentIndex: number;
  } | null>(null);
  const findQueryInputRef = useRef<HTMLInputElement>(null);

  /** A real bug was found only by live-testing typing into the *replace*
   * field, not by inspection: depending on the whole `findState` object
   * (as `gotoLineState`'s own single-input overlay harmlessly does)
   * re-fires this effect on *every* keystroke, including ones typed into
   * the replace field -- since every keystroke there also produces a new
   * `findState` object reference -- silently yanking focus back to the
   * query input mid-type. `gotoLineState` never surfaced this because it
   * has only one input to fight over with itself. Fixed the same way
   * `renameState`'s own effect already avoids it: depend on a narrow,
   * only-changes-when-it-matters value (here, whether the box is open at
   * all) instead of the whole object. */
  useEffect(() => {
    if (findState) findQueryInputRef.current?.focus();
  }, [Boolean(findState)]);

  /** A second real bug, also found only by live testing (not the first
   * one above -- this one survived that fix): clicking "Replace All"
   * disables that same button the instant `findMatches` empties out, and
   * a browser drops keyboard focus entirely off an element the moment it
   * becomes `disabled` -- so a subsequent Escape press has no focused
   * descendant of the find box left to bubble through at all, and neither
   * the query/replace inputs' own `onKeyDown` nor a box-level one (tried
   * first, and confirmed via a second live re-test to still fail this
   * exact case) can ever see it. A real, global, capture-nothing-more-
   * than-necessary `window` listener, live only while the box is open, is
   * the standard fix for this whole class of "close on Escape regardless
   * of what currently holds focus" requirement -- and it doesn't double-
   * fire when an input already handled its own Escape, since that
   * handler's `e.stopPropagation()` genuinely stops the underlying native
   * event from ever reaching this bubble-phase `window` listener. */
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
    // `file.content` is deliberately in the dep list (not `findState`
    // itself, which would also fire on unrelated field changes) so
    // live-typing while the find bar is open keeps the match list and
    // highlights in sync with the real edit, not just the query text.
  }, [findState?.query, findState?.caseSensitive, file.content]);

  /** Real, shared "select this match in the textarea and scroll it into
   * view" -- the find-bar analogue of `jumpToLocalPosition`, but selects a
   * real range instead of collapsing to a point, since a found match is
   * something the user is about to act on (replace, or just see
   * highlighted), not just navigate past. */
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
    // Fires only when the match list itself changes (a new query, a live
    // edit) or `currentIndex` moves (navigate/replace) -- not on every
    // render, since `selectMatch` itself is stable across those.
  }, [findMatches, findState?.currentIndex]);

  const findNext = useCallback((direction: 1 | -1) => {
    setFindState((prev) => {
      if (!prev) return prev;
      const count = findMatches.length;
      if (count === 0) return prev;
      const next = ((prev.currentIndex + direction) % count + count) % count;
      return { ...prev, currentIndex: next };
    });
    // A functional `setFindState` update is used specifically so this
    // doesn't need `findState` itself as a dependency -- only `findMatches`.
  }, [findMatches]);

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
    // `applyProgrammaticEdit` is deliberately not in the dep list -- it's
    // declared later in this component (its own doc comment explains the
    // real TDZ hazard listing it here would hit), but referencing it only
    // inside this closure's *body* (not the dep array) is safe, the same
    // established pattern `handleKeyDown` already relies on below.
  }, [findMatches]);

  const replaceAll = useCallback(() => {
    setFindState((prev) => {
      if (!prev || findMatches.length === 0) return prev;
      const el = textareaRef.current;
      if (!el) return prev;
      const next = replaceAllMatches(el.value, findMatches, prev.replaceQuery);
      applyProgrammaticEdit(el, next, 0, 0);
      return { ...prev, currentIndex: 0 };
    });
    // Same real reason as `replaceCurrentMatch` above.
  }, [findMatches]);

  /** Real find-match highlight marks, the find-bar analogue of
   * `documentHighlights`' own render shape (a rect per line/char span in
   * the shared `editor-symbol-highlight-layer`). A real, named v1 scope
   * cut: only single-line matches render a mark (`startPos.line ===
   * endPos.line`, true for every ordinary query) -- a query containing a
   * literal newline is still fully navigable/replaceable via
   * `findMatches` itself, it just isn't drawn, since a query that spans
   * lines is a real edge case not worth the added multi-rect-per-match
   * complexity for this first increment. */
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

  /** Real bracket-pair colorization marks, recomputed whenever the real
   * document content changes -- a plain, pure `useMemo` over `file.content`
   * (not `prevContentRef`), matching `highlightedHtml`'s own dependency
   * exactly, since this is equally a pure function of the current content
   * with no cursor/selection state involved. */
  const bracketPairMarks = useMemo(() => computeBracketPairMarks(file.content), [file.content]);

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
    if (blameGutterRef.current) blameGutterRef.current.scrollTop = el.scrollTop;
    if (highlightRef.current) {
      highlightRef.current.scrollTop = el.scrollTop;
      highlightRef.current.scrollLeft = el.scrollLeft;
    }
    if (symbolHighlightRef.current) {
      symbolHighlightRef.current.scrollTop = el.scrollTop;
      symbolHighlightRef.current.scrollLeft = el.scrollLeft;
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
   * byte-identical `BackendEditor.tsx` proved it (this same technique,
   * same bug, ported here before it could ship broken in both places):
   * typing Tab or an auto-pairing bracket visibly updated the textarea's
   * raw DOM `.value` (so it *looked* correct on screen), but the real
   * backend document was never told about the change (`handleChange` --
   * and so the real `edit` IPC call inside it -- never fired), and a
   * subsequent real Ctrl+S wrote the file to disk *without* the Tab
   * indent or the auto-paired closing character at all -- a real, silent
   * data-loss bug with nothing to do with auto-closing brackets
   * specifically; Tab alone reproduced it identically. The exact
   * mechanism was never fully isolated (a minimal, React-free HTML
   * reproduction of the identical DOM technique worked perfectly, so
   * it's specific to how this component's own React tree processes a
   * *manually dispatched* native event, not a general DOM/browser
   * limitation) -- rather than keep relying on an event-dispatch
   * technique proven unreliable here, this function makes every
   * programmatic mutation call the *same* real update path a genuine
   * native input event already goes through, sidestepping the question
   * of whether React chooses to recognize the synthetic dispatch at
   * all. */
  const applyProgrammaticEdit = useCallback(
    (el: HTMLTextAreaElement, newContent: string, selStart: number, selEnd: number) => {
      el.value = newContent;
      el.setSelectionRange(selStart, selEnd);
      const oldLength = [...prevContentRef.current].length;
      // Snippet tab-stop tracking (P1 backlog): shift an active session's
      // stop offsets by this edit's delta before prevContentRef is
      // overwritten, so Tab still lands correctly after the user types.
      if (snippetSessionRef.current) {
        adjustSnippetStops(snippetSessionRef.current, prevContentRef.current, newContent);
      }
      // Real rope-anchored breakpoint shifting -- must run before
      // `prevContentRef.current` is overwritten below, since it needs
      // the real pre-edit text to compute what moved.
      shiftBreakpointsBeforeReplace(prevContentRef.current, newContent);
      prevContentRef.current = newContent;
      setLineCount(newContent.split("\n").length);
      onContentChange(file.path, newContent);
      // A real edit invalidates any real, already-resolved highlight
      // positions immediately -- `handleSelectionChange`'s own debounce
      // will naturally re-resolve them against the new content shortly.
      setDocumentHighlights([]);
      // Real matching-bracket highlighting recompute -- unlike
      // `documentHighlights`, this is pure/synchronous, so it's
      // recomputed here directly against the real post-edit content/
      // caret rather than waiting on a later `handleSelectionChange`
      // (whose own native "select" event isn't guaranteed to fire for
      // every programmatic edit, the same reliability gap this file's
      // own `applyProgrammaticEdit` doc comment already found once).
      setBracketMatch(findMatchingBracket(newContent, selStart));
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
      const pos = selStart;
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
    [
      charWidth,
      lineHeightPx,
      file.docId,
      file.path,
      onContentChange,
      shiftBreakpointsBeforeReplace,
    ]
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
      // Real document-symbol outline dismissal (Escape), same real
      // precedence as find-references' own identical branch above.
      if (e.key === "Escape" && symbolsState) {
        setSymbolsState(null);
      }
      // Real workspace-symbol palette dismissal (Escape), reached only if
      // focus somehow left the palette's own input (which handles its own
      // Escape directly); same real precedence as the branches above.
      if (e.key === "Escape" && wsSymbolsState) {
        closeWorkspaceSymbols();
      }
      // Real call-hierarchy dismissal (Escape), same real precedence.
      if (e.key === "Escape" && callHierarchyState) {
        setCallHierarchyState(null);
      }
      // Real quick-fix dismissal (Escape), same real precedence.
      if (e.key === "Escape" && quickFixState) {
        setQuickFixState(null);
      }
      // Real Find & Replace dismissal (Escape) reached only if the user
      // clicked back into the main textarea while the find bar stayed
      // open (its own query/replace inputs handle Escape directly, see
      // below -- this is the fallback for the other real focus case).
      if (e.key === "Escape" && findState) {
        setFindState(null);
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
      // Real, manual call-hierarchy triggers: Shift+Alt+H (incoming, "who
      // calls this") and Shift+Alt+O (outgoing, "what this calls"). `e.code`
      // (not `e.key`) since Alt+letter can produce a non-letter char on some
      // keyboard layouts.
      if (e.code === "KeyH" && e.shiftKey && e.altKey) {
        e.preventDefault();
        triggerCallHierarchy("incoming");
        return;
      }
      if (e.code === "KeyO" && e.shiftKey && e.altKey) {
        e.preventDefault();
        triggerCallHierarchy("outgoing");
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
      // Real quick-fix trigger (Alt+Enter, the standard cross-editor
      // convention for "show quick fixes at the cursor"). `e.code`
      // avoidance isn't needed here -- Enter has no layout variance the
      // way Alt+letter does -- but `e.altKey` (not `e.code`/`e.key` pair
      // matching) is what makes it Alt+Enter.
      if (e.altKey && e.key === "Enter" && !quickFixState) {
        e.preventDefault();
        triggerQuickFix();
        return;
      }
      // Real "Go to Line" trigger (Ctrl+G, the standard cross-editor
      // convention). Enter/Escape are handled by the input's own
      // `onKeyDown` below, a real distinct focused element like the
      // rename input above -- this branch never fires again while open.
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "g" && !gotoLineState) {
        e.preventDefault();
        setGotoLineState({ value: "" });
        return;
      }
      // Real "Find" (Ctrl+F, no Shift -- Ctrl+Shift+F stays Format
      // Document, handled separately below) and "Find & Replace" (Ctrl+H)
      // triggers, the standard cross-editor convention. A real, common
      // touch: an active selection at open time pre-fills the query, the
      // same convention nearly every mainstream editor already follows.
      // Enter/Shift+Enter/Escape are handled by the find bar's own inputs
      // below (a real, distinct focused element, like rename/goto-line
      // above) -- these branches never fire again while it's open.
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
      // Real, manual document-symbol outline trigger (Ctrl+Shift+O, the
      // standard cross-editor "Go to Symbol in File" convention).
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        triggerDocumentSymbols();
        return;
      }
      // Real "Go to Symbol in Workspace" trigger (Ctrl+T, VS Code's own
      // standard cross-editor convention for the same action). The
      // palette's own input handles Enter/Escape/Up/Down, so this branch
      // never fires again while it's open.
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "t" && !wsSymbolsState) {
        e.preventDefault();
        triggerWorkspaceSymbols();
        return;
      }
      // Real "Format Document" trigger (Ctrl+Shift+F).
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        triggerFormatDocument();
        return;
      }
      // Real inline git blame toggle (Alt+B). `e.code` (not `e.key`) since
      // Alt+letter can produce a special character on some keyboard layouts.
      if (e.altKey && !e.ctrlKey && !e.metaKey && e.code === "KeyB") {
        e.preventDefault();
        setBlameOn((v) => !v);
        return;
      }
      // Real "Toggle Line Comment" (Ctrl+/) -- see `toggleLineComment`'s
      // own doc comment. A real, honest no-op for a language with no
      // known single-line comment token (JSON/CSS/XML/Markdown) rather
      // than guessing wrong.
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
      // Real "Duplicate Line" (Ctrl+Shift+D) -- see `duplicateLines`'s own
      // doc comment.
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "d") {
        e.preventDefault();
        const el = textareaRef.current;
        if (el) {
          const result = duplicateLines(el.value, el.selectionStart, el.selectionEnd);
          applyProgrammaticEdit(el, result.content, result.selectionStart, result.selectionEnd);
        }
        return;
      }
      // Real "Move Line Up/Down" (Alt+Up/Alt+Down) -- see `moveLines`'s own
      // doc comment. A real document boundary (nothing to swap with) is a
      // genuine no-op -- `moveLines` returns `null` and no edit is applied.
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
      // Real "Delete Line" (Ctrl+Shift+K) -- see `deleteLines`'s own doc
      // comment.
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        const el = textareaRef.current;
        if (el) {
          const result = deleteLines(el.value, el.selectionStart, el.selectionEnd);
          applyProgrammaticEdit(el, result.content, result.selectionStart, result.selectionEnd);
        }
        return;
      }
      // Real "Join Lines" (Ctrl+J): merge the touched lines into one.
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "j") {
        e.preventDefault();
        const el = textareaRef.current;
        if (el) {
          const result = joinLines(el.value, el.selectionStart, el.selectionEnd);
          if (result) {
            applyProgrammaticEdit(el, result.content, result.selectionStart, result.selectionEnd);
          }
        }
        return;
      }
      // Real font-size zoom (Ctrl+=/Ctrl++ to zoom in, Ctrl+- to zoom
      // out, Ctrl+0 to reset) -- see `zoomFontSize`'s own doc comment for
      // the real "session-only delta on top of the persisted setting"
      // design. `e.key === "+"` covers the real, common case where a US
      // keyboard layout reports "+" for Ctrl+Shift+=/Ctrl++ rather than
      // "=" -- both zoom in identically.
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
        setFontSizeDelta(0);
        return;
      }
      // Clear an in-progress snippet session on navigation away from it.
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
        const indent = " ".repeat(prefs.tabSize);
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
          const snip = m ? findSnippet(languageForPath(file.path), m[1], prefs.userSnippets) : null;
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
        // Real multi-line indent/outdent -- matches every mainstream
        // editor's own convention: Shift+Tab always outdents the touched
        // line(s), even with a collapsed caret; plain Tab only switches to
        // full-line indent once a real selection is active, otherwise it
        // keeps its existing single-position insert-at-cursor behavior.
        if (e.shiftKey || start !== end) {
          const result = reindentLines(value, start, end, indent, e.shiftKey ? -1 : 1);
          applyProgrammaticEdit(el, result.content, result.selectionStart, result.selectionEnd);
          return;
        }
        const next = `${value.slice(0, start)}${indent}${value.slice(end)}`;
        applyProgrammaticEdit(el, next, start + indent.length, start + indent.length);
      }
      // Real auto-closing brackets/quotes (task #193). Checked after Tab
      // (so Tab's own real indent behavior is unaffected) and before
      // Ctrl+S/undo (neither of which this real single-character key can
      // ever collide with).
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
            // Real wrap-selection behavior: typing an opener with an
            // active selection wraps it instead of replacing it,
            // matching every mainstream editor's own convention.
            e.preventDefault();
            const selected = value.slice(start, end);
            const next = `${value.slice(0, start)}${e.key}${selected}${closeChar}${value.slice(end)}`;
            applyProgrammaticEdit(el, next, start + 1, start + 1 + selected.length);
            return;
          }
          // A real, deliberate v1 scope cut for quotes only: a quote
          // auto-pairs only when not immediately before a real word
          // character, so closing an existing string (typing `'` to
          // finish `it'`) doesn't insert a stray extra quote. Brackets
          // always auto-pair regardless of what follows.
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
      // Real "skip over" a real already-there closing bracket/quote --
      // typing the exact same closer just moves the caret past it
      // instead of inserting a real duplicate. A pure caret move, no
      // real content change, so no `edit` call is needed at all.
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
        const doSave = () => {
          window.spartan
            .call("save_file", { doc_id: file.docId })
            .then(() => {
              onContentChange(file.path, prevContentRef.current, true);
              // Blame is committed-file-relative; a save may have committed
              // nothing, but re-fetch so the just-saved buffer's line count
              // and any new commit are reflected without a manual re-toggle.
              if (blameOn) fetchBlame();
            })
            .catch((err: Error) => console.error("save failed:", err));
        };
        // Real Format on Save (task #187): await the real format cycle
        // (including its own real `edit` apply) before the real disk
        // write, so a save never races ahead of an in-flight reformat.
        if (prefs.formatOnSave) {
          triggerFormatDocument().then(doSave);
        } else {
          doSave();
        }
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
              shiftBreakpointsBeforeReplace(prevContentRef.current, r.content);
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
      gotoLineState,
      findState,
      symbolsState,
      triggerDocumentSymbols,
      triggerFormatDocument,
      callHierarchyState,
      triggerCallHierarchy,
      quickFixState,
      triggerQuickFix,
      file.docId,
      file.path,
      onContentChange,
      prefs.tabSize,
      prefs.formatOnSave,
      shiftBreakpointsBeforeReplace,
    ]
  );

  const lineNumbers = Array.from({ length: lineCount }, (_, i) => i + 1);

  // Real §75.76 editor preferences applied as inline overrides -- the
  // highlight layer and textarea must stay pixel-identical to each other
  // (the whole overlay technique depends on it), so both always receive
  // the exact same style object rather than one being styled via CSS and
  // the other via inline props.
  const textStyle: React.CSSProperties = {
    fontSize: `${effectiveFontSize}px`,
    lineHeight: `${lineHeightPx}px`,
    tabSize: prefs.tabSize,
    whiteSpace: prefs.wordWrap ? "pre-wrap" : "pre",
  };

  return (
    <div className="editor-root">
      {blameOn && (
        <div
          className="editor-blame-gutter mono"
          ref={blameGutterRef}
          style={textStyle}
          aria-hidden="true"
        >
          {lineNumbers.map((n) => {
            const b = blameLines[n - 1];
            if (!b || !b.oid || /^0+$/.test(b.oid)) {
              return (
                <div key={n} className="editor-blame-line editor-blame-line-empty">
                  {" "}
                </div>
              );
            }
            const short = b.oid.slice(0, 7);
            const age = formatBlameAge(b.time);
            const dateStr = b.time ? new Date(b.time * 1000).toLocaleString() : "";
            return (
              <div
                key={n}
                className="editor-blame-line"
                title={`${short} • ${b.author}${dateStr ? ` • ${dateStr}` : ""}${b.summary ? `\n${b.summary}` : ""}`}
              >
                <span className="editor-blame-author">{b.author || short}</span>
                {age && <span className="editor-blame-age"> {age}</span>}
              </div>
            );
          })}
        </div>
      )}
      <div className="editor-gutter mono" ref={gutterRef} style={textStyle}>
        {lineNumbers.map((n) => {
          // Real LSP positions are 0-indexed; `n` (the displayed line
          // number) is 1-indexed, matching every other real line-number
          // convention in this codebase, and matching real DAP
          // breakpoint/stop-frame line numbers directly (no translation).
          const lineDiags = diagnosticsByLine.get(n - 1);
          const severity = lineDiags ? worstSeverity(lineDiags) : null;
          const bp = breakpointMap.get(n);
          const hasBreakpoint = bp !== undefined;
          const isConditional = !!(bp && (bp.condition || bp.logMessage));
          const isLogpoint = !!(bp && bp.logMessage);
          const isStopped = stoppedLine === n;
          const bpTitle = bp
            ? isLogpoint
              ? `Logpoint: ${bp.logMessage}${bp.condition ? `\nCondition: ${bp.condition}` : ""}\n(right-click to edit)`
              : bp.condition
                ? `Conditional breakpoint: ${bp.condition}\n(right-click to edit)`
                : "Breakpoint (right-click to add a condition/logpoint)"
            : undefined;
          return (
            <div
              key={n}
              className={`editor-gutter-line${severity ? ` editor-gutter-line-${severity}` : ""}${isStopped ? " editor-gutter-line-stopped" : ""}`}
              title={
                bpTitle ?? lineDiags?.map((d) => `${d.severity}: ${d.message}`).join("\n")
              }
              onClick={() => onToggleBreakpoint?.(n)}
              onContextMenu={
                onEditBreakpoint
                  ? (e) => {
                      e.preventDefault();
                      setBreakpointEdit({
                        line: n,
                        condition: bp?.condition ?? "",
                        logMessage: bp?.logMessage ?? "",
                        top: e.clientY,
                      });
                    }
                  : undefined
              }
            >
              {onToggleBreakpoint && (
                <span
                  className={`editor-gutter-breakpoint-dot${hasBreakpoint ? " editor-gutter-breakpoint-dot-active" : ""}${isConditional ? " editor-gutter-breakpoint-dot-conditional" : ""}${isLogpoint ? " editor-gutter-breakpoint-dot-logpoint" : ""}`}
                />
              )}
              {lineDiags && lineDiags.length > 0 && (
                <span
                  className="editor-gutter-lightbulb"
                  title="Show quick fixes"
                  onClick={(e) => {
                    // `onClick` + `stopPropagation` (not `onMouseDown`) so
                    // the lightbulb never races the gutter row's own plain
                    // breakpoint-toggle click, and never opens a breakpoint
                    // menu either -- it is a real, distinct control.
                    e.stopPropagation();
                    e.preventDefault();
                    triggerQuickFix(lineDiags[0].line, lineDiags[0].character);
                  }}
                >
                  ⚡
                </span>
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
        <div
          className="editor-symbol-highlight-layer"
          ref={symbolHighlightRef}
          aria-hidden="true"
          style={textStyle}
        >
          {/* Real, deliberate scope limit: these marks are positioned from
           * the source line/character grid, which only matches the
           * rendered visual row when word wrap is off -- a soft-wrapped
           * line adds visual rows the mark's own `line * lineHeightPx`
           * math knows nothing about, so a mark after the wrap point
           * would render at the wrong spot. Gated off entirely with word
           * wrap on rather than shown wrong; real visual-row-aware
           * positioning is separate, larger work this pass doesn't
           * attempt. */}
          {!prefs.wordWrap &&
            bracketPairMarks.map((m, i) => (
              <div
                key={`bp:${m.line}:${m.character}:${i}`}
                className={`editor-bracket-pair-mark${m.colorIndex === -1 ? " editor-bracket-pair-mark-unmatched" : ` editor-bracket-pair-mark-${m.colorIndex}`}`}
                style={{
                  top: m.line * lineHeightPx,
                  left: m.character * charWidth,
                  width: charWidth,
                  height: lineHeightPx,
                }}
              />
            ))}
          {documentHighlights.map((h, i) => (
            <div
              key={`${h.startLine}:${h.startCharacter}:${h.endCharacter}:${i}`}
              className={`editor-symbol-highlight-mark${h.kind === 3 ? " editor-symbol-highlight-mark-write" : ""}`}
              style={{
                top: h.startLine * lineHeightPx,
                left: h.startCharacter * charWidth,
                width: Math.max(2, (h.endCharacter - h.startCharacter) * charWidth),
                height: lineHeightPx,
              }}
            />
          ))}
          {semanticTokens.map((t, i) => {
            const markClass = semanticTokenMarkClass(t.token_type);
            if (!markClass) return null;
            return (
              <div
                key={`st:${t.line}:${t.character}:${i}`}
                className={`editor-semantic-token-mark ${markClass}`}
                style={{
                  top: t.line * lineHeightPx,
                  left: t.character * charWidth,
                  width: Math.max(2, t.length * charWidth),
                  height: lineHeightPx,
                }}
              />
            );
          })}
          {!prefs.wordWrap &&
            inlayHints.map((h, i) => {
              const label = renderInlayHintLabel(h);
              return (
                <div
                  key={`ih:${h.line}:${h.character}:${i}`}
                  className={`editor-inlay-hint ${inlayHintKindClass(h.kind)}`}
                  style={{
                    top: h.line * lineHeightPx,
                    left: h.character * charWidth,
                    width: Math.max(2, label.length * charWidth),
                    height: lineHeightPx,
                  }}
                >
                  {label}
                </div>
              );
            })}
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
          onMouseMove={handleMouseMove}
          onMouseLeave={handleMouseLeave}
          onClick={handleDefinitionClick}
          onSelect={handleSelectionChange}
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
      {formatStatus && <div className="editor-format-status mono">{formatStatus}</div>}
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
                // otherwise leaves keyboard focus nowhere useful (the
                // input it was on is about to unmount), unlike the Enter
                // path, which already refocuses via `jumpToLocalPosition`
                // itself -- found by live testing a third Ctrl+G press
                // right after an Escape silently doing nothing.
                textareaRef.current?.focus();
              }
            }}
            onBlur={() => setGotoLineState(null)}
            placeholder={`Go to line (1-${lineCount})…`}
          />
        </div>
      )}
      {breakpointEdit && onEditBreakpoint && (
        <div
          className="editor-breakpoint-edit-box mono"
          style={{ top: breakpointEdit.top }}
          onClick={(e) => e.stopPropagation()}
        >
          <div className="editor-breakpoint-edit-title">Breakpoint · line {breakpointEdit.line}</div>
          <label className="editor-breakpoint-edit-field">
            <span>Condition</span>
            <input
              autoFocus
              className="editor-breakpoint-edit-input"
              value={breakpointEdit.condition}
              placeholder="e.g. i == 3"
              onChange={(e) =>
                setBreakpointEdit((prev) => (prev ? { ...prev, condition: e.target.value } : prev))
              }
              onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") {
                  e.preventDefault();
                  onEditBreakpoint(
                    breakpointEdit.line,
                    breakpointEdit.condition.trim(),
                    breakpointEdit.logMessage.trim()
                  );
                  setBreakpointEdit(null);
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  setBreakpointEdit(null);
                }
              }}
            />
          </label>
          <label className="editor-breakpoint-edit-field">
            <span>Log message</span>
            <input
              className="editor-breakpoint-edit-input"
              value={breakpointEdit.logMessage}
              placeholder="e.g. hit with x={x}"
              onChange={(e) =>
                setBreakpointEdit((prev) => (prev ? { ...prev, logMessage: e.target.value } : prev))
              }
              onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") {
                  e.preventDefault();
                  onEditBreakpoint(
                    breakpointEdit.line,
                    breakpointEdit.condition.trim(),
                    breakpointEdit.logMessage.trim()
                  );
                  setBreakpointEdit(null);
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  setBreakpointEdit(null);
                }
              }}
            />
          </label>
          <div className="editor-breakpoint-edit-actions">
            <button
              type="button"
              className="editor-find-btn"
              onClick={() => {
                onEditBreakpoint(
                  breakpointEdit.line,
                  breakpointEdit.condition.trim(),
                  breakpointEdit.logMessage.trim()
                );
                setBreakpointEdit(null);
              }}
            >
              Save
            </button>
            <button
              type="button"
              className="editor-find-btn"
              onClick={() => {
                // Real "clear back to a plain breakpoint" -- empty both.
                onEditBreakpoint(breakpointEdit.line, "", "");
                setBreakpointEdit(null);
              }}
            >
              Clear
            </button>
            <button
              type="button"
              className="editor-find-btn"
              onClick={() => setBreakpointEdit(null)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}
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
                // Real Ctrl+H-while-open: expands to show Replace instead
                // of falling through to the outer Ctrl+H handler (which
                // only fires when no find bar is open at all).
                if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "h") {
                  e.preventDefault();
                  setFindState((prev) => (prev ? { ...prev, showReplace: true } : prev));
                  return;
                }
                // Real Ctrl+F-while-open: most editors just re-focus/
                // re-select the query field rather than doing nothing.
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
      {symbolsState && (
        <div
          className="editor-references-panel mono"
          style={{ left: symbolsState.x, top: symbolsState.y }}
        >
          <div className="editor-references-header">
            {symbolsState.items === null
              ? "Loading symbols…"
              : `${symbolsState.items.length} symbol${symbolsState.items.length === 1 ? "" : "s"}`}
          </div>
          {symbolsState.items === null ? (
            <div className="editor-references-item editor-references-item-empty">Loading…</div>
          ) : symbolsState.items.length === 0 ? (
            <div className="editor-references-item editor-references-item-empty">
              No symbols found
            </div>
          ) : (
            symbolsState.items.map((item, i) => (
              <div
                key={`${item.name}:${item.line}:${item.character}:${i}`}
                className="editor-references-item editor-symbol-item"
                style={{ paddingLeft: 10 + item.depth * 14 }}
                onMouseDown={(e) => {
                  e.preventDefault();
                  setSymbolsState(null);
                  jumpToLocalPosition(item.line, item.character);
                }}
              >
                <span className="editor-symbol-kind">
                  {SYMBOL_KIND_LABELS[item.kind] ?? "Symbol"}
                </span>
                {item.name}
              </div>
            ))
          )}
        </div>
      )}
      {wsSymbolsState && (
        <div
          className="editor-references-panel mono editor-ws-symbols-panel"
          style={{ left: wsSymbolsState.x, top: wsSymbolsState.y }}
        >
          <div className="editor-ws-symbols-input-row">
            <input
              ref={wsSymbolsInputRef}
              className="editor-rename-input"
              value={wsSymbolsState.query}
              placeholder="Search workspace symbols…"
              onChange={handleWsSymbolsChange}
              onKeyDown={handleWsSymbolsKeyDown}
            />
          </div>
          <div className="editor-references-header">
            {wsSymbolsState.items === null
              ? "Searching workspace…"
              : `${wsSymbolsState.items.length} symbol${wsSymbolsState.items.length === 1 ? "" : "s"}`}
          </div>
          {wsSymbolsState.items === null ? (
            <div className="editor-references-item editor-references-item-empty">Loading…</div>
          ) : wsSymbolsState.items.length === 0 ? (
            <div className="editor-references-item editor-references-item-empty">
              No symbols found
            </div>
          ) : (
            wsSymbolsState.items.map((item, i) => (
              <div
                key={`${item.path}:${item.name}:${i}`}
                className={`editor-references-item editor-symbol-item${
                  i === wsSymbolsSelected ? " editor-ws-symbols-selected" : ""
                }`}
                onMouseDown={(e) => {
                  e.preventDefault();
                  setWsSymbolsState(null);
                  goToTarget({ path: item.path, line: item.line, character: item.character });
                }}
              >
                <span className="editor-symbol-kind">
                  {SYMBOL_KIND_LABELS[item.kind] ?? "Symbol"}
                </span>
                {item.name}
                {item.containerName ? (
                  <span className="editor-ws-symbols-container">{item.containerName}</span>
                ) : null}
                <span className="editor-ws-symbols-path">
                  {item.path === file.path
                    ? `line ${item.line + 1}`
                    : `${item.path}:${item.line + 1}`}
                </span>
              </div>
            ))
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
      {callHierarchyState && (
        <div
          className="editor-references-panel mono"
          style={{ left: callHierarchyState.x, top: callHierarchyState.y }}
        >
          {(() => {
            const noun = callHierarchyState.direction === "outgoing" ? "callee" : "caller";
            const items = callHierarchyState.items;
            return (
              <>
                <div className="editor-references-header">
                  {items === null
                    ? `Finding ${noun}s…`
                    : `${items.length} ${noun}${items.length === 1 ? "" : "s"}`}
                </div>
                {items === null ? (
                  <div className="editor-references-item editor-references-item-empty">Loading…</div>
                ) : items.length === 0 ? (
                  <div className="editor-references-item editor-references-item-empty">
                    No {noun}s found
                  </div>
                ) : (
                  items.map((item, i) => (
                    <div
                      key={`${item.name}:${item.path}:${item.line}:${item.character}:${i}`}
                      className="editor-references-item"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        setCallHierarchyState(null);
                        goToTarget(item);
                      }}
                    >
                      <span className="editor-symbol-kind">{noun}</span>
                      {item.name}
                      {item.path === file.path ? "" : ` — ${item.path}`}
                    </div>
                  ))
                )}
              </>
            );
          })()}
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
      {quickFixState && (
        <div
          className="editor-references-panel mono"
          style={{ left: quickFixState.x, top: quickFixState.y }}
        >
          <div className="editor-references-header">
            {quickFixState.actions === null
              ? "Finding quick fixes…"
              : `${quickFixState.actions.length} code action${quickFixState.actions.length === 1 ? "" : "s"}`}
          </div>
          {quickFixState.actions === null ? (
            <div className="editor-references-item editor-references-item-empty">Loading…</div>
          ) : quickFixState.actions.length === 0 ? (
            <div className="editor-references-item editor-references-item-empty">
              No code actions available
            </div>
          ) : (
            quickFixState.actions.map((action, i) => (
              <div
                key={`${codeActionTitle(action)}-${i}`}
                className="editor-references-item"
                onMouseDown={(e) => {
                  // `onMouseDown`, not `onClick` -- the same reasoning as
                  // the completion dropdown's own established pattern
                  // (fires before the textarea's blur, so the caret never
                  // moves before the request goes out).
                  e.preventDefault();
                  const raw = action as { title?: string; kind?: string };
                  setQuickFixState(null);
                  pendingResolveRef.current = { action: raw };
                  window.spartan
                    .call("lsp_code_action_resolve", { doc_id: file.docId, action: raw })
                    .catch((err: Error) =>
                      console.error("lsp_code_action_resolve failed:", err)
                    );
                }}
              >
                <span className="editor-symbol-kind">{codeActionKindLabel(action)}</span>
                {codeActionTitle(action)}
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
