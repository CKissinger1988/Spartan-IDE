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

/** Finds declarations such as `--color-accent: #e33;` without executing CSS. */
export function discoverTokens(rootDir: string): DiscoveredToken[] {
  const files: string[] = [];
  collectFiles(rootDir, 0, files);
  files.sort();
  const result: DiscoveredToken[] = [];
  const declaration = /(--[a-zA-Z0-9_-]+)\s*:\s*([^;{}]+)\s*;?/g;
  for (const file of files) {
    let source: string;
    try { source = readFileSync(file, "utf8"); } catch { continue; }
    for (const match of source.matchAll(declaration)) {
      const name = match[1];
      const value = match[2].trim();
      if (!value) continue;
      result.push({ name, value, file, relativePath: posixPath(relative(rootDir, file)) });
    }
  }
  return result;
}
