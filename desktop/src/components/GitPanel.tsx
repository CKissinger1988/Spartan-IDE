import React, { useCallback, useEffect, useState } from "react";

interface StatusEntry {
  path: string;
  staged: string | null;
  unstaged: string | null;
  conflicted: boolean;
}

interface GitStatus {
  branch: string | null;
  entries: StatusEntry[];
}

const STATUS_GLYPH: Record<string, string> = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  type_changed: "T",
};

interface GitPanelProps {
  root: string;
}

interface ExpandedDiff {
  path: string;
  staged: boolean;
}

/** Real diff rendering -- ported verbatim from `LeoChatPanel.tsx`'s own
 * `DiffView` (one `<div>` per real line, colored by its real `+`/`-`/` `
 * prefix). Deliberately duplicated rather than extracted into a shared
 * component -- the two call sites (Leo's own generated-edit preview here,
 * a real git diff) have nothing else in common, and this is small enough
 * that a shared abstraction would cost more than it saves. */
function DiffView({ diff }: { diff: string }): React.ReactElement {
  const lines = diff.split("\n").filter((_, i, arr) => !(i === arr.length - 1 && arr[i] === ""));
  if (lines.length === 0) {
    return <div className="leo-diff mono git-panel-empty">No changes.</div>;
  }
  return (
    <pre className="leo-diff mono">
      {lines.map((line, i) => {
        const kind = line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "ctx";
        return (
          <div key={i} className={`leo-diff-line leo-diff-${kind}`}>
            {line || " "}
          </div>
        );
      })}
    </pre>
  );
}

/**
 * Real local Source Control panel for the Electron shell (§56.1, task
 * #56), the second half of the "Git panel + Settings screen" priority
 * item -- closes an Electron-shell parity gap the original wgpu shell
 * has had since §75.30. Reuses that shell's own click-to-stage/unstage
 * interaction model (a row in "Changes" stages on click, a row in
 * "Staged Changes" unstages on click) rather than a checkbox UI, and its
 * own independent staged/unstaged-per-file status split (a real git
 * semantic -- a file can be both staged *and* have further unstaged
 * changes on top).
 *
 * Real diff view (task #229-232): a small "±" button on each row, kept
 * deliberately separate from the row's own stage/unstage click target via
 * `stopPropagation` so viewing a diff never accidentally stages/unstages
 * the file it's showing. Clicking it toggles a real, inline expansion
 * calling the real `git_diff` IPC method -- `staged: true` for a row
 * under "Staged Changes" (a real `HEAD`-vs-index diff), `staged: false`
 * for a row under "Changes" (a real index-vs-working-tree diff) -- reused
 * via `DiffView`, the same rendering already proven correct for Leo's own
 * edit-preview diffs.
 *
 * A deliberate, named v1 scope cut, matching this whole `desktop/`
 * effort's own established pattern of naming what's deferred rather than
 * silently omitting it: no branch switcher, no per-hunk staging, no
 * stash, no merge-conflict resolution UI -- conflicted files are shown
 * with a marker but not specially handled.
 */
