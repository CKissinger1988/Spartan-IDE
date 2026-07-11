import AsyncStorage from '@react-native-async-storage/async-storage';
import { getThemeMode, setThemeMode } from '../themePreference';

// Mirrors themePreference.ts's private STORAGE_KEY -- needed here only to
// inject malformed/foreign state directly, exercising the module's own
// defensive-parse fallback rather than its public write path. Same real
// convention decisionHistory.test.ts/offlineQueue.test.ts already use.
const STORAGE_KEY = 'spartan.themeMode';

describe('themePreference', () => {
  beforeEach(async () => {
    await AsyncStorage.clear();
  });

  test('getThemeMode defaults to dark when nothing has been stored yet', async () => {
    await expect(getThemeMode()).resolves.toBe('dark');
  });

  test('setThemeMode then getThemeMode round-trips a real stored value', async () => {
    await setThemeMode('light');
    await expect(getThemeMode()).resolves.toBe('light');
    await setThemeMode('dark');
    await expect(getThemeMode()).resolves.toBe('dark');
  });

  test('an invalid stored value falls back to the real default, not thrown', async () => {
    await AsyncStorage.setItem(STORAGE_KEY, 'not-a-real-theme-mode');
    await expect(getThemeMode()).resolves.toBe('dark');
  });
});
