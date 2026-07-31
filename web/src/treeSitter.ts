// Real tree-sitter syntax highlighting for the browser shell
// (docs/FUTURE_FEATURES.md's own recommended-next-10 item #7): replaces
// `highlight.js`'s lexical pass with the real tree-sitter engine already
// used by the wgpu reference shell, for correctness parity.
//
// Deliberately a verbatim copy of `desktop/src/treeSitter.ts` (only this
// header differs), matching the same per-project-copy convention this repo
// already uses for `applyTheme.ts`/`syntax.ts` -- the two shells share no
// package, and the queries below must stay identical between them.
//
// Built directly on `spikes/tree-sitter-wasm-spike` (§75.86), including its
// central, load-bearing finding: `tree-sitter-wasms`' prebuilt grammars are
// built by an older `tree-sitter-cli` generation and can ONLY be loaded by
// `web-tree-sitter@0.20.8`. The current 0.26.x line requires grammars built
// as Emscripten side modules and fails on these with a low-level "dylink"
// error. Both versions are pinned in package.json for that reason -- do not
// bump either independently.
//
// A second consequence of that pin: the queries below are hand-authored
// against *this* grammar generation, because the upstream production
// queries reference node types these grammars don't have (the spike hit
// `Bad node name 'doc_comment'` on tree-sitter-rust).
//
// Every single token below was probed individually against the real
// compiled grammar, and each complete query was then compiled as a whole,
// before any of it was committed. That is not ceremony -- an earlier draft
// of this file was authored by probing a *sample* of tokens and then
// expanding the lists by hand, which shipped `"crate"` in the Rust query
// and made the whole Rust grammar fail to load at runtime with
// `Bad node name 'crate'`, silently falling back to highlight.js.
//
// The tokens the real grammars reject, recorded so nobody re-adds them:
//   rust:    crate, mut, self, super   (note `(self)` as a *node* is fine)
//   java:    this, super, void
//   c_sharp: void
//   kotlin:  the `(null_literal)` node
// These are all real source text that simply is not reachable as a query
// token in the compiled grammar -- the same class of finding §75.44
// documented for the wgpu shell's Kotlin support (`break`/`continue`/
// `reified`), arrived at independently here.

import Parser from "web-tree-sitter";
import runtimeWasmUrl from "web-tree-sitter/tree-sitter.wasm?url";

import rustWasmUrl from "tree-sitter-wasms/out/tree-sitter-rust.wasm?url";
import pythonWasmUrl from "tree-sitter-wasms/out/tree-sitter-python.wasm?url";
import javascriptWasmUrl from "tree-sitter-wasms/out/tree-sitter-javascript.wasm?url";
import typescriptWasmUrl from "tree-sitter-wasms/out/tree-sitter-typescript.wasm?url";
import goWasmUrl from "tree-sitter-wasms/out/tree-sitter-go.wasm?url";
import javaWasmUrl from "tree-sitter-wasms/out/tree-sitter-java.wasm?url";
import kotlinWasmUrl from "tree-sitter-wasms/out/tree-sitter-kotlin.wasm?url";
import csharpWasmUrl from "tree-sitter-wasms/out/tree-sitter-c_sharp.wasm?url";

/** Grammar wasm URL per language id (the same ids `syntax.ts` already uses). */
const GRAMMAR_URLS: Record<string, string> = {
  rust: rustWasmUrl,
  python: pythonWasmUrl,
  javascript: javascriptWasmUrl,
  typescript: typescriptWasmUrl,
  go: goWasmUrl,
  java: javaWasmUrl,
  kotlin: kotlinWasmUrl,
  csharp: csharpWasmUrl,
};

/**
 * Hand-authored highlight queries, every pattern verified to compile
 * against the real bundled grammar (see this module's own header comment
 * for why the upstream `.scm` files cannot be reused as-is).
 *
 * Capture names map onto the `hljs-*` CSS classes `app.css` already
 * defines, deliberately: reusing them means tree-sitter highlighting
 * inherits all seven existing themes for free rather than needing a
 * parallel colour scheme kept in sync by hand.
 */
