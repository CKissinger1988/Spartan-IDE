/**
 * Real §75.93 theme application, extended from 2 to 7 real variants by the
 * "make all GUI designs user changeable" pass -- the same small, shared-
 * helper pattern `reduceMotion.ts` already established: one real function,
 * called both on startup (`App.tsx`, before Settings has necessarily been
 * opened) and live the moment the Settings screen's theme selector changes.
 *
 * Sets a real `data-theme` attribute on the document root; `theme.css`'s
 * own `:root[data-theme="..."]` blocks are what react to it -- mirrors
 * `spartan_settings::ThemeName`'s 7 real variants exactly via
 * `THEME_DATA_ATTR`, so there is exactly one place either side of this
 * boundary needs to change when a theme is added or renamed.
 */
export type ThemeName =
  | "SpartanDark"
  | "SpartanLight"
  | "MinimalistZen"
  | "NeonAftergrid"
  | "WarmPaper"
  | "CommandDeck"
  | "GlassNative";

const THEME_DATA_ATTR: Record<ThemeName, string> = {
  SpartanDark: "dark",
  SpartanLight: "light",
  MinimalistZen: "minimalist-zen",
  NeonAftergrid: "neon-aftergrid",
  WarmPaper: "warm-paper",
  CommandDeck: "command-deck",
  GlassNative: "glass-native",
};

/** Human-readable labels for the Settings screen's theme `<select>`. */
export const THEME_LABELS: Record<ThemeName, string> = {
  SpartanDark: "Spartan Dark",
  SpartanLight: "Spartan Light",
  MinimalistZen: "Minimalist Zen",
  NeonAftergrid: "Neon Aftergrid",
  WarmPaper: "Warm Paper",
  CommandDeck: "Command Deck",
  GlassNative: "Glass Native",
};

export function applyTheme(theme: ThemeName): void {
  document.documentElement.setAttribute(
    "data-theme",
    THEME_DATA_ATTR[theme] ?? "dark"
  );
}
