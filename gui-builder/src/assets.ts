/** Real image/font asset discovery for the GUI Builder palette.
 *
 * This is intentionally a filesystem/metadata operation only: image files
 * are never read into the renderer and no project code is executed. The
 * returned `referencePath` is the relative path an import-aware bundler can
 * use from the currently open component file.
 */
import { readdirSync, statSync } from "node:fs";
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

function posixPath(value: string): string {
  return value.split(sep).join("/");
}

function relativeReference(fromFile: string | undefined, file: string): string {
  if (!fromFile) return `./${posixPath(file)}`;
  let result = posixPath(relative(dirname(fromFile), file));
  if (!result.startsWith(".")) result = `./${result}`;
  return result;
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
  return files.map((file) => {
    const kind = IMAGE_EXTENSIONS.has(extname(file).toLowerCase()) ? "image" as const : "font" as const;
    const asset: DiscoveredAsset = {
    file,
    relativePath: posixPath(relative(rootDir, file)),
    referencePath: relativeReference(fromFile, file),
    kind,
    label: file.slice(file.lastIndexOf(sep) + 1),
    };
    if (kind === "font") {
      asset.fontFamily = fontFamilyName(asset.label);
      asset.fontFaceSnippet = fontFaceSnippet(asset);
    }
    return asset;
  });
}
