/** Builds a standalone, runnable HTML document for a bundled GUI Builder
 * preview. The bundle is authored by esbuild, but it can still contain the
 * literal `</script` sequence inside a string; escape that closing marker so
 * the browser cannot terminate the bootstrap script early. */
export function buildPreviewDocument(bundleCode: string, title = "Spartan GUI Builder preview"): string {
  const safeBundle = bundleCode.replace(/<\/script/gi, "<\\/script");
  const safeTitle = title.replace(/[&<>\"]/g, (character) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
  }[character] ?? character));
  return `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>${safeTitle}</title><style>body{margin:0;background:#fff;color:#111;font-family:sans-serif;}</style></head><body><div id="spartan-root"></div><script>${safeBundle}</script></body></html>`;
}
