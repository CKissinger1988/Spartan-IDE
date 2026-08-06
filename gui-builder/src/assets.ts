/** Real image/font asset discovery for the GUI Builder palette.
 *
 * This is intentionally a filesystem/metadata operation only: image files
 * are never read into the renderer and no project code is executed. The
 * returned `referencePath` is the relative path an import-aware bundler can
 * use from the currently open component file.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, extname, join, relative, sep } from "node:path";

export interface DiscoveredAsset {
  /** Absolute path of the asset on disk. */
  file: string;
  /** POSIX path relative to the scanned project root. */
  relativePath: string;
  /** POSIX path usable as a JSX `src` from `fromFile`. */
  referencePath: string;
  kind: "image" | "font";
  label: string;
  /** CSS family name derived from the font filename; omitted for images. */
  fontFamily?: string;
  /** Ready-to-copy CSS for font assets; omitted for images. */
  fontFaceSnippet?: string;
  /** Number of direct source references found across the project. */
  usageCount?: number;
  /** Absolute source files containing direct references to this asset. */
  usageFiles?: string[];
}

/** Returns reusable SVG markup after removing executable or event-handler
 * content. The Design canvas already runs inside a sandbox, but copied
 * markup should remain safe when a user pastes it into a normal component. */
export function sanitizeSvgMarkup(source: string): string {
  const withoutScripts = source.replace(/<script\b[^>]*>[\s\S]*?<\/script\s*>/gi, "");
  const withoutHandlers = withoutScripts
    .replace(/\s+on[a-z]+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)/gi, "")
    .replace(/\s+(?:href|xlink:href)\s*=\s*(?:"\s*javascript:[^"]*"|'\s*javascript:[^']*'|\s*javascript:[^\s>]+)/gi, "");
  const markup = withoutHandlers.trim();
  if (!/^<svg\b[\s>]/i.test(markup) || !/<\/svg\s*>$/i.test(markup)) {
    throw new Error("The selected file does not contain a complete root SVG element.");
  }
  return markup;
}

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

const IMAGE_EXTENSIONS = new Set([".avif", ".gif", ".jpeg", ".jpg", ".png", ".svg", ".webp"]);
const FONT_EXTENSIONS = new Set([".eot", ".otf", ".ttf", ".woff", ".woff2"]);
const SOURCE_EXTENSIONS = new Set([".css", ".scss", ".sass", ".less", ".js", ".jsx", ".ts", ".tsx"]);
const MAX_DEPTH = 12;

function collectFiles(dir: string, depth: number, out: string[]): void {
  if (depth > MAX_DEPTH) return;
  let entries: string[];
  try {
    entries = readdirSync(dir);
  } catch {
    return;
  }
  for (const entry of entries) {
    if (SKIP_DIRS.has(entry)) continue;
    const full = join(dir, entry);
    let stats;
    try {
      stats = statSync(full);
    } catch {
      continue;
    }
    if (stats.isDirectory()) collectFiles(full, depth + 1, out);
    else if (IMAGE_EXTENSIONS.has(extname(entry).toLowerCase()) || FONT_EXTENSIONS.has(extname(entry).toLowerCase())) out.push(full);
  }
}

function collectSourceFiles(dir: string, depth: number, out: string[]): void {
  if (depth > MAX_DEPTH) return;
  let entries: string[];
  try { entries = readdirSync(dir); } catch { return; }
  for (const entry of entries) {
    if (SKIP_DIRS.has(entry)) continue;
    const full = join(dir, entry);
    let stats;
    try { stats = statSync(full); } catch { continue; }
    if (stats.isDirectory()) collectSourceFiles(full, depth + 1, out);
    else if (SOURCE_EXTENSIONS.has(extname(entry).toLowerCase())) out.push(full);
  }
}

function posixPath(value: string): string {
  return value.split(sep).join("/");
}

function relativeReference(fromFile: string | undefined, file: string): string {
  if (!fromFile) return `./${posixPath(file)}`;
  let result = posixPath(relative(dirname(fromFile), file));
  if (!result.startsWith(".")) result = `./${result}`;
  return result;
}

