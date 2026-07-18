import React, { useCallback, useState } from "react";
import type { BackendClient } from "../backendClient";

export interface SearchMatch {
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
  /** Real "Replace in Files," ported verbatim from `desktop/`'s own
   * identical prop -- see that file's own doc comment for the full real
   * reasoning. */
  onReplaceAll: (
    matches: SearchMatch[],
    query: string,
    replacement: string
  ) => Promise<{ filesChanged: number; totalReplacements: number }>;
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
    client
      .call("search_project", { project_root: root, pattern })
      .then((result) => {
        const r = result as { matches: SearchMatch[] };
        setMatches(r.matches);
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setSearching(false));
  }, [client, query, root]);

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