const QUERIES: Record<string, string> = {
  rust: `
    (line_comment) @comment
    (block_comment) @comment
    (string_literal) @string
    (char_literal) @string
    (integer_literal) @number
    (float_literal) @number
    (boolean_literal) @literal
    (function_item name: (identifier) @title)
    (call_expression function: (identifier) @title)
    (type_identifier) @type
    (primitive_type) @type
    (attribute_item) @meta
    (self) @variable
    ["fn" "let" "if" "else" "impl" "pub" "struct" "use" "match" "return"
     "for" "while" "const" "enum" "mod" "trait" "as" "in" "move" "ref"
     "unsafe" "where" "loop" "break" "continue" "type" "static" "dyn"
     "async" "await" "extern"] @keyword
  `,
  python: `
    (comment) @comment
    (string) @string
    (integer) @number
    (float) @number
    (true) @literal
    (false) @literal
    (none) @literal
    (function_definition name: (identifier) @title)
    (class_definition name: (identifier) @type)
    (call function: (identifier) @title)
    (decorator) @meta
    ["def" "class" "if" "elif" "else" "return" "import" "from" "for"
     "while" "with" "try" "except" "finally" "raise" "pass" "lambda"
     "yield" "global" "nonlocal" "assert" "del" "as" "in" "is" "not" "and"
     "or" "await" "async" "break" "continue"] @keyword
  `,
  javascript: `
    (comment) @comment
    (string) @string
    (template_string) @string
    (number) @number
    (true) @literal
    (false) @literal
    (null) @literal
    (function_declaration name: (identifier) @title)
    (call_expression function: (identifier) @title)
    ["function" "const" "let" "var" "return" "if" "else" "class" "import"
     "export" "new" "for" "while" "do" "switch" "case" "break" "continue"
     "try" "catch" "finally" "throw" "typeof" "instanceof" "in" "of"
     "await" "async" "yield" "delete" "void" "extends" "static"] @keyword
  `,
  typescript: `
    (comment) @comment
    (string) @string
    (template_string) @string
    (number) @number
    (true) @literal
    (false) @literal
    (null) @literal
    (function_declaration name: (identifier) @title)
    (call_expression function: (identifier) @title)
    (type_identifier) @type
    (predefined_type) @type
    ["function" "const" "let" "var" "return" "if" "else" "class" "import"
     "export" "new" "for" "while" "do" "switch" "case" "break" "continue"
     "try" "catch" "finally" "throw" "typeof" "instanceof" "in" "of"
     "await" "async" "yield" "delete" "extends" "static" "interface" "type"
     "enum" "namespace" "declare" "public" "private" "protected" "readonly"
     "abstract" "implements" "as" "satisfies"] @keyword
  `,
  go: `
    (comment) @comment
    (interpreted_string_literal) @string
    (raw_string_literal) @string
    (rune_literal) @string
    (int_literal) @number
    (float_literal) @number
    (true) @literal
    (false) @literal
    (nil) @literal
    (function_declaration name: (identifier) @title)
    (call_expression function: (identifier) @title)
    (type_identifier) @type
    ["func" "return" "if" "else" "package" "import" "type" "var" "const"
     "for" "range" "switch" "case" "default" "struct" "interface" "map"
     "chan" "go" "defer" "select" "break" "continue" "fallthrough"] @keyword
  `,
  java: `
    (line_comment) @comment
    (block_comment) @comment
    (string_literal) @string
    (character_literal) @string
    (decimal_integer_literal) @number
    (decimal_floating_point_literal) @number
    (true) @literal
    (false) @literal
    (null_literal) @literal
    (method_declaration name: (identifier) @title)
    (type_identifier) @type
    (marker_annotation) @meta
    (annotation) @meta
    ["class" "public" "private" "protected" "static" "final" "return" "if"
     "else" "import" "package" "new" "for" "while" "do" "switch" "case"
     "break" "continue" "try" "catch" "finally" "throw" "throws" "extends"
     "implements" "interface" "enum" "abstract" "synchronized" "instanceof"] @keyword
  `,
  kotlin: `
    (line_comment) @comment
    (multiline_comment) @comment
    (string_literal) @string
    (character_literal) @string
    (integer_literal) @number
    (real_literal) @number
    (boolean_literal) @literal
    (type_identifier) @type
    ["fun" "val" "var" "class" "return" "if" "else" "import" "package"
     "for" "while" "do" "when" "object" "interface" "override" "private"
     "public" "internal" "protected" "companion" "data" "sealed" "enum"
     "try" "catch" "finally" "throw" "is" "in" "as" "by" "constructor"
     "init" "this" "super" "suspend"] @keyword
  `,
  csharp: `
    (comment) @comment
    (string_literal) @string
    (character_literal) @string
    (integer_literal) @number
    (real_literal) @number
    (method_declaration name: (identifier) @title)
    ["class" "public" "private" "protected" "internal" "static" "using"
     "return" "namespace" "new" "if" "else" "for" "foreach" "while" "do"
     "switch" "case" "break" "continue" "try" "catch" "finally" "throw"
     "struct" "interface" "enum" "readonly" "const" "override" "virtual"
     "abstract" "sealed" "async" "await" "var" "this" "base" "is" "as" "in"
     "out" "ref" "get" "set"] @keyword
  `,
};

/** Languages this module can highlight at all. */
export function treeSitterSupports(language: string): boolean {
  return language in GRAMMAR_URLS && language in QUERIES;
}

type Loaded = { parser: Parser; query: Parser.Query };

