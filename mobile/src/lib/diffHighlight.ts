// §69.1's "syntax-highlighted diff/file viewing" — parses a unified diff
// patch string into typed lines so the screen can color each one, instead
// of rendering the raw patch text as one flat block.

export type DiffLineType = 'add' | 'remove' | 'hunk' | 'context';

export interface DiffLine {
  type: DiffLineType;
  text: string;
}

export function parseDiff(patch: string): DiffLine[] {
  if (!patch) return [];

  // Mock patches (and real ones) end with a trailing '\n' — drop the empty
  // line that trailing newline would otherwise produce.
  const lines = patch.split('\n');
  if (lines[lines.length - 1] === '') lines.pop();

  return lines.map((line) => {
    if (line.startsWith('@@')) return { type: 'hunk', text: line };
    if (line.startsWith('+')) return { type: 'add', text: line };
    if (line.startsWith('-')) return { type: 'remove', text: line };
    return { type: 'context', text: line };
  });
}
