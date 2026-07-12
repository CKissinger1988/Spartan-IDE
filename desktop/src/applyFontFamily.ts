/**
 * Real §75.93 font-family application -- the same small, shared-helper
 * pattern `reduceMotion.ts`/`applyTheme.ts` already established. Sets (or
 * clears) the real `--font-mono` CSS custom property `theme.css`'s own
 * `.mono` rule reads, so a chosen font applies to every real `.mono`
 * surface app-wide (editor, gutter, tab bar, sidebar, Leo chat, terminal,
 * status bar, Settings) at once, live -- not just the code editor.
 *
 * Mirrors `spartan_settings::EditorSettings.font_family: Option<String>`
 * exactly: `null`/empty means "use the real bundled default" (removes
 * the override rather than reapplying the default's own literal string,
 * so `theme.css`'s own default stays the single source of truth for what
 * that fallback chain actually is); a real name is prepended onto the
 * same real fallback chain rather than replacing it outright, so a
 * font that fails to load/resolve on this platform still degrades to a
 * real monospace font instead of the browser's arbitrary serif default.
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
