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
import { join, relative, dirname, extname, sep, resolve as resolvePath } from "node:path";
import { parserAdapter } from "./parserAdapter.js";

export interface ComponentPropHint {
  /** Public prop name as it appears in the component's props type/pattern. */
  name: string;
  /** Source-level type text when TypeScript declared one, otherwise unknown. */
  type: string;
  /** False for an optional field or a destructured field with a default. */
  required: boolean;
  /** Verbatim default expression when the parameter destructures one. */
  defaultValue?: string;
}

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
  /** Project-wide direct JSX usage scan; absent for source-only discovery. */
  usageCount?: number;
  usageFiles?: string[];
  /** Parsed from a real leading `@deprecated` JSDoc tag, when present. */
  deprecated?: boolean;
  replacement?: string;
  /** Safe, source-level hints for the component's public props API. */
  propHints?: ComponentPropHint[];
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
function deprecationMetadata(node: Record<string, unknown>): { deprecated?: boolean; replacement?: string } {
  const comments = Array.isArray(node.leadingComments) ? node.leadingComments : [];
  const text = comments.map((comment) => {
    const value = comment as Record<string, unknown>;
    return typeof value.value === "string" ? value.value : "";
  }).join("\n");
  if (!/@deprecated\b/i.test(text)) return {};
  const replacement = text.match(/@(?:replacement|see)\s+[`']?([A-Z][A-Za-z0-9_.]*)[`']?/i)
    ?? text.match(/@deprecated[\s\S]*?\buse\s+[`']?([A-Z][A-Za-z0-9_.]*)[`']?\s+instead/i);
  return { deprecated: true, ...(replacement ? { replacement: replacement[1] } : {}) };
}

function exportedComponentNames(body: unknown[]): { name: string; isDefault: boolean; deprecated?: boolean; replacement?: string }[] {
  const found: { name: string; isDefault: boolean; deprecated?: boolean; replacement?: string }[] = [];
  for (const raw of body) {
    const node = raw as Record<string, unknown>;
    if (node.type === "ExportDefaultDeclaration") {
      const decl = node.declaration as Record<string, unknown> | undefined;
      if (!decl) continue;
      // `export default function Card() {}` / `export default class Card {}`
      const id = decl.id as Record<string, unknown> | undefined;
      if (id && typeof id.name === "string") {
        found.push({ name: id.name, isDefault: true, ...deprecationMetadata(node) });
        continue;
      }
      // `export default Card;` -- a binding declared elsewhere in the file.
      if (decl.type === "Identifier" && typeof decl.name === "string") {
        found.push({ name: decl.name, isDefault: true, ...deprecationMetadata(node) });
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
        if (id && typeof id.name === "string") found.push({ name: id.name, isDefault: false, ...deprecationMetadata(node) });
      } else if (decl.type === "VariableDeclaration") {
        for (const d of (decl.declarations as Record<string, unknown>[]) ?? []) {
          const id = d.id as Record<string, unknown> | undefined;
          if (id && typeof id.name === "string") found.push({ name: id.name, isDefault: false, ...deprecationMetadata(node) });
        }
      }
      continue;
    }
    // `export { Card, Button };`
    for (const spec of (node.specifiers as Record<string, unknown>[]) ?? []) {
      const exported = spec.exported as Record<string, unknown> | undefined;
      if (exported && typeof exported.name === "string") {
        found.push({ name: exported.name, isDefault: exported.name === "default", ...deprecationMetadata(node) });
      }
    }
  }
  return found.filter((f) => isComponentName(f.name));
}

function nodeText(source: string, node: unknown): string {
  if (!node || typeof node !== "object") return "";
  const value = node as Record<string, unknown>;
  return typeof value.start === "number" && typeof value.end === "number"
    ? source.slice(value.start, value.end).trim()
    : "";
}

function identifierName(node: unknown): string | null {
  if (!node || typeof node !== "object") return null;
  const value = node as Record<string, unknown>;
  return value.type === "Identifier" && typeof value.name === "string" ? value.name : null;
}

function typeName(node: unknown): string | null {
  if (!node || typeof node !== "object") return null;
  const value = node as Record<string, unknown>;
  if (value.type === "TSTypeReference") return identifierName(value.typeName);
  return null;
}

function typeMembers(node: unknown): Record<string, Record<string, unknown>> {
  if (!node || typeof node !== "object") return {};
  const value = node as Record<string, unknown>;
  if (value.type === "TSInterfaceDeclaration") {
    const body = value.body as Record<string, unknown> | undefined;
    return Object.fromEntries(((body?.body as Record<string, unknown>[]) ?? [])
      .map((member) => [identifierName(member.key), member] as const)
      .filter(([name]) => Boolean(name)));
  }
  if (value.type === "TSTypeAliasDeclaration") {
    const annotation = value.typeAnnotation as Record<string, unknown> | undefined;
    if (annotation?.type === "TSTypeLiteral") {
      return Object.fromEntries(((annotation.members as Record<string, unknown>[]) ?? [])
        .map((member) => [identifierName(member.key), member] as const)
        .filter(([name]) => Boolean(name)));
    }
  }
  return {};
}

type PropTypeMembers = Record<string, Record<string, unknown>>;
type PropTypeCatalog = Map<string, PropTypeMembers>;
const MAX_IMPORTED_TYPE_FILES = 256;

function attachTypeSource(members: PropTypeMembers, source: string): PropTypeMembers {
  return Object.fromEntries(Object.entries(members).map(([name, member]) => [name, { ...member, __source: source }]));
}

function parsedBody(source: string): unknown[] {
  try {
    const ast = parserAdapter.parse(source) as { program?: { body?: unknown[] } };
    return ast.program?.body ?? [];
  } catch {
    return [];
  }
}

function typeDeclarations(source: string): Map<string, PropTypeMembers> {
  const result = new Map<string, PropTypeMembers>();
  for (const raw of parsedBody(source)) {
    const node = raw as Record<string, unknown>;
    const declaration = node.type === "ExportNamedDeclaration" ? node.declaration as Record<string, unknown> | undefined : node;
    if (!declaration || (declaration.type !== "TSInterfaceDeclaration" && declaration.type !== "TSTypeAliasDeclaration")) continue;
    const name = identifierName(declaration.id);
    if (name) result.set(name, attachTypeSource(typeMembers(declaration), source));
  }
  return result;
}

/** Resolves only relative imports inside the project root. Bare package
 * imports and paths escaping the root are intentionally ignored, so type
 * discovery never turns the palette scan into arbitrary filesystem access. */
function resolveProjectImport(rootDir: string, fromFile: string, specifier: string): string | null {
  if (!specifier.startsWith(".")) return null;
  const root = resolvePath(rootDir);
  const base = resolvePath(dirname(fromFile), specifier);
  const candidates = [base, ...[".ts", ".tsx", ".js", ".jsx", ".d.ts"].map((extension) => `${base}${extension}`),
    ...[".ts", ".tsx", ".js", ".jsx"].map((extension) => join(base, `index${extension}`))];
  for (const candidate of candidates) {
    const resolved = resolvePath(candidate);
    if (resolved !== root && !resolved.startsWith(`${root}${sep}`)) continue;
    try {
      if (statSync(resolved).isFile()) return resolved;
    } catch {
      // A missing extension candidate is expected; try the next one.
    }
  }
  return null;
}

function buildPropTypeCatalog(rootDir: string, sources: Map<string, string>): PropTypeCatalog {
  const catalog: PropTypeCatalog = new Map();
  const pending = [...sources.entries()];
  const seen = new Set<string>();
  while (pending.length > 0 && seen.size < MAX_IMPORTED_TYPE_FILES) {
    const [file, knownSource] = pending.shift()!;
    if (seen.has(file)) continue;
    seen.add(file);
    let source = knownSource;
    if (!sources.has(file)) {
      try { source = readFileSync(file, "utf8"); } catch { continue; }
    }
    for (const [name, members] of typeDeclarations(source)) catalog.set(`${file}\0${name}`, members);
    for (const raw of parsedBody(source)) {
      const node = raw as Record<string, unknown>;
      if (node.type !== "ImportDeclaration") continue;
      const specifier = (node.source as Record<string, unknown> | undefined)?.value;
      if (typeof specifier !== "string") continue;
      const importedFile = resolveProjectImport(rootDir, file, specifier);
      if (!importedFile || seen.has(importedFile)) continue;
      pending.push([importedFile, ""]);
    }
  }
  return catalog;
}

function importedPropTypes(source: string, file: string, rootDir: string, catalog: PropTypeCatalog): Map<string, PropTypeMembers> {
  const result = new Map<string, PropTypeMembers>();
  for (const raw of parsedBody(source)) {
    const node = raw as Record<string, unknown>;
    if (node.type !== "ImportDeclaration") continue;
    const specifier = (node.source as Record<string, unknown> | undefined)?.value;
    if (typeof specifier !== "string") continue;
    const importedFile = resolveProjectImport(rootDir, file, specifier);
    if (!importedFile) continue;
    for (const rawSpec of (node.specifiers as Record<string, unknown>[]) ?? []) {
      if (rawSpec.type !== "ImportSpecifier") continue;
      const imported = identifierName(rawSpec.imported);
      const local = identifierName(rawSpec.local);
      if (!imported || !local) continue;
      const members = catalog.get(`${importedFile}\0${imported}`);
      if (members) result.set(local, members);
    }
  }
  return result;
}

function propTypeText(source: string, member: Record<string, unknown> | undefined): string {
  const annotation = member?.typeAnnotation as Record<string, unknown> | undefined;
  const memberSource = typeof member?.__source === "string" ? member.__source : source;
  return annotation?.typeAnnotation ? nodeText(memberSource, annotation.typeAnnotation) : "unknown";
}

function parameterPropNames(source: string, parameter: Record<string, unknown>): Map<string, { required: boolean; defaultValue?: string }> {
  const result = new Map<string, { required: boolean; defaultValue?: string }>();
  const pattern = parameter.type === "ObjectPattern" ? parameter : null;
  if (!pattern) {
    // `function Card(props: CardProps)` exposes the named type's fields, not
    // a prop literally called `props`; an untyped identifier has no safe
    // field-level information to offer.
    return result;
  }
  for (const raw of (pattern.properties as Record<string, unknown>[]) ?? []) {
    if (raw.type !== "ObjectProperty") continue;
    const name = identifierName(raw.key);
    if (!name) continue;
    const value = raw.value as Record<string, unknown> | undefined;
    if (value?.type === "AssignmentPattern") {
      result.set(name, { required: false, defaultValue: nodeText(source, value.right) });
    } else {
      result.set(name, { required: true });
    }
  }
  return result;
}

function componentPropHints(source: string, body: unknown[], externalPropTypes?: Map<string, PropTypeMembers>): Map<string, ComponentPropHint[]> {
  const declarations = new Map<string, Record<string, unknown>>();
  const propTypes = new Map<string, Record<string, Record<string, unknown>>>();
  for (const raw of body) {
    const node = raw as Record<string, unknown>;
    const typeDeclaration = node.type === "ExportNamedDeclaration" ? node.declaration as Record<string, unknown> | undefined : node;
    if (typeDeclaration?.type === "TSInterfaceDeclaration" || typeDeclaration?.type === "TSTypeAliasDeclaration") {
      const name = identifierName(typeDeclaration.id);
      if (name) propTypes.set(name, typeMembers(typeDeclaration));
    }
    const declaration = node.type === "ExportNamedDeclaration" || node.type === "ExportDefaultDeclaration"
      ? node.declaration as Record<string, unknown> | undefined
      : node;
    if (!declaration) continue;
    const declarationName = identifierName(declaration.id);
    if (declarationName) declarations.set(declarationName, declaration);
    if (declaration.type === "VariableDeclaration") {
      for (const rawDeclarator of (declaration.declarations as Record<string, unknown>[]) ?? []) {
        const name = identifierName(rawDeclarator.id);
        if (name) declarations.set(name, rawDeclarator.init as Record<string, unknown>);
      }
    }
  }
  const result = new Map<string, ComponentPropHint[]>();
  for (const [name, declaration] of declarations) {
    const params = (declaration.params as Record<string, unknown>[]) ?? [];
    if (params.length === 0) continue;
    const parameter = params[0];
    const parameterAnnotation = parameter.typeAnnotation as Record<string, unknown> | undefined;
    const referencedType = typeName(parameterAnnotation?.typeAnnotation);
    const members = referencedType ? propTypes.get(referencedType) ?? externalPropTypes?.get(referencedType) ?? {} : {};
    const destructured = parameterPropNames(source, parameter);
    const names = new Set([...Object.keys(members), ...destructured.keys()]);
    const hints = [...names].map((propName) => {
      const pattern = destructured.get(propName);
      const member = members[propName];
      return {
        name: propName,
        type: propTypeText(source, member),
        required: pattern ? pattern.required && !member?.optional : !member?.optional,
        ...(pattern?.defaultValue ? { defaultValue: pattern.defaultValue } : {}),
      };
    });
    if (hints.length > 0) result.set(name, hints);
  }
  return result;
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
export function discoverComponentsInSource(source: string, file: string, fromFile?: string, externalPropTypes?: Map<string, PropTypeMembers>): DiscoveredComponent[] {
  let ast: { program?: { body?: unknown[] } };
  try {
    ast = parserAdapter.parse(source) as { program?: { body?: unknown[] } };
  } catch {
    return [];
  }
  const body = ast.program?.body ?? [];
  const sourceTags = collectJsxTagNames(ast);
  const propHints = componentPropHints(source, body, externalPropTypes);
  return exportedComponentNames(body).map(({ name, isDefault, deprecated, replacement }) => ({
    name,
    file,
    isDefault,
    importFrom: fromFile && file === fromFile ? null : fromFile ? relativeSpecifier(fromFile, file) : null,
    usageCount: sourceTags.get(name) ?? 0,
    usageFiles: sourceTags.has(name) ? [file] : [],
    ...(deprecated ? { deprecated: true } : {}),
    ...(replacement ? { replacement } : {}),
    ...(propHints.get(name)?.length ? { propHints: propHints.get(name) } : {}),
  }));
}

function jsxName(node: unknown): string | null {
  if (!node || typeof node !== "object") return null;
  const value = node as Record<string, unknown>;
  if (value.type === "JSXIdentifier" && typeof value.name === "string") return value.name;
  if (value.type === "JSXMemberExpression") {
    const object = jsxName(value.object);
    const property = jsxName(value.property);
    return object && property ? `${object}.${property}` : null;
  }
  return null;
}

/** Counts real JSX opening tags in a parsed source buffer without regex false positives. */
function collectJsxTagNames(ast: unknown): Map<string, number> {
  const result = new Map<string, number>();
  const seen = new Set<object>();
  const visit = (value: unknown): void => {
    if (!value || typeof value !== "object") return;
    const object = value as Record<string, unknown>;
    if (seen.has(object)) return;
    seen.add(object);
    if (object.type === "JSXElement" || object.type === "JSXOpeningElement" || object.type === "JSXClosingElement") {
      const name = jsxName(object.type === "JSXElement" ? (object.openingElement as Record<string, unknown> | undefined)?.name : object.name);
      if (name) result.set(name, (result.get(name) ?? 0) + (object.type === "JSXElement" ? 1 : 0));
    }
    for (const child of Object.values(object)) {
      if (Array.isArray(child)) child.forEach(visit);
      else visit(child);
    }
  };
  visit(ast);
  return result;
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
  const sources = new Map<string, string>();
  for (const file of files) {
    try {
      const source = readFileSync(file, "utf8");
      sources.set(file, source);
    } catch { /* skip unreadable files */ }
  }
  const typeCatalog = buildPropTypeCatalog(rootDir, sources);
  for (const [file, source] of sources) {
    out.push(...discoverComponentsInSource(source, file, fromFile, importedPropTypes(source, file, rootDir, typeCatalog)));
  }
  const usageByFile = new Map<string, Map<string, number>>();
  for (const [file, source] of sources) {
    try {
      usageByFile.set(file, collectJsxTagNames(parserAdapter.parse(source)));
    } catch {
      usageByFile.set(file, new Map());
    }
  }
  return out.map((component) => {
    const usageFiles = [...usageByFile.entries()]
      .filter(([, tags]) => (tags.get(component.name) ?? 0) > 0)
      .map(([file]) => file);
    const usageCount = [...usageByFile.values()]
      .reduce((count, tags) => count + (tags.get(component.name) ?? 0), 0);
    return { ...component, usageCount, usageFiles };
  });
}
