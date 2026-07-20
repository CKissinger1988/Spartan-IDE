import React, { useCallback, useEffect, useState } from "react";
import type { BackendClient } from "../backendClient";

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
  client: BackendClient;
  /** The real, absolute project root the connected devserver advertised
   * (`client.projectRoot`) -- callers only render this component once
   * that's known to be non-null. */
  root: string;
}

interface ExpandedDiff {
  path: string;
  staged: boolean;
}

/** Real diff rendering -- ported verbatim from `desktop/`'s own copy in
 * `GitPanel.tsx` (itself ported from `LeoChatPanel.tsx`'s `DiffView`):
 * one `<div>` per real line, colored by its real `+`/`-`/` ` prefix. */
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
 * Real Source Control panel for the web app, closing the "git" half of the
 * gap `App.tsx`'s own doc comment named since §75.89 ("no LSP, no DAP, no
 * Leo, no git"). A direct port of `desktop/src/components/GitPanel.tsx`
 * (§75.65) -- identical interaction model (click a "Changes" row to stage
 * it, click a "Staged Changes" row to unstage it, independent staged/
 * unstaged-per-file split, the same real git semantic that shell already
 * established), with `window.spartan.call` replaced by the real
 * `BackendClient.call` this app connects with (§75.88's WebSocket
 * transport, reached via the devserver's own `/__spartan/session` token
 * handoff, §75.88 continued below by the project-root advertisement this
 * component depends on).
 *
 * Unlike `desktop/`, `root` here is never a value this app's own UI lets
 * the user choose -- it's the real path the devserver was launched
 * against (`--project-root:`), since the File System Access API has no
 * way to hand this component a real OS path for whatever folder was
 * separately opened via `showDirectoryPicker`. A real, deliberate
 * consequence, not hidden: this panel operates on the devserver's own
 * project root, which may or may not be the same directory the File
 * System Access side currently has open.
 *
 * Real diff view (task #229-232): a "±" button per row toggles an inline
 * expansion calling the real `git_diff` IPC method (staged rows diff
 * `HEAD`-vs-index, unstaged rows diff index-vs-working-tree), kept off
 * the row's own stage/unstage click target via `stopPropagation` --
 * ported verbatim from `desktop/`'s own copy.
 *
 * Same real, named v1 scope cut as the ported original: no branch
 * switcher, no per-hunk staging, no stash, no merge-conflict resolution
 * UI (conflicted files show a marker only).
 */
export default function GitPanel({ client, root }: GitPanelProps): React.ReactElement {
  const [status, setStatus] = useState<GitStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [committing, setCommitting] = useState(false);
  const [expandedDiff, setExpandedDiff] = useState<ExpandedDiff | null>(null);
  const [diffContent, setDiffContent] = useState<string | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const [diffError, setDiffError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    client
      .call("git_status", { project_root: root })
      .then((result) => {
        setStatus(result as GitStatus);
        setError(null);
      })
      .catch((e: Error) => {
        setStatus(null);
        setError(e.message);
      });
  }, [client, root]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const stage = useCallback(
    (path: string) => {
      client
        .call("git_stage", { project_root: root, path })
        .then(refresh)
        .catch((e: Error) => setError(e.message));
    },
    [client, root, refresh]
  );

  const unstage = useCallback(
    (path: string) => {
      client
        .call("git_unstage", { project_root: root, path })
        .then(refresh)
        .catch((e: Error) => setError(e.message));
    },
    [client, root, refresh]
  );

  const commit = useCallback(() => {
    if (!message.trim()) return;
    setCommitting(true);
    client
      .call("git_commit", { project_root: root, message })
      .then(() => {
        setMessage("");
        refresh();
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setCommitting(false));
  }, [client, root, message, refresh]);

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
      client
        .call("git_diff", { project_root: root, path, staged })
        .then((result) => setDiffContent((result as { diff: string }).diff))
        .catch((err: Error) => setDiffError(err.message))
        .finally(() => setDiffLoading(false));
    },
    [client, root, expandedDiff]
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

      {status.entries.length === 0 && <div className="git-panel-empty mono">No changes.</div>}
    </div>
  );
}
