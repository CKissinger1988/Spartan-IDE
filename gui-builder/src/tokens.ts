/** Real CSS custom-property discovery for the GUI Builder token palette. */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join, relative, sep } from "node:path";

export interface DiscoveredToken {
  name: string;
  value: string;
  file: string;
  relativePath: string;
}

const SKIP_DIRS = new Set(["node_modules", ".git", "dist", "build", ".next", "coverage", "out", ".cache"]);
const CSS_EXTENSIONS = new Set([".css", ".scss", ".sass", ".less"]);
const MAX_DEPTH = 12;

function collectFiles(dir: string, depth: number, out: string[]): void {
  if (depth > MAX_DEPTH) return;
  let entries: string[];
  try { entries = readdirSync(dir); } catch { return; }
  for (const entry of entries) {
    if (SKIP_DIRS.has(entry)) continue;
    const full = join(dir, entry);
    let stats;
    try { stats = statSync(full); } catch { continue; }
    if (stats.isDirectory()) collectFiles(full, depth + 1, out);
    else if (CSS_EXTENSIONS.has(extname(entry).toLowerCase())) out.push(full);
  }
}

function posixPath(value: string): string { return value.split(sep).join("/"); }

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function validateTokenName(name: string): void {
  if (!/^--[a-zA-Z0-9_-]+$/.test(name)) throw new Error(`Invalid CSS custom-property name "${name}".`);
}

function validateToken(name: string, value: string): string {
  validateTokenName(name);
  const trimmed = value.trim();
  if (!trimmed || /[;{}]/.test(trimmed)) {
    throw new Error("Token value must be non-empty and cannot contain ';', '{', or '}'.");
  }
  return trimmed;
}

/** Replaces one real custom-property declaration while preserving the rest
 * of the CSS source exactly. Values cannot contain declaration delimiters in
 * this v1, preventing an edit from injecting a second property or rule. */
export function applyTokenValue(source: string, name: string, value: string): string {
  const trimmed = validateToken(name, value);
  const declaration = new RegExp(`(^|[;{\\s])(${escapeRegExp(name)})\\s*:\\s*([^;{}]+)`, "m");
  if (!declaration.test(source)) throw new Error(`No declaration for token "${name}" was found.`);
  return source.replace(declaration, (_match, prefix: string, tokenName: string) => `${prefix}${tokenName}: ${trimmed}`);
}

/** Removes one custom-property declaration while leaving surrounding CSS
 * source and neighboring declarations intact. */
export function removeTokenValue(source: string, name: string): string {
  validateTokenName(name);
  const declaration = new RegExp(`(^|[;{\\s])${escapeRegExp(name)}\\s*:\\s*[^;{}]+;?`, "m");
  if (!declaration.test(source)) throw new Error(`No declaration for token "${name}" was found.`);
  return source.replace(declaration, (_match, prefix: string) => prefix);
}

/** Creates a custom-property declaration in `:root`, or updates it when it
 * already exists. The edit is deliberately limited to one declaration value
 * and preserves the surrounding stylesheet source. */
export function defineTokenValue(source: string, name: string, value: string): string {
  const trimmed = validateToken(name, value);
  const declaration = new RegExp(`(^|[;{\\s])(${escapeRegExp(name)})\\s*:\\s*([^;{}]+)`, "m");
  if (declaration.test(source)) return applyTokenValue(source, name, trimmed);

  const rootBlock = /(:root\s*\{)([\s\S]*?)(\})/m;
  const match = rootBlock.exec(source);
  if (!match) return `:root {\n  ${name}: ${trimmed};\n}\n${source}`;

  const body = match[2];
  if (!body.includes("\n")) {
    const inlineBody = body.trim();
    const separator = inlineBody ? " " : "";
    const updated = `${match[1]}${body}${separator}${name}: ${trimmed};${inlineBody ? " " : ""}${match[3]}`;
    return source.slice(0, match.index) + updated + source.slice(match.index + match[0].length);
  }

  const closingIndent = body.match(/\n([ \t]*)$/)?.[1] ?? "";
  const content = body.endsWith("\n") ? body : `${body}\n`;
  const updated = `${match[1]}${content}  ${name}: ${trimmed};\n${closingIndent}${match[3]}`;
  return source.slice(0, match.index) + updated + source.slice(match.index + match[0].length);
}

/** Finds declarations in one already-loaded stylesheet without executing CSS. */
export function discoverTokensInSource(source: string, file: string, rootDir: string): DiscoveredToken[] {
  const result: DiscoveredToken[] = [];
  const declaration = /(--[a-zA-Z0-9_-]+)\s*:\s*([^;{}]+)\s*;?/g;
  for (const match of source.matchAll(declaration)) {
    const name = match[1];
    const value = match[2].trim();
    if (!value) continue;
    result.push({ name, value, file, relativePath: posixPath(relative(rootDir, file)) });
  }
  return result;
}

/** Finds declarations such as `--color-accent: #e33;` without executing CSS. */
export function discoverTokens(rootDir: string): DiscoveredToken[] {
  const files: string[] = [];
  collectFiles(rootDir, 0, files);
  files.sort();
  const result: DiscoveredToken[] = [];
  for (const file of files) {
    let source: string;
    try { source = readFileSync(file, "utf8"); } catch { continue; }
    result.push(...discoverTokensInSource(source, file, rootDir));
  }
  return result;
}
