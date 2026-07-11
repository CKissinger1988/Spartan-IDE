import { DarkTheme, Theme } from '@react-navigation/native';

// The same design tokens as the desktop reference prototype
// (../../prototypes/interface-prototype.jsx's `C` object), per
// docs/architecture-spec.md §50.3's high-contrast, Antigravity-researched
// palette (background/surface/border values researched from Antigravity's
// own documented tokens; the rust/terracotta accent is Spartan's own kept
// identity, not Antigravity's purple -- see §50.3's named divergence).
// Kept in sync with the desktop values rather than reinvented for mobile.
export const C = {
  bg: '#09090B',
  s1: '#141416',
  s2: '#18181B',
  s3: '#202024',
  border: '#27272A',
  borderLt: '#35353A',
  text: '#E9E7E4',
  textMid: '#A6A5A2',
  textDim: '#84838A',
  accent: '#C4432B',
  accentDim: '#8F3323',
  accentBg: 'rgba(196,67,43,0.13)',
  accentBorder: 'rgba(196,67,43,0.4)',
  green: '#4E9E72',
  greenBg: 'rgba(78,158,114,0.13)',
  amber: '#C99A3D',
  amberBg: 'rgba(201,154,61,0.13)',
  red: '#B2453B',
  redBg: 'rgba(178,69,59,0.13)',
  teal: '#3D9C93',
  tealBg: 'rgba(61,156,147,0.13)',
} as const;

// Real, bundled JetBrains Mono (see assets/fonts/README.md) -- the same
// default-monospace-font choice as every other Spartan project (desktop/,
// web/, crates/spartan-editor-core), registered via the `expo-font` config
// plugin in app.json rather than left to whatever `'Courier'` happens to
// resolve to per-platform.
export const MONO_FONT_FAMILY = 'JetBrainsMono-Regular';

// Matches STATUS_META in interface-prototype.jsx exactly -- desktop has no
// "paused" state on mobile's SessionStatus, so it's omitted here rather
// than invented.
export const STATUS_COLOR = {
  running: C.accent,
  review: C.amber,
  done: C.green,
} as const;

// Applied via <NavigationContainer theme={navigationTheme}> so every
// screen's native-stack header and background come from these tokens by
// default, instead of each screen re-deriving its own header colors.
export const navigationTheme: Theme = {
  ...DarkTheme,
  colors: {
    ...DarkTheme.colors,
    primary: C.accent,
    background: C.bg,
    card: C.s1,
    text: C.text,
    border: C.border,
    notification: C.red,
  },
};