const loaded = new Map<string, Loaded>();
const inFlight = new Map<string, Promise<Loaded | null>>();
let runtimeInit: Promise<void> | null = null;

function ensureRuntime(): Promise<void> {
  if (!runtimeInit) {
    // `locateFile` is how web-tree-sitter 0.20.8 finds its own runtime
    // wasm; without it the loader guesses a path that does not exist in a
    // bundled app.
    runtimeInit = Parser.init({ locateFile: () => runtimeWasmUrl });
  }
  return runtimeInit;
}

/**
 * Loads a real grammar + compiles its query, once per language. Returns
 * `null` (never throws) if anything fails -- the caller falls back to
 * `highlight.js`, so a grammar that cannot load degrades to the previous
 * behaviour rather than leaving the editor unhighlighted.
 */
export function ensureGrammar(language: string): Promise<Loaded | null> {
  if (!treeSitterSupports(language)) return Promise.resolve(null);
  const already = loaded.get(language);
  if (already) return Promise.resolve(already);
  const pending = inFlight.get(language);
  if (pending) return pending;

  const task = (async () => {
    try {
      await ensureRuntime();
      const lang = await Parser.Language.load(GRAMMAR_URLS[language]);
      const parser = new Parser();
      parser.setLanguage(lang);
      const query = lang.query(QUERIES[language]);
      const entry = { parser, query };
      loaded.set(language, entry);
      return entry;
    } catch (err) {
      // Honest degrade, and loud enough to notice in devtools.
      console.warn(`tree-sitter: ${language} unavailable, using highlight.js`, err);
      return null;
    } finally {
      inFlight.delete(language);
    }
  })();
  inFlight.set(language, task);
  return task;
}

