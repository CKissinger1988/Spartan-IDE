import AsyncStorage from '@react-native-async-storage/async-storage';
import { DEFAULT_THEME_MODE, ThemeMode } from '../theme';

// Real §75.93 theme-mode persistence, user-requested ("Add user
// customizable theme and font options to all Spartan interfaces") --
// mirrors `offlineQueue.ts`/`decisionHistory.ts`'s own established real
// AsyncStorage read/write/defensive-parse convention exactly, since this
// app has no backend settings store of its own (matching the desktop
// Electron shell's `spartan-backend`-backed persistence is out of scope
// here -- mobile has never had a backend to persist through, per §69's
// own real v1 boundaries).

const STORAGE_KEY = 'spartan.themeMode';

function isThemeMode(value: unknown): value is ThemeMode {
  return value === 'dark' || value === 'light';
}

export async function getThemeMode(): Promise<ThemeMode> {
  const raw = await AsyncStorage.getItem(STORAGE_KEY);
  if (raw && isThemeMode(raw)) {
    return raw;
  }
  return DEFAULT_THEME_MODE;
}

export async function setThemeMode(mode: ThemeMode): Promise<void> {
  await AsyncStorage.setItem(STORAGE_KEY, mode);
}
