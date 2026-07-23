// Curated code snippets, expanded by typing a prefix and pressing Tab,
// with tab-stop navigation (Tab jumps to the next placeholder). A small,
// self-contained subset of the VS Code snippet template syntax:
// `${N:default}` (a numbered stop with selectable default text), `$N`
// (a zero-width numbered stop), and `$0` (the final cursor position).
// No mirroring, no choices, no variables -- deliberately kept simple, and
// the curated bodies below avoid repeating any index so no mirroring is
// needed. Pure client-side; no backend round trip. Mirrored verbatim
// between `desktop/` and `web/` (the two shells don't share a package),
// the same convention `applyTheme.ts`/`syntax.ts` already established.

export interface Snippet {
  prefix: string;
  body: string;
  description: string;
}

/** One resolved tab stop in expanded text: absolute-within-the-expansion
 * `[start, end)` offsets. `end > start` means it carries selectable
 * default text; `end === start` is a bare cursor position. */
export interface TabStop {
  index: number;
  start: number;
  end: number;
}

export interface ExpandedSnippet {
  text: string;
  /** In tab order: index 1, 2, 3, … then `$0` (the final cursor) last. */
  stops: TabStop[];
}

/** Language id (matching `languageForPath`/hljs ids) -> its snippets. */
export const SNIPPETS: Record<string, Snippet[]> = {
  python: [
    { prefix: "def", body: "def ${1:name}(${2:args}):\n    ${0:pass}", description: "Function" },
    {
      prefix: "class",
      body: "class ${1:Name}:\n    def __init__(self${2:, args}):\n        ${0:pass}",
      description: "Class",
    },
    { prefix: "for", body: "for ${1:item} in ${2:iterable}:\n    ${0:pass}", description: "For loop" },
    { prefix: "while", body: "while ${1:condition}:\n    ${0:pass}", description: "While loop" },
    { prefix: "ifmain", body: 'if __name__ == "__main__":\n    ${0:main()}', description: "Main guard" },
    {
      prefix: "try",
      body: "try:\n    ${1:pass}\nexcept ${2:Exception} as ${3:e}:\n    ${0:pass}",
      description: "Try/except",
    },
  ],
  rust: [
    { prefix: "fn", body: "fn ${1:name}(${2}) ${3:-> ()} {\n    ${0}\n}", description: "Function" },
    { prefix: "test", body: "#[test]\nfn ${1:name}() {\n    ${0}\n}", description: "Test function" },
    { prefix: "match", body: "match ${1:expr} {\n    ${2:pattern} => ${0:{}},\n}", description: "Match" },
    { prefix: "impl", body: "impl ${1:Type} {\n    ${0}\n}", description: "Impl block" },
    { prefix: "struct", body: "struct ${1:Name} {\n    ${0}\n}", description: "Struct" },
    { prefix: "pln", body: "println!(\"${1}\"${0});", description: "println!" },
  ],
  typescript: [
    { prefix: "fn", body: "function ${1:name}(${2}) {\n    ${0}\n}", description: "Function" },
    { prefix: "afn", body: "const ${1:name} = (${2}) => {\n    ${0}\n};", description: "Arrow function" },
    { prefix: "for", body: "for (let i = 0; i < ${1:n}; i++) {\n    ${0}\n}", description: "For loop" },
    { prefix: "log", body: "console.log(${0});", description: "Console log" },
    { prefix: "interface", body: "interface ${1:Name} {\n    ${0}\n}", description: "Interface" },
  ],
  javascript: [
    { prefix: "fn", body: "function ${1:name}(${2}) {\n    ${0}\n}", description: "Function" },
    { prefix: "afn", body: "const ${1:name} = (${2}) => {\n    ${0}\n};", description: "Arrow function" },
    { prefix: "for", body: "for (let i = 0; i < ${1:n}; i++) {\n    ${0}\n}", description: "For loop" },
    { prefix: "log", body: "console.log(${0});", description: "Console log" },
  ],
  go: [
    { prefix: "func", body: "func ${1:name}(${2}) ${3} {\n    ${0}\n}", description: "Function" },
    { prefix: "for", body: "for i := 0; i < ${1:n}; i++ {\n    ${0}\n}", description: "For loop" },
    { prefix: "iferr", body: "if err != nil {\n    ${0:return err}\n}", description: "If err" },
    { prefix: "struct", body: "type ${1:Name} struct {\n    ${0}\n}", description: "Struct" },
  ],
  java: [
    { prefix: "sout", body: "System.out.println(${0});", description: "Print line" },
    { prefix: "fori", body: "for (int i = 0; i < ${1:n}; i++) {\n    ${0}\n}", description: "For loop" },
    { prefix: "main", body: "public static void main(String[] args) {\n    ${0}\n}", description: "Main method" },
  ],
  csharp: [
    { prefix: "cw", body: "Console.WriteLine(${0});", description: "Write line" },
    { prefix: "for", body: "for (int i = 0; i < ${1:n}; i++)\n{\n    ${0}\n}", description: "For loop" },
    { prefix: "prop", body: "public ${1:int} ${2:Name} { get; set; }${0}", description: "Property" },
  ],
  kotlin: [
    { prefix: "fun", body: "fun ${1:name}(${2}) {\n    ${0}\n}", description: "Function" },
    { prefix: "main", body: "fun main() {\n    ${0}\n}", description: "Main" },
  ],
};

/** Look up a snippet by language id + exact prefix, or `null`. */
export function findSnippet(langId: string | null, prefix: string): Snippet | null {
  if (!langId) return null;
  return SNIPPETS[langId]?.find((s) => s.prefix === prefix) ?? null;
}

/** Parse a snippet body into its literal text plus ordered tab stops. */
export function expandSnippet(body: string): ExpandedSnippet {
  let text = "";
  const raw: TabStop[] = [];
  let i = 0;
  while (i < body.length) {
    if (body[i] === "$") {
      const brace = /^\$\{(\d+):([^}]*)\}/.exec(body.slice(i));
      if (brace) {
        const start = text.length;
        text += brace[2];
        raw.push({ index: parseInt(brace[1], 10), start, end: start + brace[2].length });
        i += brace[0].length;
        continue;
      }
      const simple = /^\$(\d+)/.exec(body.slice(i));
      if (simple) {
        const start = text.length;
        raw.push({ index: parseInt(simple[1], 10), start, end: start });
        i += simple[0].length;
        continue;
      }
    }
    text += body[i];
    i += 1;
  }
  const ordered = raw.filter((s) => s.index !== 0).sort((a, b) => a.index - b.index);
  const zero = raw.find((s) => s.index === 0);
  ordered.push(zero ?? { index: 0, start: text.length, end: text.length });
  return { text, stops: ordered };
}

/** A live snippet session: absolute stop offsets in the whole document,
 * and which stop the cursor is currently on. */
export interface SnippetSession {
  stops: { start: number; end: number }[];
  index: number;
}

/** Shift a session's recorded stop offsets by the delta of an edit, so
 * Tab still lands correctly after the user types at a stop. Computes the
 * edit's start (common-prefix length) and delta from old vs. new text --
 * robust for a single contiguous edit (typing, deleting, pasting). */
export function adjustSnippetStops(session: SnippetSession, oldText: string, newText: string): void {
  if (oldText === newText) return;
  let p = 0;
  const minLen = Math.min(oldText.length, newText.length);
  while (p < minLen && oldText[p] === newText[p]) p += 1;
  const delta = newText.length - oldText.length;
  for (const stop of session.stops) {
    if (stop.start >= p) stop.start += delta;
    if (stop.end >= p) stop.end += delta;
  }
}
