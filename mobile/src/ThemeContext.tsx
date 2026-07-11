import { createContext, ReactNode, useContext, useEffect, useMemo, useState } from 'react';
import { getThemeMode, setThemeMode as persistThemeMode } from './lib/themePreference';
import { C_DARK, colorsForMode, DEFAULT_THEME_MODE, ThemeColors, ThemeMode } from './theme';

// Real §75.93 live theme context, user-requested ("Add user customizable
// theme and font options to all Spartan interfaces"). Unlike the wgpu
// desktop shell's own real "applies next launch" scope (that pass's own
// environment had no display/GPU available to verify a live mid-session
// palette swap), React Native's own render model makes a real *live*
// swap the natural, correct choice here -- every screen that reads
// `useTheme().colors` re-renders automatically the instant the mode
// changes, no restart needed.
interface ThemeContextValue {
  mode: ThemeMode;
  colors: ThemeColors;
  setMode: (mode: ThemeMode) => void;
}

const ThemeContext = createContext<ThemeContextValue>({
  mode: DEFAULT_THEME_MODE,
  colors: C_DARK,
  setMode: () => {},
});

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [mode, setModeState] = useState<ThemeMode>(DEFAULT_THEME_MODE);

  // Real, one-time read of the real, previously-persisted preference on
  // mount -- defaults to `DEFAULT_THEME_MODE` (dark) until it resolves,
  // matching this app's own real `userInterfaceStyle: "dark"` default so
  // there's no visible flash of an unexpected theme before the real
  // stored value loads.
  useEffect(() => {
    getThemeMode().then(setModeState);
  }, []);

  const setMode = (next: ThemeMode) => {
    setModeState(next);
    // Real, best-effort persistence -- a write failure must never block
    // the real, already-applied live UI update (the same "don't let a
    // secondary I/O failure hide that the primary action succeeded"
    // discipline this project's own crash-report upload path already
    // established, §75.82).
    persistThemeMode(next).catch(() => {});
  };

  const colors = useMemo(() => colorsForMode(mode), [mode]);

  return (
    <ThemeContext.Provider value={{ mode, colors, setMode }}>{children}</ThemeContext.Provider>
  );
}

export function useTheme(): ThemeContextValue {
  return useContext(ThemeContext);
}
