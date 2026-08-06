/** Browser-safe token hierarchy and dependency analysis. */

export type TokenTier = "primitive" | "semantic" | "component";

export interface TokenModel {
  tier: TokenTier;
  references: string[];
}

/**
 * Classifies an explicit token hierarchy without guessing from arbitrary
 * product naming. `--component-*`/`--cmp-*` are component tokens,
 * `--semantic-*`/`--theme-*` are semantic tokens, and aliases become
 * semantic by default. Everything else remains a primitive token.
 */
export function describeToken(name: string, value: string): TokenModel {
  const normalized = name.trim().toLowerCase();
  const references = [...value.matchAll(/var\(\s*(--[a-zA-Z0-9_-]+)/g)].map((match) => match[1]);
  const tier: TokenTier = normalized.startsWith("--component-") || normalized.startsWith("--cmp-")
    ? "component"
    : normalized.startsWith("--semantic-") || normalized.startsWith("--theme-") || references.length > 0
      ? "semantic"
      : "primitive";
  return { tier, references: [...new Set(references)] };
}
