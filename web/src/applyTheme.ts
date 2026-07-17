/**
 * Real §75.93 theme application, extended from 2 to 7 real variants --
 * copied verbatim from `desktop/src/applyTheme.ts` (one shared source of
 * truth, matching this project's own existing "copied verbatim from
 * desktop/" convention already used for `syntax.ts`/`theme.css`). Sets a
 * real `data-theme` attribute on the document root; `theme.css`'s own
 * `:root[data-theme="..."]` blocks are what react to it.
 *
 * Unlike `desktop/`, this app has no `spartan-backend` settings store to
 * read from/write to for the pure client-side path (§75.89's own named
 * scope) -- `App.tsx` persists the chosen value to `localStorage` directly
 * instead, the same real, deliberate pattern `desktop/`'s own Leo
 * voice-output toggle already established (§75.71) for a pure renderer
 * preference with no backend of its own.
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

/** Human-readable labels for the theme `<select>`. */
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
