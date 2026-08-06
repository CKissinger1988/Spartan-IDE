/** Browser-safe helpers for binding GUI Builder values to CSS custom properties. */

function validateTokenName(name: string): void {
  if (!/^--[a-zA-Z0-9_-]+$/.test(name)) throw new Error(`Invalid CSS custom-property name "${name}".`);
}

/** Builds a safe CSS custom-property reference for a discovered token. */
export function buildTokenReference(name: string): string {
  validateTokenName(name);
  return `var(${name})`;
}
