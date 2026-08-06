import { strict as assert } from "node:assert";
import { test } from "node:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { discoverAssets, fontFaceSnippet, fontMetadata, sanitizeSvgMarkup } from "./assets.js";

function fixture(files: string[]): string {
  const root = mkdtempSync(join(tmpdir(), "spartan-assets-"));
  for (const rel of files) {
    const full = join(root, rel);
    mkdirSync(join(full, ".."), { recursive: true });
    writeFileSync(full, "fixture");
  }
  return root;
}

test("discovers supported image assets with project-relative paths", () => {
  const root = fixture(["public/logo.svg", "src/images/hero.webp", "notes.txt"]);
  const assets = discoverAssets(root);
  assert.deepEqual(assets.map((asset) => asset.relativePath), ["public/logo.svg", "src/images/hero.webp"]);
  assert.equal(assets[0].kind, "image");
  assert.equal(assets[0].label, "logo.svg");
});

test("computes a real JSX reference path relative to the open component", () => {
  const root = fixture(["src/pages/Home.tsx", "src/assets/hero.png"]);
  const fromFile = join(root, "src/pages/Home.tsx");
  const asset = discoverAssets(root, fromFile)[0];
  assert.equal(asset.referencePath, "../assets/hero.png");
});

test("discovers font assets with a real relative reference path", () => {
  const root = fixture(["src/pages/Home.tsx", "src/fonts/Inter.woff2", "src/fonts/Inter.ttf"]);
  const fromFile = join(root, "src/pages/Home.tsx");
  const fonts = discoverAssets(root, fromFile).filter((asset) => asset.kind === "font");
  assert.deepEqual(fonts.map((font) => font.relativePath), ["src/fonts/Inter.ttf", "src/fonts/Inter.woff2"]);
  assert.equal(fonts[0].referencePath, "../fonts/Inter.ttf");
});

test("generates a format-aware @font-face snippet for a discovered font", () => {
  const root = fixture(["src/pages/Home.tsx", "src/fonts/Inter.woff2"]);
  const font = discoverAssets(root, join(root, "src/pages/Home.tsx")).find((asset) => asset.kind === "font");
  assert.ok(font);
  assert.equal(fontFaceSnippet(font), `@font-face {\n  font-family: "Inter";\n  src: url("../fonts/Inter.woff2") format("woff2");\n  font-style: normal;\n  font-weight: 400;\n}`);
  assert.equal(font.fontFaceSnippet, fontFaceSnippet(font));
});

test("infers conventional font weight and italic metadata", () => {
  assert.deepEqual(fontMetadata("Inter-SemiBoldItalic.woff2"), { fontWeight: 600, fontStyle: "italic" });
  assert.deepEqual(fontMetadata("Brand-700.woff2"), { fontWeight: 700, fontStyle: "normal" });
  assert.deepEqual(fontMetadata("Display-Variable.woff2"), { fontWeight: "100 900", fontStyle: "normal" });
});

test("generates metadata-aware @font-face declarations", () => {
  const root = fixture(["fonts/Inter-BoldItalic.woff2"]);
  const font = discoverAssets(root).find((asset) => asset.kind === "font");
  assert.ok(font);
  assert.equal(font.fontWeight, 700);
  assert.equal(font.fontStyle, "italic");
  assert.match(font.fontFaceSnippet ?? "", /font-style: italic;/);
  assert.match(font.fontFaceSnippet ?? "", /font-weight: 700;/);
});

test("derives a usable CSS family name for a discovered font", () => {
  const assets = discoverAssets(fixture(["fonts/Brand Sans.woff2"]));
  const font = assets.find((asset) => asset.kind === "font");
  assert.ok(font);
  assert.equal(font.fontFamily, "Brand Sans");
});

test("never walks dependency or build output", () => {
  const root = fixture(["src/App.tsx", "node_modules/pkg/logo.png", "dist/generated.png", "public/ok.png"]);
  assert.deepEqual(discoverAssets(root).map((asset) => asset.relativePath), ["public/ok.png"]);
});

test("indexes direct image and font references while skipping dependencies and builds", () => {
  const root = fixture([
    "src/App.tsx",
    "src/assets/logo.svg",
    "src/fonts/Inter.woff2",
    "styles.css",
    "node_modules/pkg/ignored.svg",
    "dist/ignored.woff2",
  ]);
  writeFileSync(join(root, "src/App.tsx"), "import logo from './assets/logo.svg';\nconst font = './fonts/Inter.woff2';\nexport const view = <img src={logo} />;\n");
  writeFileSync(join(root, "styles.css"), "@font-face { src: url('./src/fonts/Inter.woff2'); } .logo { background: url('./src/assets/logo.svg'); }");
  const assets = discoverAssets(root);
  const logo = assets.find((asset) => asset.relativePath === "src/assets/logo.svg");
  const font = assets.find((asset) => asset.relativePath === "src/fonts/Inter.woff2");
  assert.ok(logo);
  assert.ok(font);
  assert.equal(logo.usageCount, 2);
  assert.deepEqual(logo.usageFiles, [join(root, "src/App.tsx"), join(root, "styles.css")]);
  assert.deepEqual(logo.usageLocations?.map(({ file, line }) => ({ file, line })), [
    { file: join(root, "src/App.tsx"), line: 1 },
    { file: join(root, "styles.css"), line: 1 },
  ]);
  assert.equal(font.usageCount, 2);
  assert.deepEqual(font.usageFiles, [join(root, "src/App.tsx"), join(root, "styles.css")]);
  assert.deepEqual(font.usageLocations?.map(({ file, line }) => ({ file, line })), [
    { file: join(root, "src/App.tsx"), line: 2 },
    { file: join(root, "styles.css"), line: 1 },
  ]);
});

test("uses unsaved source overrides when indexing asset references", () => {
  const root = fixture(["src/App.tsx", "src/assets/logo.svg", "styles.css"]);
  const app = join(root, "src/App.tsx");
  writeFileSync(app, "import logo from './assets/logo.svg';\nexport const view = <img src={logo} />;\n");
  writeFileSync(join(root, "styles.css"), ".logo { background: url('./src/assets/logo.svg'); }");
  const assets = discoverAssets(root, undefined, { [app]: "export const view = <div />;\n" });
  const logo = assets.find((asset) => asset.relativePath === "src/assets/logo.svg");
  assert.ok(logo);
  assert.equal(logo.usageCount, 1);
  assert.deepEqual(logo.usageFiles, [join(root, "styles.css")]);
});

test("sanitizes executable and event-handler content from reusable SVG markup", () => {
  const source = `<svg viewBox="0 0 10 10" onclick="alert(1)"><script>alert(2)</script><a href="javascript:evil()"><path d="M0 0" /></a></svg>`;
  const safe = sanitizeSvgMarkup(source);
  assert.match(safe, /^<svg/);
  assert.doesNotMatch(safe, /script|onclick|javascript:/i);
  assert.match(safe, /<path d="M0 0" \/>/);
});

test("rejects incomplete or non-SVG markup", () => {
  assert.throws(() => sanitizeSvgMarkup("<div>nope</div>"), /complete root SVG/);
  assert.throws(() => sanitizeSvgMarkup("<svg><path /></div>"), /complete root SVG/);
});
