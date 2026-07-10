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
 * A deliberate, named v1 scope cut, matching this whole `desktop/`
 * effort's own established pattern of naming what's deferred rather than
 * silently omitting it: no diff view (no Diff Card component exists in
 * this shell yet), no branch switcher, no per-hunk staging, no stash, no
 * merge-conflict resolution UI -- conflicted files are shown with a
 * marker but not specially handled.
 */
export default function GitPanel({ root }: GitPanelProps): React.ReactElement {
  const [status, setStatus] = useState<GitStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [committing, setCommitting] = useState(false);

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
          <div key={`staged-${entry.path}`} className="git-row" onClick={() => unstage(entry.path)}>
            <span className="git-status-glyph mono">{STATUS_GLYPH[entry.staged ?? ""] ?? "?"}</span>
            <span className="mono git-row-path">{entry.path}</span>
          </div>
        ))}
      </div>

      <div className="git-section-label mono">Changes ({unstaged.length})</div>
      <div className="git-section">
        {unstaged.map((entry) => (
          <div key={`unstaged-${entry.path}`} className="git-row" onClick={() => stage(entry.path)}>
            <span className="git-status-glyph mono">{STATUS_GLYPH[entry.unstaged ?? ""] ?? "?"}</span>
            <span className="mono git-row-path">{entry.path}</span>
            {entry.conflicted && <span className="git-conflict-marker mono">!</span>}
          </div>
        ))}
      </div>

      {status.entries.length === 0 && (
        <div className="git-panel-empty mono">No changes.</div>
      )}
    </div>
  );
}
