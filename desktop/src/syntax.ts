// Real client-side syntax highlighting (originally from the §75.62 audit's
// own prioritized backlog: "Editor.tsx is a plain unstyled textarea").
//
// This started as a deliberate, named `highlight.js`-only v1, with real
// tree-sitter parity called out as future work. That work is now done:
// `treeSitter.ts` runs the real tree-sitter engine in-process via
// `web-tree-sitter`, and this module prefers it whenever a grammar is
// loaded, keeping `highlight.js` as a genuine fallback rather than
// removing it -- it still covers the languages with no bundled grammar
// (json/css/xml/markdown/bash) and the window before a grammar finishes
// loading. See `highlightSource` below for the exact tier order, and
// `treeSitter.ts`'s header for the real version-pin and query-authoring
// constraints that shape it.

import { grammarReady, highlightWithTreeSitter } from "./treeSitter";
import hljs from "highlight.js/lib/core";
import rust from "highlight.js/lib/languages/rust";
import typescript from "highlight.js/lib/languages/typescript";
import javascript from "highlight.js/lib/languages/javascript";
import python from "highlight.js/lib/languages/python";
import kotlin from "highlight.js/lib/languages/kotlin";
import java from "highlight.js/lib/languages/java";
import go from "highlight.js/lib/languages/go";
import csharp from "highlight.js/lib/languages/csharp";
import xml from "highlight.js/lib/languages/xml";
import css from "highlight.js/lib/languages/css";
import json from "highlight.js/lib/languages/json";
import markdown from "highlight.js/lib/languages/markdown";
import bash from "highlight.js/lib/languages/bash";

hljs.registerLanguage("rust", rust);
hljs.registerLanguage("typescript", typescript);
hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("python", python);
hljs.registerLanguage("kotlin", kotlin);
hljs.registerLanguage("java", java);
hljs.registerLanguage("go", go);
hljs.registerLanguage("csharp", csharp);
hljs.registerLanguage("xml", xml);
hljs.registerLanguage("css", css);
hljs.registerLanguage("json", json);
hljs.registerLanguage("markdown", markdown);
hljs.registerLanguage("bash", bash);

/** Real extension -> hljs-registered-language mapping, matching the same
 * 7 real Tier 1 languages `spartan-languages`'s own `languages.toml`
 * covers (§20, §75.51), plus a few common config/markup languages a real
 * project's file tree will contain. `null` means "no highlighting" (a
 * real, honest fallback, not a crash) rather than guessing wrong. */
const EXTENSION_TO_LANGUAGE: Record<string, string> = {
  rs: "rust",
  ts: "typescript",
  tsx: "typescript",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  py: "python",
  kt: "kotlin",
  kts: "kotlin",
  java: "java",
  go: "go",
  cs: "csharp",
  html: "xml",
  xml: "xml",
  css: "css",
  json: "json",
  md: "markdown",
  sh: "bash",
  bash: "bash",
};

export function languageForPath(path: string): string | null {
  const ext = path.split(".").pop()?.toLowerCase();
  if (!ext) return null;
  return EXTENSION_TO_LANGUAGE[ext] ?? null;
}

/** Escapes real HTML metacharacters in plain (non-highlighted) text --
 * needed for the language-not-recognized fallback, where the raw source
 * is rendered verbatim into the same `dangerouslySetInnerHTML` overlay
 * highlighted output uses, and must not be interpreted as real markup. */
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/** Real, defensive highlighting, in three tiers.
 *
 * 1. Real tree-sitter (`treeSitter.ts`), once that language's grammar has
 *    finished loading -- a real parse, not a lexical guess, matching the
 *    wgpu reference shell's own engine. This is the roadmap item this
 *    module's original header comment named as future work.
 * 2. `highlight.js`, used while a grammar is still loading, for languages
 *    with no tree-sitter grammar bundled (json/css/xml/markdown/bash), and
 *    if tree-sitter fails for any reason at all.
 * 3. Plain escaped text if even that throws.
 *
 * Grammar loading is asynchronous and deliberately kept off this path:
 * this function stays synchronous so the render path is unchanged. The
 * caller (`Editor.tsx`) kicks off `ensureGrammar` and re-renders once it
 * resolves, so the first paint is highlight.js and every subsequent one is
 * tree-sitter. */
export function highlightSource(source: string, path: string): string {
  const language = languageForPath(path);
  if (!language) return escapeHtml(source);
  if (grammarReady(language)) {
    const viaTreeSitter = highlightWithTreeSitter(source, language, path);
    if (viaTreeSitter !== null) return viaTreeSitter;
  }
  try {
    return hljs.highlight(source, { language, ignoreIllegals: true }).value;
  } catch {
    return escapeHtml(source);
  }
}
