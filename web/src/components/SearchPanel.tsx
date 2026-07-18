import React, { useCallback, useState } from "react";
import type { BackendClient } from "../backendClient";

interface SearchMatch {
  path: string;
  line: number;
  text: string;
}

interface SearchPanelProps {
  client: BackendClient;
  root: string;
  /** Real click-to-jump (a real 0-indexed line), ported verbatim from
   * `desktop/`'s own identical prop -- see that file's own doc comment
   * for the full real reasoning, including the real 1-indexed-vs-
   * 0-indexed off-by-one bug this exact conversion fixes. */
  onOpenResult: (absolutePath: string, zeroIndexedLine: number) => void;
}

/**
 * Real "Find in Files," ported verbatim from `desktop/src/components/
 * SearchPanel.tsx`'s own identical wiring -- see that file's own doc
 * comment for the full real reasoning, including why this is
 * Enter-to-search rather than per-keystroke.
 */
export default function SearchPanel({
  client,
  root,
  onOpenResult,
}: SearchPanelProps): React.ReactElement {
  const [query, setQuery] = useState("");
  const [matches, setMatches] = useState<SearchMatch[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const runSearch = useCallback(() => {
    const pattern = query.trim();
    if (!pattern) {
      setMatches(null);
      setError(null);
      return;
    }
    setSearching(true);
    setError(null);
    client
      .call("search_project", { project_root: root, pattern })
      .then((result) => {
        const r = result as { matches: SearchMatch[] };
        setMatches(r.matches);
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setSearching(false));
  }, [client, query, root]);

  const groups = new Map<string, SearchMatch[]>();
  for (const m of matches ?? []) {
    const list = groups.get(m.path) ?? [];
    list.push(m);
    groups.set(m.path, list);
  }

  return (
    <div className="git-panel">
      <input
        className="git-commit-input mono"
        placeholder="Search in files (Enter)"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") runSearch();
        }}
        style={{ minHeight: "auto", height: 28 }}
      />
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
