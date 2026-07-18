import React, { useCallback, useState } from "react";

export interface SearchMatch {
  path: string;
  line: number;
  text: string;
}

interface SearchPanelProps {
  root: string;
  /** Real click-to-jump -- hands the real absolute path and a real
   * 0-indexed line up to `App.tsx`, which reuses the exact same real
   * `handleJumpToDefinition` open-then-jump machinery go-to-definition
   * already established (character always 0: a search match names a
   * line, not a column). **A real off-by-one bug was caught only by
   * live testing, not by inspection**: `search_project`'s own real
   * `SearchMatch.line` is 1-indexed (a real, correct display
   * convention), but `jumpToLocalPosition`/`pendingJump` -- shared with
   * every LSP-backed jump in this app -- expect a real 0-indexed LSP
   * line, matching `textDocument/definition`'s own convention. Passing
   * the raw 1-indexed value through landed the caret one real line past
   * the actual match every time; this component's own click handler
   * subtracts 1 before calling this prop so callers never need to
   * remember the conversion themselves. */
  onOpenResult: (absolutePath: string, zeroIndexedLine: number) => void;
  /** Real "Replace in Files" (task #226) -- given the real current search
   * results, a query, and a replacement, `App.tsx`'s own
   * `applyReplaceInFiles` opens (or reuses) every real affected file and
   * applies the replacement through the real `edit` IPC call, resolving
   * to the real number of files/occurrences actually changed. */
  onReplaceAll: (
    matches: SearchMatch[],
    query: string,
    replacement: string
  ) => Promise<{ filesChanged: number; totalReplacements: number }>;
}

/**
 * Real "Find in Files" (task #190/#191) -- the first real, direct UI
 * caller of `spartan_leo::tool::Sandbox::search_files`, a real, bounded
 * substring search that's existed since §75.68 as one of Leo's own tool
 * calls but never had a caller outside the agent loop. Deliberately
 * Enter-to-search, not per-keystroke -- matching this whole session's
 * own established "smallest real, correct increment" precedent (Ctrl+
 * Space over automatic completion) rather than firing a real filesystem
 * walk on every typed character.
 */
export default function SearchPanel({
  root,
  onOpenResult,
  onReplaceAll,
}: SearchPanelProps): React.ReactElement {
  const [query, setQuery] = useState("");
  const [matches, setMatches] = useState<SearchMatch[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showReplace, setShowReplace] = useState(false);
  const [replaceQuery, setReplaceQuery] = useState("");
  const [replacing, setReplacing] = useState(false);
  const [replaceStatus, setReplaceStatus] = useState<string | null>(null);

  const runSearch = useCallback(() => {
    const pattern = query.trim();
    if (!pattern) {
      setMatches(null);
      setError(null);
      return;
    }
    setSearching(true);
    setError(null);
    window.spartan
      .call("search_project", { project_root: root, pattern })
      .then((result) => {
        const r = result as { matches: SearchMatch[] };
        setMatches(r.matches);
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setSearching(false));
  }, [query, root]);

  const runReplaceAll = useCallback(() => {
    if (!matches || matches.length === 0) return;
    setReplacing(true);
    setReplaceStatus(null);
    onReplaceAll(matches, query.trim(), replaceQuery)
      .then((result) => {
        setReplaceStatus(
          `Replaced ${result.totalReplacements} occurrence(s) across ${result.filesChanged} file(s).`
        );
        // Real, deliberate re-search rather than trusting the now-stale
        // preview: reflects the real, post-replace on-disk state exactly
        // the same way a fresh Enter-triggered search would.
        runSearch();
      })
      .catch((e: Error) => setReplaceStatus(`Replace failed: ${e.message}`))
      .finally(() => setReplacing(false));
  }, [matches, query, replaceQuery, onReplaceAll, runSearch]);

  const groups = new Map<string, SearchMatch[]>();
  for (const m of matches ?? []) {
    const list = groups.get(m.path) ?? [];
    list.push(m);
    groups.set(m.path, list);
  }

  return (
    <div className="git-panel">
      <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
        <input
          className="git-commit-input mono"
          placeholder="Search in files (Enter)"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") runSearch();
          }}
          style={{ minHeight: "auto", height: 28, flex: 1 }}
        />
        <button
          type="button"
          className={`editor-find-btn${showReplace ? " editor-find-btn-active" : ""}`}
          onClick={() => setShowReplace((prev) => !prev)}
          title="Toggle Replace"
        >
          ⇄
        </button>
      </div>
      {showReplace && (
        <div style={{ display: "flex", gap: 4, alignItems: "center", marginTop: 4 }}>
          <input
            className="git-commit-input mono"
            placeholder="Replace with…"
            value={replaceQuery}
            onChange={(e) => setReplaceQuery(e.target.value)}
            style={{ minHeight: "auto", height: 28, flex: 1 }}
          />
          <button
            type="button"
            className="git-commit-button"
            disabled={!matches || matches.length === 0 || replacing}
            onClick={runReplaceAll}
            style={{ whiteSpace: "nowrap" }}
          >
            {replacing ? "Replacing…" : `Replace All${matches ? ` (${matches.length})` : ""}`}
          </button>
        </div>
      )}
      {replaceStatus && <div className="git-panel-empty mono">{replaceStatus}</div>}
      {error && <div className="git-panel-empty mono">{error}</div>}
      {searching && <div className="git-panel-empty mono">Searching…</div>}
      {!searching && matches !== null && matches.length === 0 && (
        <div className="git-panel-empty mono">No matches.</div>
      )}
      {!searching &&
        matches !== null &&
        Array.from(groups.entries()).map(([path, groupMatches]) => (
          <div key={path}>
            <div className="git-section-label mono">
              {path} ({groupMatches.length})
            </div>
            <div className="git-section">
              {groupMatches.map((m) => {
                const absolutePath = `${root.replace(/\/+$/, "")}/${path}`;
                return (
                  <div
                    key={`${path}:${m.line}`}
                    className="git-row"
                    onClick={() => onOpenResult(absolutePath, Math.max(0, m.line - 1))}
                    title={m.text}
                  >
                    <span className="git-status-glyph mono">{m.line}</span>
                    <span className="mono git-row-path">{m.text}</span>
                  </div>
                );
              })}
            </div>
          </div>
        ))}
    </div>
  );
}