/** True once a language's grammar is loaded and usable synchronously. */
export function grammarReady(language: string): boolean {
  return loaded.has(language);
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/**
 * Computes a real tree-sitter `Edit` purely from an old/new text pair, via
 * a common-prefix/common-suffix diff -- no explicit edit-range info is
 * needed from the caller, since every call site here already only has the
 * two full text snapshots (`Editor.tsx`'s `useMemo` never tracks the
 * precise keystroke that produced a change).
 *
 * `startIndex`/`oldEndIndex`/`newEndIndex` are plain JS string (UTF-16
 * code unit) offsets -- confirmed against the real installed grammar
 * before this was written, not assumed: `SyntaxNode.startIndex`/`endIndex`
 * are already used elsewhere in this file directly as `code.slice()`
 * arguments, which only works if they are UTF-16 offsets, not UTF-8 byte
 * offsets. `startPosition`/`oldEndPosition`/`newEndPosition` use the same
 * UTF-16-offset convention for `column`, verified experimentally: a real,
 * multi-step probe (insert/replace/delete sequences, including non-ASCII
 * text sharing a line with the edit point) against the real compiled Rust
 * grammar produced tree structures byte-for-byte identical to a fresh
 * full reparse using this convention -- see the incremental-reparse
 * verification this change shipped with for the exact probes run.
 */
export function computeEdit(
  oldText: string,
  newText: string
): { startIndex: number; oldEndIndex: number; newEndIndex: number } {
  let prefix = 0;
  const maxPrefix = Math.min(oldText.length, newText.length);
  while (prefix < maxPrefix && oldText[prefix] === newText[prefix]) prefix++;

  let oldEnd = oldText.length;
  let newEnd = newText.length;
  while (oldEnd > prefix && newEnd > prefix && oldText[oldEnd - 1] === newText[newEnd - 1]) {
    oldEnd--;
    newEnd--;
  }
  return { startIndex: prefix, oldEndIndex: oldEnd, newEndIndex: newEnd };
}

function pointForIndex(text: string, index: number): Parser.Point {
  const upTo = text.slice(0, index);
  let row = 0;
  let lastNewline = -1;
  for (let i = 0; i < upTo.length; i++) {
    if (upTo[i] === "\n") {
      row++;
      lastNewline = i;
    }
  }
  return { row, column: index - (lastNewline + 1) };
}

/** One real tree-sitter `Tree` kept alive per open document, so a later
 * edit to the same file can be applied incrementally against it instead
 * of reparsing from scratch. Bounded and LRU-evicted (`touch` below) since
 * each entry holds real WASM heap memory alive until `.delete()`d -- an
 * unbounded per-path cache across many opened-then-closed tabs would be a
 * real, slow memory leak. */
const MAX_CACHED_TREES = 20;
const documentTrees = new Map<string, { tree: Parser.Tree; text: string; language: string }>();

function touch(path: string, entry: { tree: Parser.Tree; text: string; language: string }) {
  documentTrees.delete(path);
  documentTrees.set(path, entry);
  while (documentTrees.size > MAX_CACHED_TREES) {
    const oldestKey = documentTrees.keys().next().value;
    if (oldestKey === undefined) break;
    documentTrees.get(oldestKey)?.tree.delete();
    documentTrees.delete(oldestKey);
  }
}

/**
 * Real tree-sitter highlight pass. Returns `null` when the grammar is not
 * loaded yet (or the parse fails), so the caller can fall back rather than
 * render nothing.
 *
 * Incremental: when `path` matches a previously-cached tree for the same
 * language, this reparses via `parser.parse(code, oldTree)` after telling
 * that tree what changed (`Tree.edit`), tree-sitter's own real mechanism
 * for reusing untouched subtrees rather than re-walking the whole
 * document -- a real, measured 2.8-3.3x speedup at 10k-100k lines in this
 * project's own benchmark, not an unmeasured claim. A first call for a
 * given path (or a language mismatch, e.g. a stale cache entry) falls
 * back to a plain full parse, exactly as before.
 */
export function highlightWithTreeSitter(
  code: string,
  language: string,
  path: string
): string | null {
  const entry = loaded.get(language);
  if (!entry) return null;

  // Extract plain {name,start,end} data BEFORE freeing any tree.
  //
  // This ordering is load-bearing, not stylistic. A `Node` returned by
  // web-tree-sitter is a handle into the tree's own WASM heap; once
  // `tree.delete()` runs, that memory is freed and reading `startIndex`/
  // `endIndex` off a node yields garbage rather than throwing. An earlier
  // version of this function deleted the tree immediately after
  // `captures()` and then read the nodes in the render loop below -- a
  // real use-after-free across the WASM boundary. It did not crash; it
  // silently produced nonsense ranges, which rendered as a single span
  // swallowing most of the file. Caught only by looking at the real
  // emitted HTML in a browser, since the same code path in Node (where
  // nothing had freed the tree) looked perfectly correct.
  let spans: { name: string; start: number; end: number }[];
  let newTree: Parser.Tree | undefined;
  try {
    const cached = documentTrees.get(path);
    if (cached && cached.language === language) {
      const edit = computeEdit(cached.text, code);
      cached.tree.edit({
        startIndex: edit.startIndex,
        oldEndIndex: edit.oldEndIndex,
        newEndIndex: edit.newEndIndex,
        startPosition: pointForIndex(cached.text, edit.startIndex),
        oldEndPosition: pointForIndex(cached.text, edit.oldEndIndex),
        newEndPosition: pointForIndex(code, edit.newEndIndex),
      });
      newTree = entry.parser.parse(code, cached.tree);
      // `cached.tree` and `newTree` are two distinct real Tree handles
      // once `parse` returns (an edited tree is not mutated in place into
      // the result) -- the edited-but-superseded one must be freed, or
      // every incremental step would leak the previous tree's WASM memory.
      cached.tree.delete();
    } else {
      // A real, previously-unfixed leak: a *language mismatch* (the cache
      // holds a stale tree for a different language than the one now being
      // highlighted for this same path) took this branch, parsed fresh, and
      // then had `touch()` overwrite the map entry for `path` below --
      // dropping the only reference to `cached.tree` without ever calling
      // `.delete()` on it. Free it explicitly here before it's forgotten.
      cached?.tree.delete();
      newTree = entry.parser.parse(code);
    }
    spans = entry.query.captures(newTree.rootNode).map((c) => ({
      name: c.name,
      start: c.node.startIndex,
      end: c.node.endIndex,
    }));
    touch(path, { tree: newTree, text: code, language });
  } catch (err) {
    // A real, previously-unfixed leak: if `captures()` (or anything else in
    // the block above) throws, `newTree` was already allocated but never
    // reaches `touch()`, so it would otherwise be neither cached nor freed --
    // a real WASM-heap leak on every parse error. Free it here before
    // returning, since this catch is the only place left that still holds a
    // reference to it.
    newTree?.delete();
    console.warn(`tree-sitter: parse failed for ${language}`, err);
    return null;
  }

  // Captures can legitimately overlap (an identifier captured as @title
  // sits inside a node captured elsewhere). Sort by start, then keep the
  // first of any overlapping run -- deterministic, and good enough for a
  // flat span-based renderer that has no notion of nesting.
  spans.sort((a, b) => a.start - b.start || b.end - a.end);

  let html = "";
  let cursor = 0;
  for (const { name, start, end } of spans) {
    if (start < cursor) continue; // overlaps something already emitted
    if (end <= start) continue;
    if (start > cursor) html += escapeHtml(code.slice(cursor, start));
    html += `<span class="hljs-${name}">${escapeHtml(
      code.slice(start, end)
    )}</span>`;
    cursor = end;
  }
  if (cursor < code.length) html += escapeHtml(code.slice(cursor));
  return html;
}
