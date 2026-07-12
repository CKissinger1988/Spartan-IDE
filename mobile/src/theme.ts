import { DarkTheme, DefaultTheme, Theme } from '@react-navigation/native';

// The same design tokens as the desktop reference prototype
// (../../prototypes/interface-prototype.jsx's `C` object), per
// docs/architecture-spec.md §50.3's high-contrast, Antigravity-researched
// palette (background/surface/border values researched from Antigravity's
// own documented tokens). Kept in sync with the desktop values rather than
// reinvented for mobile.
//
// Real §75.95 user-requested rebrand: "All Spartan projects default colors
// are blue and gold." `accent` (blue, `#2E7DFF`) replaces the original
// rust/terracotta primary accent -- the exact same real hex already shipped
// for `desktop/src/theme.css`'s/`web/src/theme.css`'s dark `--accent`, one
// shared brand palette. `gold` is a new, real, distinct token (not reusing
// `amber`, which stays a status-semantic color for the "review" state) for
// the secondary brand accent -- matching `desktop/`'s own `--hud` gold
// token, used here for the theme toggle's own Light-mode active pill
// (`SettingsScreen.tsx`) as a real, visible "both brand colors together"
// moment rather than left as an unused, symbolic-only constant.
export const C_DARK = {
  bg: '#09090B',
  s1: '#141416',
  s2: '#18181B',
  s3: '#202024',
  border: '#27272A',
  borderLt: '#35353A',
  text: '#E9E7E4',
  textMid: '#A6A5A2',
  textDim: '#84838A',
  accent: '#2E7DFF',
  accentDim: '#1E5FCC',
  accentBg: 'rgba(46,125,255,0.14)',
  accentBorder: 'rgba(46,125,255,0.4)',
  gold: '#D4AF37',
  goldDim: '#A9891F',
  goldBg: 'rgba(212,175,55,0.14)',
  goldBorder: 'rgba(212,175,55,0.4)',
  green: '#4E9E72',
  greenBg: 'rgba(78,158,114,0.13)',
  amber: '#C99A3D',
  amberBg: 'rgba(201,154,61,0.13)',
  red: '#B2453B',
  redBg: 'rgba(178,69,59,0.13)',
  teal: '#3D9C93',
  tealBg: 'rgba(61,156,147,0.13)',
} as const;

// Real §75.93 light theme, user-requested ("Add user customizable theme
// and font options to all Spartan interfaces") -- the exact same real
// bg/surface/border/text/accent values already researched and shipped
// for `desktop/`'s/`web/`'s own light theme
// (`desktop/src/theme.css`'s `:root[data-theme="light"]` block), not a
// second, independently-invented palette. Real §75.95 rebrand: `accent`
// (blue, `#1B54C4`) and `gold` (`#9C7A1D`) are both real, deliberately
// re-picked slightly deeper shades of their dark-theme counterparts (not
// the identical hex) so accent-colored text/icons keep real, legible
// contrast against a light background instead of washing out -- the
// identical reasoning `desktop/src/theme.css`'s own doc comment already
// gives. `green`/`amber`/`red`/`teal` are kept at their real dark-theme
// hues -- these are always used as solid badge/dot fills paired with
// fixed white text, not as text-on-background, so they read correctly on
// a light background unchanged.
export const C_LIGHT = {
  bg: '#F4F3F1',
  s1: '#FAF9F7',
  s2: '#FFFFFF',
  s3: '#FFFFFF',
  border: '#DCDAD5',
  borderLt: '#C5C2BC',
  text: '#1B1A19',
  textMid: '#4A4844',
  textDim: '#6B6862',
  accent: '#1B54C4',
  accentDim: '#123D8F',
  accentBg: 'rgba(27,84,196,0.10)',
  accentBorder: 'rgba(27,84,196,0.35)',
  gold: '#9C7A1D',
  goldDim: '#725911',
  goldBg: 'rgba(156,122,29,0.10)',
  goldBorder: 'rgba(156,122,29,0.35)',
  green: '#4E9E72',
  greenBg: 'rgba(78,158,114,0.13)',
  amber: '#C99A3D',
  amberBg: 'rgba(201,154,61,0.13)',
  red: '#B2453B',
  redBg: 'rgba(178,69,59,0.13)',
  teal: '#3D9C93',
  tealBg: 'rgba(61,156,147,0.13)',
} as const;

export type ThemeColors = Record<keyof typeof C_DARK, string>;
export type ThemeMode = 'dark' | 'light';

/** Real, non-negotiable default -- `SpartanDark` is this app's own
 * already-shipped, only-ever-tested palette (`userInterfaceStyle: "dark"`
 * in `app.json`), so every existing install looks unchanged until a user
 * explicitly opts into light. */
export const DEFAULT_THEME_MODE: ThemeMode = 'dark';

export function colorsForMode(mode: ThemeMode): ThemeColors {
  return mode === 'light' ? C_LIGHT : C_DARK;
}

/** Real, backward-compatible default export -- `C` keeps meaning "the
 * dark palette" for any real call site that hasn't been converted to the
 * reactive `useTheme()` hook (`StatusPill.tsx`'s own `STATUS_COLOR`
 * below, which intentionally stays theme-invariant -- see its own doc
 * comment). */
export const C = C_DARK;

// Real, bundled JetBrains Mono (see assets/fonts/README.md) -- the same
// default-monospace-font choice as every other Spartan project (desktop/,
// web/, crates/spartan-editor-core), registered via the `expo-font` config
// plugin in app.json rather than left to whatever `'Courier'` happens to
// resolve to per-platform.
export const MONO_FONT_FAMILY = 'JetBrainsMono-Regular';

// Matches STATUS_META in interface-prototype.jsx exactly -- desktop has no
// "paused" state on mobile's SessionStatus, so it's omitted here rather
// than invented. Deliberately theme-invariant: `StatusPill` always pairs
// these with fixed white text as a solid badge fill (§75.93 audit), the
// same real reasoning `green`/`amber`/`red`/`teal` above stay fixed hues
// across both themes for.
export const STATUS_COLOR = {
  running: C_DARK.accent,
  review: C_DARK.amber,
  done: C_DARK.green,
} as const;

// Applied via <NavigationContainer theme={navigationTheme(mode)}> so every
// screen's native-stack header and background come from these tokens by
// default, instead of each screen re-deriving its own header colors. Real
// §75.93 change: was a fixed `const`, now a function of the real selected
// mode -- `DefaultTheme` (react-navigation's own light base) for
// `'light'`, matching the same real base-theme-plus-override pattern the
// original dark version already used with `DarkTheme`.
export function navigationTheme(mode: ThemeMode): Theme {
  const colors = colorsForMode(mode);
  const base = mode === 'light' ? DefaultTheme : DarkTheme;
  return {
    ...base,
    colors: {
      ...base.colors,
      primary: colors.accent,
      background: colors.bg,
      card: colors.s1,
      text: colors.text,
      border: colors.border,
      notification: colors.red,
    },
  };
}
