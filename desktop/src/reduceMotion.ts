/**
 * Real §75.76 "reduce motion" appearance setting -- a small, shared
 * helper rather than duplicated in both `App.tsx` (applied once on
 * startup, before the user has necessarily opened Settings) and
 * `SettingsScreen.tsx` (applied immediately when the toggle changes, an
 * optimistic UI update ahead of the real `settings_set` round trip).
 */
export function applyReduceMotion(enabled: boolean): void {
  document.documentElement.classList.toggle("reduce-motion", enabled);
}
