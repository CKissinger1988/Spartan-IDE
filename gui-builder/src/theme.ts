export interface ThemeTokenValue {
  name: string;
  value: string;
}

/** Builds a preview-only root override from real project CSS custom
 * properties. Names are restricted to custom-property syntax and values are
 * emitted as text inside a style element in the sandbox. */
export function buildThemeOverride(tokens: ThemeTokenValue[]): string {
  const declarations = tokens.flatMap((token) => {
    const name = token.name.trim();
    const value = token.value.trim();
    return /^--[A-Za-z0-9_-]+$/.test(name) && value && !/[{}<>]/.test(value)
      ? [`${name}:${value};`]
      : [];
  });
  return declarations.length > 0 ? `:root{${declarations.join("")}}` : "";
}
