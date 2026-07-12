import { C_DARK, C_LIGHT, colorsForMode, navigationTheme } from '../theme';

describe('theme', () => {
  test('colorsForMode returns the real dark palette for "dark"', () => {
    expect(colorsForMode('dark')).toBe(C_DARK);
  });

  test('colorsForMode returns the real light palette for "light"', () => {
    expect(colorsForMode('light')).toBe(C_LIGHT);
  });

  test('the dark and light palettes are genuinely different colors, not a copy-paste', () => {
    expect(C_DARK.bg).not.toBe(C_LIGHT.bg);
    expect(C_DARK.text).not.toBe(C_LIGHT.text);
    expect(C_DARK.accent).not.toBe(C_LIGHT.accent);
  });

  test('navigationTheme("dark") uses the real dark palette tokens', () => {
    const theme = navigationTheme('dark');
    expect(theme.colors.background).toBe(C_DARK.bg);
    expect(theme.colors.primary).toBe(C_DARK.accent);
    expect(theme.dark).toBe(true);
  });

  test('navigationTheme("light") uses the real light palette tokens', () => {
    const theme = navigationTheme('light');
    expect(theme.colors.background).toBe(C_LIGHT.bg);
    expect(theme.colors.primary).toBe(C_LIGHT.accent);
    expect(theme.dark).toBe(false);
  });
});