export default function GitPanel({ root }: GitPanelProps): React.ReactElement {
  const [status, setStatus] = useState<GitStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [committing, setCommitting] = useState(false);
  const [expandedDiff, setExpandedDiff] = useState<ExpandedDiff | null>(null);
  const [diffContent, setDiffContent] = useState<string | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const [diffError, setDiffError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    window.spartan
      .call("git_status", { project_root: root })
      .then((result) => {
        setStatus(result as GitStatus);
        setError(null);
      })
      .catch((e: Error) => {
        setStatus(null);
        setError(e.message);
      });
  }, [root]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const stage = useCallback(
    (path: string) => {
      window.spartan
        .call("git_stage", { project_root: root, path })
        .then(refresh)
        .catch((e: Error) => setError(e.message));
    },
    [root, refresh]
  );

  const unstage = useCallback(
    (path: string) => {
      window.spartan
        .call("git_unstage", { project_root: root, path })
        .then(refresh)
        .catch((e: Error) => setError(e.message));
    },
    [root, refresh]
  );

  const commit = useCallback(() => {
    if (!message.trim()) return;
    setCommitting(true);
    window.spartan
      .call("git_commit", { project_root: root, message })
      .then(() => {
        setMessage("");
        refresh();
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setCommitting(false));
  }, [root, message, refresh]);

  const toggleDiff = useCallback(
    (path: string, staged: boolean, e: React.MouseEvent) => {
      e.stopPropagation();
      if (expandedDiff && expandedDiff.path === path && expandedDiff.staged === staged) {
        setExpandedDiff(null);
        setDiffContent(null);
        setDiffError(null);
        return;
      }
      setExpandedDiff({ path, staged });
      setDiffContent(null);
      setDiffError(null);
      setDiffLoading(true);
      window.spartan
        .call("git_diff", { project_root: root, path, staged })
        .then((result) => setDiffContent((result as { diff: string }).diff))
        .catch((err: Error) => setDiffError(err.message))
        .finally(() => setDiffLoading(false));
    },
    [root, expandedDiff]
  );

  if (error) {
    return <div className="git-panel git-panel-empty mono">{error}</div>;
  }
  if (!status) {
    return <div className="git-panel git-panel-empty mono">Loading git status…</div>;
  }

  const staged = status.entries.filter((e) => e.staged);
  const unstaged = status.entries.filter((e) => e.unstaged);

  return (
    <div className="git-panel">
      <div className="git-branch mono">{status.branch ? `⎇ ${status.branch}` : "(detached HEAD)"}</div>

      <textarea
        className="git-commit-input mono"
        placeholder="Commit message"
        value={message}
        onChange={(e) => setMessage(e.target.value)}
      />
      <button
        className="git-commit-button"
        disabled={staged.length === 0 || !message.trim() || committing}
        onClick={commit}
      >
        {committing ? "Committing…" : `Commit (${staged.length})`}
      </button>

      <div className="git-section-label mono">Staged Changes ({staged.length})</div>
      <div className="git-section">
        {staged.map((entry) => (
          <React.Fragment key={`staged-${entry.path}`}>
            <div className="git-row" onClick={() => unstage(entry.path)}>
              <span className="git-status-glyph mono">{STATUS_GLYPH[entry.staged ?? ""] ?? "?"}</span>
              <span className="mono git-row-path">{entry.path}</span>
              <button
                type="button"
                className="editor-find-btn"
                title="View diff"
                onClick={(e) => toggleDiff(entry.path, true, e)}
              >
                ±
              </button>
            </div>
            {expandedDiff?.path === entry.path && expandedDiff.staged === true && (
              <div onClick={(e) => e.stopPropagation()}>
                {diffLoading && <div className="git-panel-empty mono">Loading diff…</div>}
                {diffError && <div className="git-panel-empty mono">{diffError}</div>}
                {diffContent !== null && <DiffView diff={diffContent} />}
              </div>
            )}
          </React.Fragment>
        ))}
      </div>

      <div className="git-section-label mono">Changes ({unstaged.length})</div>
      <div className="git-section">
        {unstaged.map((entry) => (
          <React.Fragment key={`unstaged-${entry.path}`}>
            <div className="git-row" onClick={() => stage(entry.path)}>
              <span className="git-status-glyph mono">{STATUS_GLYPH[entry.unstaged ?? ""] ?? "?"}</span>
              <span className="mono git-row-path">{entry.path}</span>
              {entry.conflicted && <span className="git-conflict-marker mono">!</span>}
              <button
                type="button"
                className="editor-find-btn"
                title="View diff"
                onClick={(e) => toggleDiff(entry.path, false, e)}
              >
                ±
              </button>
            </div>
            {expandedDiff?.path === entry.path && expandedDiff.staged === false && (
              <div onClick={(e) => e.stopPropagation()}>
                {diffLoading && <div className="git-panel-empty mono">Loading diff…</div>}
                {diffError && <div className="git-panel-empty mono">{diffError}</div>}
                {diffContent !== null && <DiffView diff={diffContent} />}
              </div>
            )}
          </React.Fragment>
        ))}
      </div>

      {status.entries.length === 0 && (
        <div className="git-panel-empty mono">No changes.</div>
      )}
    </div>
  );
}
