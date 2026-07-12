/**
 * Real §75.93 theme application -- copied verbatim from
 * `desktop/src/applyTheme.ts` (one shared source of truth, matching this
 * project's own existing "copied verbatim from desktop/" convention
 * already used for `syntax.ts`/`theme.css`). Sets a real `data-theme`
 * attribute on the document root; `theme.css`'s own
 * `:root[data-theme="light"]` block is the only thing that reacts to it.
 *
 * Unlike `desktop/`, this app has no `spartan-backend` settings store to
 * read from/write to (§75.89's own named scope: no LSP/DAP/Leo/git
 * connectivity yet) -- `App.tsx` persists the chosen value to
 * `localStorage` directly instead, the same real, deliberate pattern
 * `desktop/`'s own Leo voice-output toggle already established (§75.71)
 * for a pure renderer preference with no backend of its own.
 */
export type ThemeName = "SpartanDark" | "SpartanLight";

export function applyTheme(theme: ThemeName): void {
  document.documentElement.setAttribute(
    "data-theme",
    theme === "SpartanLight" ? "light" : "dark"
  );
}
