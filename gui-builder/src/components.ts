/**
 * Real component discovery for the Design screen's component-library
 * browser (task #278) -- the last piece §75.90 named as GUI Builder's
 * own remaining MVP gap.
 *
 * Scans a real project directory for `.jsx`/`.tsx` files, parses each
 * with the exact same `parserAdapter` every other module here uses, and
 * reports the React components each file exports. No code is executed:
 * this is a pure AST read, the same discipline `parse.ts` already
 * follows.
 *
 * **The "is this a component?" test is a real, named heuristic, not
 * analysis**: an exported binding whose name starts with an uppercase
 * letter. That is exactly the rule React itself enforces at the JSX call
 * site (a lowercase tag is a DOM element, an uppercase one is a
 * component), so it matches how the code will actually behave -- but it
 * genuinely cannot tell an exported component apart from an exported
 * class, constant, or type-like value that happens to be capitalized.
 * Naming it here rather than pretending to deeper certainty.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, dirname, extname, sep } from "node:path";
import { parserAdapter } from "./parserAdapter.js";

export interface DiscoveredComponent {
  /** The real exported binding name, e.g. `"Card"`. */
  name: string;
  /** Absolute path of the real file that exports it. */
  file: string;
  /** Whether it is that file's `export default` -- decides whether an
   * inserting caller needs `import Card from "..."` or
   * `import { Card } from "..."`. */
  isDefault: boolean;
  /**
   * A real, extension-stripped relative module specifier from whichever
   * file the caller named as the insertion target (e.g. `"./Card"`,
   * `"../shared/Button"`), or `null` when the component is declared in
   * that same file and therefore needs no import at all. Computed here
   * rather than in the UI because it is a genuine path concern and the
   * renderer process has no `node:path`.
   */
  importFrom: string | null;
}

/** Directories never worth walking for a component palette -- build
 * output and dependency trees would swamp a real project's own
 * components with thousands of irrelevant matches. */
const SKIP_DIRS = new Set([
  "node_modules",
  ".git",
  "dist",
  "build",
  ".next",
  "coverage",
  "out",
  ".cache",
]);

const COMPONENT_EXTENSIONS = new Set([".jsx", ".tsx"]);

/** Depth-bounded so a pathologically deep (or symlink-looped) tree can
 * never hang the caller -- the same bounded-walk discipline
 * `spartan-leo`'s own `search_files` already applies. */
const MAX_DEPTH = 12;

function isComponentName(name: string): boolean {
  return /^[A-Z]/.test(name);
}

function collectFiles(dir: string, depth: number, out: string[]): void {
  if (depth > MAX_DEPTH) return;
  let entries: string[];
  try {
    entries = readdirSync(dir);
  } catch {
    // An unreadable directory is a real, expected case (permissions, a
    // dangling symlink) -- skip it rather than failing the whole scan.
    return;
  }
  for (const entry of entries) {
    if (entry.startsWith(".") && entry !== ".") {
      if (SKIP_DIRS.has(entry)) continue;
    }
    if (SKIP_DIRS.has(entry)) continue;
    const full = join(dir, entry);
    let stats;
    try {
      stats = statSync(full);
    } catch {
      continue;
    }
    if (stats.isDirectory()) {
      collectFiles(full, depth + 1, out);
    } else if (COMPONENT_EXTENSIONS.has(extname(entry))) {
      out.push(full);
    }
  }
}

/** Pulls every exported, capitalized binding name out of one already-
 * parsed module body, tagging which one (if any) is the default export. */