function referenceCandidates(rootDir: string, sourceFile: string, assetFile: string): string[] {
  const candidates = new Set<string>();
  const projectPath = posixPath(relative(rootDir, assetFile));
  const sourceRelativePath = posixPath(relative(dirname(sourceFile), assetFile));
  candidates.add(projectPath);
  candidates.add(`./${projectPath}`);
  candidates.add(sourceRelativePath.startsWith(".") ? sourceRelativePath : `./${sourceRelativePath}`);
  if (projectPath.startsWith("public/")) candidates.add(`/${projectPath.slice("public/".length)}`);
  return [...candidates].filter((candidate) => candidate.length > 0);
}

function countLiteralReferences(source: string, candidates: string[]): number {
  const matches: Array<{ start: number; end: number }> = [];
  for (const candidate of candidates.sort((left, right) => right.length - left.length)) {
    let offset = 0;
    while (true) {
      const index = source.indexOf(candidate, offset);
      if (index < 0) break;
      matches.push({ start: index, end: index + candidate.length });
      offset = index + candidate.length;
    }
  }
  matches.sort((left, right) => left.start - right.start || right.end - left.end);
  const occupied: Array<{ start: number; end: number }> = [];
  for (const match of matches) {
    if (occupied.some((existing) => match.start < existing.end && match.end > existing.start)) continue;
    occupied.push(match);
  }
  return occupied.length;
}

function collectAssetUsages(rootDir: string, assetFiles: string[]): Map<string, { count: number; files: string[] }> {
  const sourceFiles: string[] = [];
  collectSourceFiles(rootDir, 0, sourceFiles);
  sourceFiles.sort();
  const usages = new Map<string, { count: number; files: string[] }>();
  for (const sourceFile of sourceFiles) {
    let source: string;
    try { source = readFileSync(sourceFile, "utf8"); } catch { continue; }
    for (const assetFile of assetFiles) {
      const count = countLiteralReferences(source, referenceCandidates(rootDir, sourceFile, assetFile));
      if (count === 0) continue;
      const usage = usages.get(assetFile) ?? { count: 0, files: [] };
      usage.count += count;
      usage.files.push(sourceFile);
      usages.set(assetFile, usage);
    }
  }
  return usages;
}

function fontFormat(file: string): string {
  switch (extname(file).toLowerCase()) {
    case ".eot": return "embedded-opentype";
    case ".otf": return "opentype";
    case ".ttf": return "truetype";
    case ".woff": return "woff";
    case ".woff2": return "woff2";
    default: return "truetype";
  }
}

export function fontFamilyName(label: string): string {
  return label.replace(/\.[^.]+$/, "").replace(/["\\]/g, "");
}

/** Builds a format-aware, ready-to-paste @font-face declaration without
 * reading or executing the font file. */
export function fontFaceSnippet(asset: Pick<DiscoveredAsset, "kind" | "label" | "referencePath" | "fontFamily">): string | undefined {
  if (asset.kind !== "font") return undefined;
  const family = asset.fontFamily ?? fontFamilyName(asset.label);
  return `@font-face {\n  font-family: "${family}";\n  src: url("${asset.referencePath.replace(/["\\]/g, "\\$&")}") format("${fontFormat(asset.label)}");\n  font-style: normal;\n  font-weight: 400;\n}`;
}

/** Discovers image and font assets under `rootDir`, ignoring dependency/build trees. */
export function discoverAssets(rootDir: string, fromFile?: string): DiscoveredAsset[] {
  const files: string[] = [];
  collectFiles(rootDir, 0, files);
  files.sort();
  const usages = collectAssetUsages(rootDir, files);
  return files.map((file) => {
    const kind = IMAGE_EXTENSIONS.has(extname(file).toLowerCase()) ? "image" as const : "font" as const;
    const asset: DiscoveredAsset = {
      file,
      relativePath: posixPath(relative(rootDir, file)),
      referencePath: relativeReference(fromFile, file),
      kind,
      label: file.slice(file.lastIndexOf(sep) + 1),
      usageCount: usages.get(file)?.count ?? 0,
      usageFiles: usages.get(file)?.files ?? [],
    };
    if (kind === "font") {
      asset.fontFamily = fontFamilyName(asset.label);
      asset.fontFaceSnippet = fontFaceSnippet(asset);
    }
    return asset;
  });
}
