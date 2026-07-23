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

interface BranchInfo {
  name: string;
  current: boolean;
}

interface CommitInfo {
  oid: string;
  summary: string;
  author: string;
  /** Real commit time, unix seconds. */
  time: number;
}

interface ChangedFile {
  path: string;
  status: string;
}

/** Real relative-age formatting for the history list -- coarse on
 * purpose (a source-control sidebar, not a timestamp report). */
function formatAge(unixSeconds: number): string {
  const seconds = Math.max(0, Math.floor(Date.now() / 1000) - unixSeconds);
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
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
 * Real branch switcher (task #233-235), ported verbatim from `desktop/`'s
 * own copy: clicking the branch label opens a freshly-fetched branch
 * list, clicking a non-current branch performs a real safe checkout (a
 * conflicting dirty change surfaces libgit2's own real refusal, repo
 * untouched), and a new-branch input creates a real branch from `HEAD`
 * without switching.
 *
 * Real commit history + per-commit detail (task #236-237), ported
 * verbatim from `desktop/`'s own copy: a History section, and clicking a
 * commit row expands its real changed-file list, clicking a file within
 * it drills into that file's real per-commit diff.
 *
 * Same real, named v1 scope cut as the ported original: no per-hunk
 * staging, no stash, no branch delete/rename, no merge-conflict
 * resolution UI (conflicted files show a marker only).
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
  const [showBranches, setShowBranches] = useState(false);
  const [branches, setBranches] = useState<BranchInfo[] | null>(null);
  const [branchError, setBranchError] = useState<string | null>(null);
  const [newBranchName, setNewBranchName] = useState("");
  const [switching, setSwitching] = useState(false);
  const [expandedCommit, setExpandedCommit] = useState<string | null>(null);
  const [commitFiles, setCommitFiles] = useState<ChangedFile[] | null>(null);
  const [commitFilesError, setCommitFilesError] = useState<string | null>(null);
  const [expandedCommitFile, setExpandedCommitFile] = useState<string | null>(null);
  const [commitFileDiff, setCommitFileDiff] = useState<string | null>(null);
  const [commitFileDiffLoading, setCommitFileDiffLoading] = useState(false);
  const [commitFileDiffError, setCommitFileDiffError] = useState<string | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  const [commits, setCommits] = useState<CommitInfo[] | null>(null);
  const [historyError, setHistoryError] = useState<string | null>(null);
  // Real git remote operations (P1 backlog), mirroring desktop/'s own.
  const [remotes, setRemotes] = useState<{ name: string; url: string | null }[] | null>(null);
  const [remoteBusy, setRemoteBusy] = useState(false);
  const [remoteStatus, setRemoteStatus] = useState<string | null>(null);
  // Real git stash (roadmap), mirroring desktop/'s own.
  const [stashes, setStashes] = useState<{ index: number; message: string; oid: string }[]>([]);
  const [stashBusy, setStashBusy] = useState(false);
  const [stashError, setStashError] = useState<string | null>(null);

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

  useEffect(() => {
    client
      .call("git_remotes", { project_root: root })
      .then((r) => setRemotes((r as { remotes?: { name: string; url: string | null }[] }).remotes ?? []))
      .catch(() => setRemotes([]));
  }, [client, root]);

  const runRemote = useCallback(
    (op: "fetch" | "pull" | "push") => {
      const remote = remotes?.[0]?.name;
      const branch = status?.branch;
      if (!remote) {
        setRemoteStatus("No remote configured");
        return;
      }
      if ((op === "pull" || op === "push") && !branch) {
        setRemoteStatus(`Detached HEAD — no branch to ${op}`);
        return;
      }
      setRemoteBusy(true);
      setRemoteStatus(op === "fetch" ? "Fetching…" : op === "pull" ? "Pulling…" : "Pushing…");
      const method = op === "fetch" ? "git_fetch" : op === "pull" ? "git_pull" : "git_push";
      const params: Record<string, unknown> = { project_root: root, remote };
      if (op !== "fetch") params.branch = branch;
      client
        .call(method, params)
        .then((r) => {
          if (op === "pull") {
            const outcome = (r as { outcome?: string }).outcome;
            setRemoteStatus(
              outcome === "fast_forwarded"
                ? "Pulled (fast-forward)"
                : outcome === "up_to_date"
                  ? "Already up to date"
                  : "Diverged — pull left your branch untouched (fast-forward only)"
            );
            refresh();
          } else {
            setRemoteStatus(op === "fetch" ? "Fetched" : "Pushed");
            if (op === "fetch") refresh();
          }
        })
        .catch((e: Error) => setRemoteStatus(`${op} failed: ${e.message}`))
        .finally(() => setRemoteBusy(false));
    },
    [remotes, status?.branch, client, root, refresh]
  );

  const refreshStashes = useCallback(() => {
    client
      .call("git_stash_list", { project_root: root })
      .then((r) => setStashes((r as { stashes?: typeof stashes }).stashes ?? []))
      .catch(() => setStashes([]));
  }, [client, root]);

  useEffect(() => {
    refreshStashes();
  }, [refreshStashes]);

  const stashSave = useCallback(() => {
    setStashBusy(true);
    setStashError(null);
    client
      .call("git_stash_save", { project_root: root, message: "" })
      .then((r) => {
        if (!(r as { stashed?: boolean }).stashed) setStashError("Nothing to stash");
        refresh();
        refreshStashes();
      })
      .catch((e: Error) => setStashError(e.message))
      .finally(() => setStashBusy(false));
  }, [client, root, refresh, refreshStashes]);

  const stashAction = useCallback(
    (method: "git_stash_pop" | "git_stash_drop", index: number) => {
      setStashBusy(true);
      setStashError(null);
      client
        .call(method, { project_root: root, index })
        .then(() => {
          refresh();
          refreshStashes();
        })
        .catch((e: Error) => setStashError(e.message))
        .finally(() => setStashBusy(false));
    },
    [client, root, refresh, refreshStashes]
  );

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

  const toggleBranches = useCallback(() => {
    if (showBranches) {
      setShowBranches(false);
      setBranchError(null);
      return;
    }
    // Fetched fresh on every open, never cached -- branches can change
    // out from under the panel between opens.
    setShowBranches(true);
    setBranchError(null);
    client
      .call("git_branches", { project_root: root })
      .then((result) => setBranches((result as { branches: BranchInfo[] }).branches))
      .catch((e: Error) => setBranchError(e.message));
  }, [client, root, showBranches]);

  const checkoutBranch = useCallback(
    (name: string) => {
      setSwitching(true);
      setBranchError(null);
      client
        .call("git_checkout", { project_root: root, branch: name })
        .then(() => {
          setShowBranches(false);
          setBranches(null);
          refresh();
        })
        // A real safe-checkout refusal surfaces libgit2's own real error
        // here, repo untouched -- shown, never force-resolved.
        .catch((e: Error) => setBranchError(e.message))
        .finally(() => setSwitching(false));
    },
    [client, root, refresh]
  );

  const createBranch = useCallback(() => {
    const name = newBranchName.trim();
    if (!name) return;
    setBranchError(null);
    client
      .call("git_create_branch", { project_root: root, branch: name })
      .then(() => {
        setNewBranchName("");
        return client.call("git_branches", { project_root: root });
      })
      .then((result) => setBranches((result as { branches: BranchInfo[] }).branches))
      .catch((e: Error) => setBranchError(e.message));
  }, [client, root, newBranchName]);

  const toggleHistory = useCallback(() => {
    if (showHistory) {
      setShowHistory(false);
      setHistoryError(null);
      return;
    }
    // Fetched fresh on every open, matching the branch list's own
    // no-caching choice -- a commit can land between opens.
    setShowHistory(true);
    setHistoryError(null);
    setCommits(null);
    client
      .call("git_log", { project_root: root, max: 25 })
      .then((result) => setCommits((result as { commits: CommitInfo[] }).commits))
      .catch((e: Error) => setHistoryError(e.message));
  }, [client, root, showHistory]);

  const toggleCommit = useCallback(
    (oid: string) => {
      if (expandedCommit === oid) {
        setExpandedCommit(null);
        setCommitFiles(null);
        setCommitFilesError(null);
        setExpandedCommitFile(null);
        setCommitFileDiff(null);
        setCommitFileDiffError(null);
        return;
      }
      setExpandedCommit(oid);
      setCommitFiles(null);
      setCommitFilesError(null);
      setExpandedCommitFile(null);
      setCommitFileDiff(null);
      setCommitFileDiffError(null);
      client
        .call("git_commit_files", { project_root: root, oid })
        .then((result) => setCommitFiles((result as { files: ChangedFile[] }).files))
        .catch((e: Error) => setCommitFilesError(e.message));
    },
    [client, root, expandedCommit]
  );

  const toggleCommitFile = useCallback(
    (oid: string, path: string) => {
      if (expandedCommitFile === path) {
        setExpandedCommitFile(null);
        setCommitFileDiff(null);
        setCommitFileDiffError(null);
        return;
      }
      setExpandedCommitFile(path);
      setCommitFileDiff(null);
      setCommitFileDiffError(null);
      setCommitFileDiffLoading(true);
      client
        .call("git_commit_diff", { project_root: root, oid, path })
        .then((result) => setCommitFileDiff((result as { diff: string }).diff))
        .catch((e: Error) => setCommitFileDiffError(e.message))
        .finally(() => setCommitFileDiffLoading(false));
    },
    [client, root, expandedCommitFile]
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
      <div
        className="git-branch mono"
        onClick={toggleBranches}
        style={{ cursor: "pointer" }}
        title="Switch branch"
      >
        {status.branch ? `⎇ ${status.branch}` : "(detached HEAD)"} {showBranches ? "▾" : "▸"}
      </div>
      {showBranches && (
        <div className="git-section">
          {branchError && <div className="git-panel-empty mono">{branchError}</div>}
          {branches === null && !branchError && (
            <div className="git-panel-empty mono">Loading branches…</div>
          )}
          {branches?.map((b) => (
            <div
              key={b.name}
              className="git-row"
              onClick={() => {
                if (!b.current && !switching) checkoutBranch(b.name);
              }}
              title={b.current ? "Current branch" : `Switch to ${b.name}`}
            >
              <span className="git-status-glyph mono">{b.current ? "✓" : ""}</span>
              <span className="mono git-row-path">{b.name}</span>
            </div>
          ))}
          <div style={{ display: "flex", gap: 4, alignItems: "center", marginTop: 4 }}>
            <input
              className="git-commit-input mono"
              placeholder="New branch name…"
              value={newBranchName}
              onChange={(e) => setNewBranchName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") createBranch();
              }}
              style={{ minHeight: "auto", height: 28, flex: 1 }}
            />
            <button
              type="button"
              className="editor-find-btn"
              disabled={!newBranchName.trim()}
              onClick={createBranch}
            >
              Create
            </button>
          </div>
        </div>
      )}

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

      {remotes && remotes.length > 0 && (
        <div className="git-section">
          <div className="git-section-label mono">Remote: {remotes[0].name}</div>
          <div style={{ display: "flex", gap: 4, marginTop: 4 }}>
            <button
              type="button"
              className="editor-find-btn"
              disabled={remoteBusy}
              onClick={() => runRemote("fetch")}
            >
              Fetch
            </button>
            <button
              type="button"
              className="editor-find-btn"
              disabled={remoteBusy}
              onClick={() => runRemote("pull")}
            >
              Pull
            </button>
            <button
              type="button"
              className="editor-find-btn"
              disabled={remoteBusy}
              onClick={() => runRemote("push")}
            >
              Push
            </button>
          </div>
          {remoteStatus && (
            <div className="git-panel-empty mono" style={{ marginTop: 4 }}>
              {remoteStatus}
            </div>
          )}
        </div>
      )}

      <div className="git-section">
        <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
          <div className="git-section-label mono" style={{ flex: 1 }}>
            Stashes ({stashes.length})
          </div>
          <button
            type="button"
            className="editor-find-btn"
            disabled={stashBusy}
            onClick={stashSave}
            title="Stash working changes"
          >
            Stash
          </button>
        </div>
        {stashes.map((s) => (
          <div key={s.index} className="git-row" style={{ cursor: "default" }}>
            <span className="mono git-row-path" title={s.oid}>
              {s.message}
            </span>
            <button
              type="button"
              className="editor-find-btn"
              disabled={stashBusy}
              onClick={() => stashAction("git_stash_pop", s.index)}
            >
              Pop
            </button>
            <button
              type="button"
              className="editor-find-btn"
              disabled={stashBusy}
              onClick={() => stashAction("git_stash_drop", s.index)}
            >
              Drop
            </button>
          </div>
        ))}
        {stashError && (
          <div className="git-panel-empty mono" style={{ marginTop: 4 }}>
            {stashError}
          </div>
        )}
      </div>

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

      <div
        className="git-section-label mono"
        onClick={toggleHistory}
        style={{ cursor: "pointer" }}
        title="Commit history"
      >
        History {showHistory ? "▾" : "▸"}
      </div>
      {showHistory && (
        <div className="git-section">
          {historyError && <div className="git-panel-empty mono">{historyError}</div>}
          {commits === null && !historyError && (
            <div className="git-panel-empty mono">Loading history…</div>
          )}
          {commits?.length === 0 && <div className="git-panel-empty mono">No commits yet.</div>}
          {commits?.map((c) => (
            <React.Fragment key={c.oid}>
              <div
                className="git-row"
                onClick={() => toggleCommit(c.oid)}
                style={{ cursor: "pointer" }}
                title={`${c.oid}\n${c.author} — ${new Date(c.time * 1000).toLocaleString()}`}
              >
                <span
                  className="mono"
                  style={{ color: "var(--accent)", fontSize: 11, flexShrink: 0 }}
                >
                  {c.oid.slice(0, 7)}
                </span>
                <span className="mono git-row-path">{c.summary}</span>
                <span
                  className="mono"
                  style={{ opacity: 0.6, whiteSpace: "nowrap", fontSize: 11 }}
                >
                  {formatAge(c.time)}
                </span>
              </div>
              {expandedCommit === c.oid && (
                <div style={{ paddingLeft: 12 }}>
                  {commitFilesError && (
                    <div className="git-panel-empty mono">{commitFilesError}</div>
                  )}
                  {commitFiles === null && !commitFilesError && (
                    <div className="git-panel-empty mono">Loading files…</div>
                  )}
                  {commitFiles?.length === 0 && (
                    <div className="git-panel-empty mono">No file changes.</div>
                  )}
                  {commitFiles?.map((f) => (
                    <React.Fragment key={f.path}>
                      <div
                        className="git-row"
                        onClick={() => toggleCommitFile(c.oid, f.path)}
                        style={{ cursor: "pointer" }}
                        title={`${f.status}: ${f.path}`}
                      >
                        <span className="git-status-glyph mono">
                          {STATUS_GLYPH[f.status] ?? "?"}
                        </span>
                        <span className="mono git-row-path">{f.path}</span>
                      </div>
                      {expandedCommitFile === f.path && (
                        <div>
                          {commitFileDiffLoading && (
                            <div className="git-panel-empty mono">Loading diff…</div>
                          )}
                          {commitFileDiffError && (
                            <div className="git-panel-empty mono">{commitFileDiffError}</div>
                          )}
                          {commitFileDiff !== null && <DiffView diff={commitFileDiff} />}
                        </div>
                      )}
                    </React.Fragment>
                  ))}
                </div>
              )}
            </React.Fragment>
          ))}
        </div>
      )}
    </div>
  );
}