function exportedComponentNames(body: unknown[]): { name: string; isDefault: boolean }[] {
  const found: { name: string; isDefault: boolean }[] = [];
  for (const raw of body) {
    const node = raw as Record<string, unknown>;
    if (node.type === "ExportDefaultDeclaration") {
      const decl = node.declaration as Record<string, unknown> | undefined;
      if (!decl) continue;
      // `export default function Card() {}` / `export default class Card {}`
      const id = decl.id as Record<string, unknown> | undefined;
      if (id && typeof id.name === "string") {
        found.push({ name: id.name, isDefault: true });
        continue;
      }
      // `export default Card;` -- a binding declared elsewhere in the file.
      if (decl.type === "Identifier" && typeof decl.name === "string") {
        found.push({ name: decl.name, isDefault: true });
      }
      // An anonymous `export default () => ...` has no name to offer a
      // palette, so it is deliberately skipped rather than invented.
      continue;
    }
    if (node.type !== "ExportNamedDeclaration") continue;
    const decl = node.declaration as Record<string, unknown> | undefined;
    if (decl) {
      if (decl.type === "FunctionDeclaration" || decl.type === "ClassDeclaration") {
        const id = decl.id as Record<string, unknown> | undefined;
        if (id && typeof id.name === "string") found.push({ name: id.name, isDefault: false });
      } else if (decl.type === "VariableDeclaration") {
        for (const d of (decl.declarations as Record<string, unknown>[]) ?? []) {
          const id = d.id as Record<string, unknown> | undefined;
          if (id && typeof id.name === "string") found.push({ name: id.name, isDefault: false });
        }
      }
      continue;
    }
    // `export { Card, Button };`
    for (const spec of (node.specifiers as Record<string, unknown>[]) ?? []) {
      const exported = spec.exported as Record<string, unknown> | undefined;
      if (exported && typeof exported.name === "string") {
        found.push({ name: exported.name, isDefault: exported.name === "default" });
      }
    }
  }
  return found.filter((f) => isComponentName(f.name));
}

/** Builds the real relative module specifier a JSX import would use --
 * POSIX-separated (module specifiers always are, even on Windows) and
 * extension-stripped, with a leading `./` when it would otherwise look
 * like a bare package name. */
export function relativeSpecifier(fromFile: string, toFile: string): string {
  let rel = relative(dirname(fromFile), toFile);
  rel = rel.split(sep).join("/");
  const ext = extname(rel);
  if (ext) rel = rel.slice(0, -ext.length);
  if (!rel.startsWith(".")) rel = `./${rel}`;
  return rel;
}

/** Discovers exported components from an already-loaded source buffer. This
 * is the live-editor counterpart to `discoverComponents`; it never reads
 * from disk and therefore reflects unsaved exports immediately. */
export function discoverComponentsInSource(source: string, file: string, fromFile?: string): DiscoveredComponent[] {
  let ast: { program?: { body?: unknown[] } };
  try {
    ast = parserAdapter.parse(source) as { program?: { body?: unknown[] } };
  } catch {
    return [];
  }
  return exportedComponentNames(ast.program?.body ?? []).map(({ name, isDefault }) => ({
    name,
    file,
    isDefault,
    importFrom: fromFile && file === fromFile ? null : fromFile ? relativeSpecifier(fromFile, file) : null,
  }));
}

/**
 * Discovers every exported React component under `rootDir`. When
 * `fromFile` is given, each result also carries the real relative
 * specifier an import in that file would need (`null` for components
 * declared in `fromFile` itself, which need no import).
 *
 * A file that fails to parse is skipped rather than failing the whole
 * scan -- a real project routinely contains a file mid-edit, and a
 * component palette that refuses to open because one unrelated file is
 * temporarily invalid would be strictly worse than one that lists
 * everything it could actually read.
 */
export function discoverComponents(rootDir: string, fromFile?: string): DiscoveredComponent[] {
  const files: string[] = [];
  collectFiles(rootDir, 0, files);
  files.sort();

  const out: DiscoveredComponent[] = [];
  for (const file of files) {
    try { out.push(...discoverComponentsInSource(readFileSync(file, "utf8"), file, fromFile)); } catch { /* skip unreadable files */ }
  }
  return out;
}
