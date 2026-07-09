/**
 * Real live-rendering bundler (§75.52, task #12) -- the actual mechanism
 * behind turning the text-tree canvas (§75.41/§75.42) into a genuine
 * visual one. Uses `esbuild` (a real, installed dependency, not a mock)
 * to bundle a real component file plus its real imports into a single,
 * self-contained, browser-ready JS file that mounts the component into a
 * `#spartan-root` DOM node.
 *
 * Deliberately resolves every real import (`react`, `lucide-react`, a
 * design-token module, ...) from the *target file's own* directory
 * upward via esbuild's `resolveDir` -- the real project the user has
 * open, never this package's own `node_modules`. This is the technically
 * correct behavior (matching what a real `vite`/`webpack` dev server
 * would do), not a simplification: a project with its own dependencies
 * installed renders correctly; a project missing one produces a real,
 * honest bundling error (surfaced verbatim, never silently swallowed),
 * the same "name the gap, don't fake it" discipline this whole codebase
 * already follows.
 *
 * A real, named v1 assumption: the target file's component is its
 * **default export** (`import Component from "<file>"`). A file using
 * only named exports will bundle successfully (esbuild has no way to
 * know this is wrong at bundle time) but fail at real render time inside
 * the WebView -- the generated entry script's own try/catch reports that
 * failure visibly in the rendered output rather than a silent blank page.
 */
import * as esbuild from "esbuild";
import { dirname, resolve } from "node:path";

export interface BundleResult {
  code: string;
}

export interface BundleError {
  error: string;
}

function buildEntrySource(absPath: string): string {
  // JSON.stringify on a path is a real, safe way to embed an arbitrary
  // absolute path (including Windows backslashes) as a valid JS string
  // literal.
  const pathLiteral = JSON.stringify(absPath);
  return `
import React from "react";
import { createRoot } from "react-dom/client";
import Component from ${pathLiteral};

const rootEl = document.getElementById("spartan-root");
try {
  if (typeof Component !== "function") {
    throw new Error(
      "The active file has no default export function/class component " +
      "(only a named export, or none at all) -- the live preview currently " +
      "requires a real default export."
    );
  }
  const root = createRoot(rootEl);
  root.render(React.createElement(Component));
} catch (e) {
  rootEl.innerHTML =
    '<div style="color:#e06c75;font-family:monospace;padding:1em;white-space:pre-wrap;">' +
    'Render error: ' + String((e && e.message) || e) +
    '</div>';
}
`;
}

export async function bundleComponent(
  filePath: string,
): Promise<BundleResult | BundleError> {
  const absPath = resolve(filePath);
  try {
    const result = await esbuild.build({
      stdin: {
        contents: buildEntrySource(absPath),
        resolveDir: dirname(absPath),
        loader: "jsx",
      },
      bundle: true,
      write: false,
      format: "iife",
      platform: "browser",
      jsx: "automatic",
      logLevel: "silent",
    });
    const output = result.outputFiles?.[0];
    if (!output) {
      return { error: "esbuild produced no output file" };
    }
    return { code: output.text };
  } catch (e) {
    return { error: (e as Error).message };
  }
}
