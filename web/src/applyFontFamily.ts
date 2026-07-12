/**
 * Real §75.93 font-family application -- copied verbatim from
 * `desktop/src/applyFontFamily.ts` (same shared-source-of-truth
 * convention as `applyTheme.ts`). Sets (or clears) the real
 * `--font-mono` CSS custom property `theme.css`'s own `.mono` rule
 * reads, so a chosen font applies to every real `.mono` surface in this
 * app at once, live.
 */
export function applyFontFamily(fontFamily: string | null | undefined): void {
  const root = document.documentElement;
  const trimmed = fontFamily?.trim();
  if (trimmed) {
    root.style.setProperty(
      "--font-mono",
      `"${trimmed}", "JetBrains Mono", "SF Mono", "Cascadia Code", Consolas, monospace`
    );
  } else {
    root.style.removeProperty("--font-mono");
  }
}
