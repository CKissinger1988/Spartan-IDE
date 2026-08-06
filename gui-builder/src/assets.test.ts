import { strict as assert } from "node:assert";
import { test } from "node:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { discoverAssets, fontFaceSnippet } from "./assets.js";

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
