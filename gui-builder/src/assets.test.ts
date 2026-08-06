import { strict as assert } from "node:assert";
import { test } from "node:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { discoverAssets } from "./assets.js";

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

test("never walks dependency or build output", () => {
  const root = fixture(["src/App.tsx", "node_modules/pkg/logo.png", "dist/generated.png", "public/ok.png"]);
  assert.deepEqual(discoverAssets(root).map((asset) => asset.relativePath), ["public/ok.png"]);
});

