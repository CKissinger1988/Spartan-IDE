/** Real bracket-pair colorization -- extends the already-shipped, purely
 * cursor-adjacent matching-bracket highlight (`Editor.tsx`'s own
 * `findMatchingBracket`) to color *every* bracket in the document by
 * nesting depth, the standard "rainbow brackets" feature every mainstream
 * editor now ships. A real, deliberate, named v1 scope cut, matching the
 * exact same honest simplification `findMatchingBracket`'s own doc comment
 * already states: no string/comment awareness -- a bracket character
 * inside a string literal or comment is tracked like any other, since this
 * is a plain raw-text scan, not a tokenizer-aware pass. Reusing an LSP or
 * tree-sitter parse for this would be real, separate, much larger work
 * (and neither is guaranteed to exist for every open file); a linear scan
 * is correct for the overwhelmingly common case and matches this codebase's
 * own established "smallest real, correct mechanism" precedent.
 *
 * Uses one combined depth counter across all three bracket kinds
 * (`(`/`[`/`{`), matching VS Code's own default bracket-pair-colorization
 * behavior -- a `(` immediately inside a `[...]` is one nesting level
 * deeper than the `[`, not tracked on a separate per-kind counter.
 *
 * A real, stack-based matching pass (not just a depth counter) is used so
 * a genuinely unmatched bracket -- a stray closer with no real opener on
 * the stack, or an opener never closed by end-of-document -- is correctly
 * distinguished and reported with `colorIndex: -1` rather than silently
 * misassigned a color as if it were validly paired. */

export interface BracketPairMark {
  line: number;
  character: number;
  /** 0-indexed nesting-depth color slot, cycling through
   * `BRACKET_PAIR_COLOR_COUNT` colors -- or `-1` for a real, genuinely
   * unmatched bracket (a stray closer, or an opener never closed). */
  colorIndex: number;
}

export const BRACKET_PAIR_COLOR_COUNT = 4;

/** A real, bounded safety limit -- a pathologically huge file (or one with
 * an enormous number of brackets) stops being tracked past this many real
 * bracket marks rather than growing the returned array without bound,
 * matching this codebase's own established "bounded, not unbounded"
 * precedent (e.g. Find in Files' own 200-match cap). Ordinary source files
 * never come close to this. */
const MAX_BRACKET_PAIR_MARKS = 5000;

const BRACKET_OPENERS = new Set(["(", "[", "{"]);
const BRACKET_CLOSE_TO_OPEN: Record<string, string> = { ")": "(", "]": "[", "}": "{" };

export function computeBracketPairMarks(content: string): BracketPairMark[] {
  const marks: BracketPairMark[] = [];
  // `markIndex` is `null` for an opener scanned after the real output cap
  // was already hit -- matching (stack push/pop) still tracks it for
  // correctness, it just has no rendered mark of its own. Deliberately
  // *not* the same condition as the loop bound below: matching must run
  // over the whole real document regardless of the cap, or a real opener
  // pushed just before the cap would be misreported as unmatched the
  // instant its own real closer past the cap is never even scanned.
  const stack: { markIndex: number | null; char: string }[] = [];
  let line = 0;
  let character = 0;

  for (let i = 0; i < content.length; i++) {
    const ch = content[i];
    if (ch === "\n") {
      line++;
      character = 0;
      continue;
    }
    if (BRACKET_OPENERS.has(ch)) {
      const colorIndex = stack.length % BRACKET_PAIR_COLOR_COUNT;
      let markIndex: number | null = null;
      if (marks.length < MAX_BRACKET_PAIR_MARKS) {
        marks.push({ line, character, colorIndex });
        markIndex = marks.length - 1;
      }
      stack.push({ markIndex, char: ch });
    } else if (ch in BRACKET_CLOSE_TO_OPEN) {
      const expectedOpen = BRACKET_CLOSE_TO_OPEN[ch];
      const top = stack[stack.length - 1];
      if (top && top.char === expectedOpen) {
        stack.pop();
        // Shares the same color as its already-recorded opener, so a
        // pair reads as one visually matched unit -- only if both the
        // opener and this closer landed inside the real cap; `markIndex`
        // growth is monotonic, so a `null` opener here always also means
        // the cap is already exceeded for this closer too.
        if (top.markIndex !== null && marks.length < MAX_BRACKET_PAIR_MARKS) {
          marks.push({ line, character, colorIndex: marks[top.markIndex].colorIndex });
        }
      } else if (marks.length < MAX_BRACKET_PAIR_MARKS) {
        // A real stray closer -- nothing valid on the stack to pair with.
        marks.push({ line, character, colorIndex: -1 });
      }
    }
    character++;
  }

  // Any openers still on the stack were never validly closed by
  // end-of-document -- reclassify their already-pushed mark as unmatched
  // rather than leaving it colored as if it were a real, closed pair.
  // `markIndex === null` means this opener was scanned past the real
  // output cap and never had a mark to reclassify in the first place.
  for (const { markIndex } of stack) {
    if (markIndex !== null) {
      marks[markIndex] = { ...marks[markIndex], colorIndex: -1 };
    }
  }

  return marks;
}
