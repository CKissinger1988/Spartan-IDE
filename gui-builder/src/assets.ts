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
  /** CSS font weight inferred from a conventional font filename. */
  fontWeight?: number | string;
  /** CSS font style inferred from a conventional font filename. */
  fontStyle?: "normal" | "italic";
  /** Ready-to-copy CSS for font assets; omitted for images. */
  fontFaceSnippet?: string;
  /** Number of direct source references found across the project. */
  usageCount?: number;
  /** Absolute source files containing direct references to this asset. */
  usageFiles?: string[];
  /** Exact one-based line and zero-based column for each direct reference. */
  usageLocations?: Array<{ file: string; line: number; column: number }>;
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

function literalReferenceOffsets(source: string, candidates: string[]): number[] {
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
  return occupied.map((match) => match.start);
}

function sourcePosition(source: string, offset: number): { line: number; column: number } {
  const before = source.slice(0, offset);
  const lineBreak = before.lastIndexOf("\n");
  return {
    line: before.split("\n").length,
    column: offset - lineBreak - 1,
  };
}

function isProjectSource(rootDir: string, file: string): boolean {
  const normalizedRoot = rootDir.replace(/[\\/]$/, "").replace(/\\/g, "/");
  const normalizedFile = file.replace(/\\/g, "/");
  return normalizedFile.startsWith(`${normalizedRoot}/`) || normalizedFile === normalizedRoot;
}

function readSource(file: string, sourceOverrides: Record<string, string>): string | null {
  if (Object.prototype.hasOwnProperty.call(sourceOverrides, file)) return sourceOverrides[file];
  try { return readFileSync(file, "utf8"); } catch { return null; }
}

function collectAssetUsages(rootDir: string, assetFiles: string[], sourceOverrides: Record<string, string>): Map<string, { count: number; files: string[]; locations: Array<{ file: string; line: number; column: number }> }> {
  const sourceFiles: string[] = [];
  collectSourceFiles(rootDir, 0, sourceFiles);
  for (const file of Object.keys(sourceOverrides)) {
    if (isProjectSource(rootDir, file) && SOURCE_EXTENSIONS.has(extname(file).toLowerCase())) sourceFiles.push(file);
  }
  const uniqueSourceFiles = [...new Set(sourceFiles)];
  uniqueSourceFiles.sort();
  const usages = new Map<string, { count: number; files: string[]; locations: Array<{ file: string; line: number; column: number }> }>();
  for (const sourceFile of uniqueSourceFiles) {
    const source = readSource(sourceFile, sourceOverrides);
    if (source === null) continue;
    for (const assetFile of assetFiles) {
      const offsets = literalReferenceOffsets(source, referenceCandidates(rootDir, sourceFile, assetFile));
      if (offsets.length === 0) continue;
      const usage = usages.get(assetFile) ?? { count: 0, files: [], locations: [] };
      usage.count += offsets.length;
      usage.files.push(sourceFile);
      usage.locations.push(...offsets.map((offset) => ({ file: sourceFile, ...sourcePosition(source, offset) })));
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

const FONT_WEIGHT_NAMES: Array<[RegExp, number]> = [
  [/\b(?:thin|hairline)\b/, 100],
  [/\b(?:extra|ultra)[ -]?light\b/, 200],
  [/\blight\b/, 300],
  [/\b(?:regular|book|normal)\b/, 400],
  [/\bmedium\b/, 500],
  [/\b(?:semi|demi)[ -]?bold\b/, 600],
  [/\b(?:extra|ultra)[ -]?bold\b/, 800],
  [/\bbold\b/, 700],
  [/\b(?:black|heavy)\b/, 900],
];

function normalizedFontLabel(label: string): string {
  return label
    .replace(/\.[^.]+$/, "")
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .replace(/([a-z])([0-9])/gi, "$1 $2")
    .replace(/[^a-zA-Z0-9]+/g, " ")
    .toLowerCase()
    .trim();
}

/** Infers useful CSS metadata from common font naming conventions without
 * opening or parsing the binary font file. */
export function fontMetadata(label: string): { fontWeight: number | string; fontStyle: "normal" | "italic" } {
  const normalized = normalizedFontLabel(label);
  const numericWeight = normalized.match(/\b([1-9]00)\b/);
  const fontWeight = numericWeight
    ? Number(numericWeight[1])
    : normalized.includes("variable") || normalized.includes("var")
      ? "100 900"
      : FONT_WEIGHT_NAMES.find(([pattern]) => pattern.test(normalized))?.[1] ?? 400;
  return { fontWeight, fontStyle: /\b(?:italic|oblique)\b/.test(normalized) ? "italic" : "normal" };
}

/** Builds a format-aware, ready-to-paste @font-face declaration without
 * reading or executing the font file. */
export function fontFaceSnippet(asset: Pick<DiscoveredAsset, "kind" | "label" | "referencePath" | "fontFamily" | "fontWeight" | "fontStyle">): string | undefined {
  if (asset.kind !== "font") return undefined;
  const family = asset.fontFamily ?? fontFamilyName(asset.label);
  const metadata = fontMetadata(asset.label);
  const fontWeight = asset.fontWeight ?? metadata.fontWeight;
  const fontStyle = asset.fontStyle ?? metadata.fontStyle;
  return `@font-face {\n  font-family: "${family}";\n  src: url("${asset.referencePath.replace(/["\\]/g, "\\$&")}") format("${fontFormat(asset.label)}");\n  font-style: ${fontStyle};\n  font-weight: ${fontWeight};\n}`;
}

/** Discovers image and font assets under `rootDir`, ignoring dependency/build trees. */
export function discoverAssets(rootDir: string, fromFile?: string, sourceOverrides: Record<string, string> = {}): DiscoveredAsset[] {
  const files: string[] = [];
  collectFiles(rootDir, 0, files);
  files.sort();
  const usages = collectAssetUsages(rootDir, files, sourceOverrides);
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
      usageLocations: usages.get(file)?.locations ?? [],
    };
    if (kind === "font") {
      asset.fontFamily = fontFamilyName(asset.label);
      Object.assign(asset, fontMetadata(asset.label));
      asset.fontFaceSnippet = fontFaceSnippet(asset);
    }
    return asset;
  });
}
