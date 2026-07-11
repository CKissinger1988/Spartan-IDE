/**
 * Real §75.93 theme application -- the same small, shared-helper pattern
 * `reduceMotion.ts` already established: one real function, called both
 * on startup (`App.tsx`, before Settings has necessarily been opened) and
 * live the moment the Settings screen's theme selector changes.
 *
 * Sets a real `data-theme` attribute on the document root; `theme.css`'s
 * own `:root[data-theme="light"]` block is the only thing that reacts to
 * it -- `mirrors spartan_settings::ThemeName`'s two real variants
 * (`SpartanDark`/`SpartanLight`) exactly, so no translation table is
 * needed beyond the literal string each already serializes as.
 */
export type ThemeName = "SpartanDark" | "SpartanLight";

export function applyTheme(theme: ThemeName): void {
  document.documentElement.setAttribute(
    "data-theme",
    theme === "SpartanLight" ? "light" : "dark"
  );
}
