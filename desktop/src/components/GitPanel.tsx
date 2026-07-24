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

interface ConflictEntry {
  path: string;
  ancestor: string | null;
  ours: string | null;
  theirs: string | null;
}

interface HunkInfo {
  index: number;
  header: string;
  body: string;
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

/** Split a line's content into word/whitespace/punctuation tokens for
 * word-level (intra-line) diffing. */
function tokenizeForDiff(s: string): string[] {
  return s.match(/[A-Za-z0-9_]+|\s+|[^A-Za-z0-9_\s]/g) ?? [];
}

/** LCS-based token diff: returns which tokens of each side are *changed*
 * (i.e. not part of the longest common subsequence). */
function tokenDiff(a: string[], b: string[]): [boolean[], boolean[]] {
  const n = a.length;
  const m = b.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const aChanged = new Array(n).fill(true);
  const bChanged = new Array(m).fill(true);
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      aChanged[i] = false;
      bChanged[j] = false;
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      i++;
    } else {
      j++;
    }
  }
  return [aChanged, bChanged];
}

/** Merge adjacent same-flag tokens into contiguous rendered segments. */
function toSegments(tokens: string[], changed: boolean[]): { text: string; changed: boolean }[] {
  const out: { text: string; changed: boolean }[] = [];
  for (let k = 0; k < tokens.length; k++) {
    const last = out[out.length - 1];
    if (last && last.changed === changed[k]) last.text += tokens[k];
    else out.push({ text: tokens[k], changed: changed[k] });
  }
  return out;
}

/** Real diff rendering -- one `<div>` per real line, colored by its real
 * `+`/`-`/` ` prefix, now with word-level (intra-line) highlighting: a run
 * of removed lines immediately followed by added lines is paired up and the
 * changed *words* within each pair are emphasized, so a one-token edit no
 * longer reads as a whole line replaced. Deliberately duplicated rather than
 * extracted into a shared component -- the two call sites (Leo's own
 * generated-edit preview, a real git diff) have nothing else in common. */
interface DiffLine {
  kind: "add" | "del" | "ctx";
  prefix: string;
  content: string;
  raw: string;
}

type Seg = { text: string; changed: boolean };

/** Parse a unified diff string into per-line records plus the word-level
 * change segments for paired removed/added runs. Shared by both the unified
 * and split renderers below. */
function parseDiff(diff: string): { lines: DiffLine[]; wordSegs: Map<number, Seg[]> } {
  const raw = diff.split("\n").filter((_, i, arr) => !(i === arr.length - 1 && arr[i] === ""));
  const lines: DiffLine[] = raw.map((line) => {
    const kind = line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "ctx";
    return { kind, prefix: line ? line[0] : " ", content: line ? line.slice(1) : "", raw: line };
  });
  const wordSegs = new Map<number, Seg[]>();
  let idx = 0;
  while (idx < lines.length) {
    if (lines[idx].kind !== "del") {
      idx++;
      continue;
    }
    let d = idx;
    while (d < lines.length && lines[d].kind === "del") d++;
    let a = d;
    while (a < lines.length && lines[a].kind === "add") a++;
    const pairs = Math.min(d - idx, a - d);
    for (let k = 0; k < pairs; k++) {
      const delIdx = idx + k;
      const addIdx = d + k;
      const at = tokenizeForDiff(lines[delIdx].content);
      const bt = tokenizeForDiff(lines[addIdx].content);
      const [ac, bc] = tokenDiff(at, bt);
      wordSegs.set(delIdx, toSegments(at, ac));
      wordSegs.set(addIdx, toSegments(bt, bc));
    }
    idx = a; // always > current idx (a >= d >= idx + 1)
  }
  return { lines, wordSegs };
}

/** Render one diff line's content, applying word-level highlighting when the
 * line has change segments (paired del/add), else the raw text. */
function renderDiffContent(line: DiffLine, segs: Seg[] | undefined): React.ReactNode {
  if (!segs) return line.content || "";
  return segs.map((s, si) =>
    s.changed ? (
      <span key={si} className={`leo-diff-word leo-diff-word-${line.kind}`}>
        {s.text}
      </span>
    ) : (
      <React.Fragment key={si}>{s.text}</React.Fragment>
    )
  );
}

/** Turn the unified line list into side-by-side rows: paired del/add runs sit
 * on the same row (left=del, right=add); a context line spans both columns;
 * unpaired del/add lines get a blank cell on the other side. */
function buildSplitRows(
  lines: DiffLine[],
  wordSegs: Map<number, Seg[]>
): { left: number | null; right: number | null }[] {
  const rows: { left: number | null; right: number | null }[] = [];
  let idx = 0;
  while (idx < lines.length) {
    if (lines[idx].kind === "ctx") {
      rows.push({ left: idx, right: idx });
      idx++;
      continue;
    }
    // Gather a run of dels then a run of adds.
    let d = idx;
    while (d < lines.length && lines[d].kind === "del") d++;
    let a = d;
    while (a < lines.length && lines[a].kind === "add") a++;
    const dels: number[] = [];
    for (let i = idx; i < d; i++) dels.push(i);
    const adds: number[] = [];
    for (let i = d; i < a; i++) adds.push(i);
    const n = Math.max(dels.length, adds.length);
    for (let k = 0; k < n; k++) {
      rows.push({ left: dels[k] ?? null, right: adds[k] ?? null });
    }
    idx = a > idx ? a : idx + 1;
  }
  return rows;
}

/** Real diff rendering with a unified / side-by-side (split) toggle. Unified
 * is one `<div>` per line colored by its `+`/`-`/` ` prefix; split lays paired
 * removed/added lines in two columns. Both share word-level (intra-line)
 * highlighting so a one-token edit doesn't read as a whole line replaced. */
function DiffView({ diff }: { diff: string }): React.ReactElement {
  const [split, setSplit] = useState(false);
  const { lines, wordSegs } = parseDiff(diff);
  if (lines.length === 0) {
    return <div className="leo-diff mono git-panel-empty">No changes.</div>;
  }

  const toggle = (
    <button
      type="button"
      className="editor-find-btn leo-diff-split-toggle"
      title={split ? "Show unified diff" : "Show side-by-side diff"}
      onClick={(e) => {
        e.stopPropagation();
        setSplit((s) => !s);
      }}
    >
      {split ? "Unified" : "Split"}
    </button>
  );

  if (split) {
    const rows = buildSplitRows(lines, wordSegs);
    return (
      <div className="leo-diff-split-wrap">
        {toggle}
        <div className="leo-diff leo-diff-split mono">
          {rows.map((row, i) => {
            const l = row.left !== null ? lines[row.left] : null;
            const r = row.right !== null ? lines[row.right] : null;
            return (
              <div key={i} className="leo-diff-split-row">
                <div className={`leo-diff-split-cell leo-diff-${l ? l.kind : "blank"}`}>
                  {l ? renderDiffContent(l, wordSegs.get(row.left as number)) : ""}
                </div>
                <div className={`leo-diff-split-cell leo-diff-${r ? r.kind : "blank"}`}>
                  {r ? renderDiffContent(r, wordSegs.get(row.right as number)) : ""}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    );
  }

  return (
    <div className="leo-diff-split-wrap">
      {toggle}
      <pre className="leo-diff mono">
        {lines.map((line, i) => {
          const segs = wordSegs.get(i);
          return (
            <div key={i} className={`leo-diff-line leo-diff-${line.kind}`}>
              {segs ? (
                <>
                  {line.prefix}
                  {renderDiffContent(line, segs)}
                </>
              ) : (
                line.raw || " "
              )}
            </div>
          );
        })}
      </pre>
    </div>
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
 * Real branch switcher (task #233-235): clicking the branch label opens
 * a real, freshly-fetched branch list -- clicking a non-current branch
 * performs a real *safe* checkout (`spartan_git::checkout_branch` uses
 * libgit2's own conflict-refusing checkout, so a conflicting dirty change
 * surfaces the real error with the repo untouched, never force-discarded)
 * -- plus a new-branch input creating a real branch from `HEAD` without
 * switching to it (matching `git branch`'s own behavior).
 *
 * Real commit history (task #236) with a per-commit detail view (task
 * #237): clicking a History row expands the real list of files that
 * commit changed (relative to its first parent), and clicking a file
 * within it drills into that file's real per-commit diff (its blob vs.
 * the parent's blob), reusing the same `DiffView` the working-tree diff
 * already uses.
 *
 * Real merge-conflict resolution UI (task #270): a "Merge" button on each
 * non-current branch row runs a real `git_merge_branch` (fast-forward,
 * a real no-conflict three-way merge, or a real conflicted merge --
 * conflict markers written to the working tree exactly like the real
 * `git merge` CLI). While a merge is genuinely in progress
 * (`git_merge_status`'s own real `RepositoryState::Merge` check), a
 * dedicated conflict panel lists every real conflicted file with its
 * real `ours`/`theirs` content, one-click "Take ours"/"Take theirs"
 * resolution, and a manual-edit textarea (pre-filled with `ours`) for a
 * hand-merged result -- a real, named v1 simplification versus opening
 * the file in the main code editor tab, not yet wired to this panel.
 * "Complete Merge" is disabled until every conflict is resolved and
 * performs a real two-parent commit; "Abort" is a real, destructive
 * `git merge --abort` equivalent, confirmed first.
 *
 * Real per-hunk staging (task #271): expanding an *unstaged* row's diff
 * (the merge-conflict panel's own "±" button, unaffected) also fetches its
 * real hunk list via `spartan_git::diff_hunks` (built on real
 * `git2::Patch::from_blob_and_buffer`, not a hand-rolled diff) and renders
 * a "Stage this hunk" button per hunk -- `resolve_conflict_with_content`'s
 * own real one-file-at-a-time discipline applied one layer finer.
 * `stage_hunk` recomputes the diff fresh every call, so hunks are always
 * staged one at a time against the real current index, never a stale
 * client-side list; both the hunk list and overall status are refetched
 * after each stage.
 *
 * A deliberate, named v1 scope cut, matching this whole `desktop/`
 * effort's own established pattern of naming what's deferred rather than
 * silently omitting it: no per-line (sub-hunk) selection, no unstage-a-
 * hunk (whole-file unstage only), no stash-during-merge interplay, no
 * branch delete/rename, no merging via drag-and-drop.
 */
export default function GitPanel({ root }: GitPanelProps): React.ReactElement {
  const [status, setStatus] = useState<GitStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [committing, setCommitting] = useState(false);
  const [amending, setAmending] = useState(false);
  const [expandedDiff, setExpandedDiff] = useState<ExpandedDiff | null>(null);
  const [diffContent, setDiffContent] = useState<string | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const [diffError, setDiffError] = useState<string | null>(null);
  // Real per-hunk staging (task #271) -- only fetched for an unstaged
  // row's own expansion (a staged diff has nothing left to hunk-stage).
  const [hunks, setHunks] = useState<HunkInfo[] | null>(null);
  const [hunksError, setHunksError] = useState<string | null>(null);
  const [hunkBusy, setHunkBusy] = useState(false);
  const [showBranches, setShowBranches] = useState(false);
  const [branches, setBranches] = useState<BranchInfo[] | null>(null);
  // Real remote-tracking branches (`origin/feature`) as of the last fetch
  // (task #251) -- null until the branch list is opened.
  const [remoteBranches, setRemoteBranches] = useState<string[] | null>(null);
  const [branchError, setBranchError] = useState<string | null>(null);
  const [newBranchName, setNewBranchName] = useState("");
  // Real cherry-pick (task #272): browsing a branch's own commits (local or
  // remote-tracking) without checking it out, and applying one of them onto
  // the current branch. At most one branch's commit log expanded at a time,
  // matching this panel's own established "one expansion" convention.
  const [expandedBranchLog, setExpandedBranchLog] = useState<string | null>(null);
  const [branchLogCommits, setBranchLogCommits] = useState<CommitInfo[] | null>(null);
  const [branchLogError, setBranchLogError] = useState<string | null>(null);
  const [cherryPickBusy, setCherryPickBusy] = useState(false);
  const [cherryPickError, setCherryPickError] = useState<string | null>(null);
  const [switching, setSwitching] = useState(false);
  const [showHistory, setShowHistory] = useState(false);
  const [commits, setCommits] = useState<CommitInfo[] | null>(null);
  const [historyError, setHistoryError] = useState<string | null>(null);
  // At most one commit expanded at a time (like the diff view), keyed by
  // oid; `commitFiles`/`commitDiff` hold the currently-expanded commit's
  // real changed-file list and the one file within it the user drilled
  // into (if any).
  const [expandedCommit, setExpandedCommit] = useState<string | null>(null);
  const [commitFiles, setCommitFiles] = useState<ChangedFile[] | null>(null);
  const [commitFilesError, setCommitFilesError] = useState<string | null>(null);
  const [expandedCommitFile, setExpandedCommitFile] = useState<string | null>(null);
  const [commitFileDiff, setCommitFileDiff] = useState<string | null>(null);
  const [commitFileDiffLoading, setCommitFileDiffLoading] = useState(false);
  const [commitFileDiffError, setCommitFileDiffError] = useState<string | null>(null);
  // Real git remote operations (P1 backlog) -- fetch/pull/push against a
  // configured remote. Fast-forward-only pull; a divergence is reported,
  // never auto-merged. Loaded once per root.
  const [remotes, setRemotes] = useState<{ name: string; url: string | null }[] | null>(null);
  const [remoteBusy, setRemoteBusy] = useState(false);
  const [remoteStatus, setRemoteStatus] = useState<string | null>(null);
  // Real git stash (roadmap): stash working changes, list/pop/drop.
  const [stashes, setStashes] = useState<{ index: number; message: string; oid: string }[]>([]);
  const [stashBusy, setStashBusy] = useState(false);
  const [stashError, setStashError] = useState<string | null>(null);
  const [stashMessage, setStashMessage] = useState("");
  // Real git tags.
  const [tags, setTags] = useState<{ name: string; target: string; annotated: boolean }[]>([]);
  // Real merge-conflict resolution (task #270). `mergeStatus` reflects
  // `git_merge_status`'s real `RepositoryState::Merge` check plus every
  // real conflicted file's ancestor/ours/theirs content -- refetched
  // after every action that could change it (branch checkout, merge
  // attempt, a resolve, completing/aborting).
  const [mergeStatus, setMergeStatus] = useState<{
    inProgress: boolean;
    conflicts: ConflictEntry[];
  } | null>(null);
  const [mergeBusy, setMergeBusy] = useState(false);
  const [mergeError, setMergeError] = useState<string | null>(null);
  const [mergeCommitMessage, setMergeCommitMessage] = useState("");
  const [conflictEdits, setConflictEdits] = useState<Record<string, string>>({});

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

  useEffect(() => {
    window.spartan
      .call("git_remotes", { project_root: root })
      .then((r) => setRemotes((r as { remotes?: { name: string; url: string | null }[] }).remotes ?? []))
      .catch(() => setRemotes([]));
  }, [root]);

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
      window.spartan
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
    [remotes, status?.branch, root, refresh]
  );

  const refreshStashes = useCallback(() => {
    window.spartan
      .call("git_stash_list", { project_root: root })
      .then((r) => setStashes((r as { stashes?: typeof stashes }).stashes ?? []))
      .catch(() => setStashes([]));
  }, [root]);

  const refreshTags = useCallback(() => {
    window.spartan
      .call("git_tags", { project_root: root })
      .then((r) => setTags((r as { tags?: typeof tags }).tags ?? []))
      .catch(() => setTags([]));
  }, [root]);

  const refreshMergeStatus = useCallback(() => {
    window.spartan
      .call("git_merge_status", { project_root: root })
      .then((r) => {
        const result = r as { in_progress: boolean; conflicts: ConflictEntry[] };
        setMergeStatus({ inProgress: result.in_progress, conflicts: result.conflicts });
      })
      .catch(() => setMergeStatus(null));
  }, [root]);

  useEffect(() => {
    refreshStashes();
    refreshTags();
    refreshMergeStatus();
  }, [refreshStashes, refreshTags, refreshMergeStatus]);

  // Real merge attempt (task #270) -- fast-forward, a clean three-way
  // merge, or a real conflicted merge (conflict markers written to the
  // working tree, exactly like the real `git merge` CLI). Whatever the
  // real outcome, both status and merge-status are refetched so the
  // conflict panel (if any) appears immediately.
  const mergeBranch = useCallback(
    (name: string, e: React.MouseEvent) => {
      e.stopPropagation();
      setMergeBusy(true);
      setMergeError(null);
      setBranchError(null);
      window.spartan
        .call("git_merge_branch", { project_root: root, branch: name })
        .then(() => {
          setShowBranches(false);
          setBranches(null);
          setConflictEdits({});
          refresh();
          refreshMergeStatus();
        })
        .catch((e: Error) => setBranchError(e.message))
        .finally(() => setMergeBusy(false));
    },
    [root, refresh, refreshMergeStatus]
  );

  // Real one-click conflict resolution -- writes `content` to the real
  // working-tree file and stages it. `content` is typically a conflict's
  // own real `ours`/`theirs` value ("Take ours"/"Take theirs") or the
  // user's own hand-edited text from the manual-edit textarea below.
  const resolveConflictWith = useCallback(
    (path: string, content: string) => {
      setMergeBusy(true);
      setMergeError(null);
      window.spartan
        .call("git_resolve_conflict", { project_root: root, path, content })
        .then(() => refreshMergeStatus())
        .catch((e: Error) => setMergeError(e.message))
        .finally(() => setMergeBusy(false));
    },
    [root, refreshMergeStatus]
  );

  // Completes the merge with a real two-parent commit once every real
  // conflict is resolved.
  const commitMerge = useCallback(() => {
    const msg = mergeCommitMessage.trim() || "Merge";
    setMergeBusy(true);
    setMergeError(null);
    window.spartan
      .call("git_commit_merge", { project_root: root, message: msg })
      .then(() => {
        setMergeCommitMessage("");
        setConflictEdits({});
        refresh();
        refreshMergeStatus();
      })
      .catch((e: Error) => setMergeError(e.message))
      .finally(() => setMergeBusy(false));
  }, [root, mergeCommitMessage, refresh, refreshMergeStatus]);

  // A real, destructive `git merge --abort` equivalent -- resets the
  // working tree/index back to `HEAD`, discarding the in-progress merge
  // and any partial resolutions. Confirmed first.
  const abortMerge = useCallback(() => {
    if (
      !window.confirm(
        "Abort this merge? All conflict resolutions will be discarded and the working tree reset to HEAD."
      )
    )
      return;
    setMergeBusy(true);
    setMergeError(null);
    window.spartan
      .call("git_abort_merge", { project_root: root })
      .then(() => {
        setConflictEdits({});
        refresh();
        refreshMergeStatus();
      })
      .catch((e: Error) => setMergeError(e.message))
      .finally(() => setMergeBusy(false));
  }, [root, refresh, refreshMergeStatus]);

  // Tag a specific commit (lightweight, via a prompt for the name).
  const tagCommit = useCallback(
    (oid: string, e: React.MouseEvent) => {
      e.stopPropagation();
      const name = window.prompt(`Tag name for commit ${oid.slice(0, 7)}:`);
      if (!name || !name.trim()) return;
      window.spartan
        .call("git_create_tag", { project_root: root, name: name.trim(), oid })
        .then(refreshTags)
        .catch((err: Error) => setError(err.message));
    },
    [root, refreshTags]
  );

  const deleteTag = useCallback(
    (name: string) => {
      if (!window.confirm(`Delete tag "${name}"?`)) return;
      window.spartan
        .call("git_delete_tag", { project_root: root, name })
        .then(refreshTags)
        .catch((err: Error) => setError(err.message));
    },
    [root, refreshTags]
  );

  const stashSave = useCallback(() => {
    setStashBusy(true);
    setStashError(null);
    window.spartan
      .call("git_stash_save", { project_root: root, message: stashMessage.trim() })
      .then((r) => {
        if (!(r as { stashed?: boolean }).stashed) setStashError("Nothing to stash");
        else setStashMessage("");
        refresh();
        refreshStashes();
      })
      .catch((e: Error) => setStashError(e.message))
      .finally(() => setStashBusy(false));
  }, [root, stashMessage, refresh, refreshStashes]);

  const stashAction = useCallback(
    (method: "git_stash_pop" | "git_stash_apply" | "git_stash_drop", index: number) => {
      setStashBusy(true);
      setStashError(null);
      window.spartan
        .call(method, { project_root: root, index })
        .then(() => {
          refresh();
          refreshStashes();
        })
        .catch((e: Error) => setStashError(e.message))
        .finally(() => setStashBusy(false));
    },
    [root, refresh, refreshStashes]
  );

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

  // Real "discard changes" -- destructive (restores the working file to the
  // index version, dropping unstaged edits), so confirm first.
  const discard = useCallback(
    (path: string) => {
      if (!window.confirm(`Discard all unstaged changes to ${path}? This cannot be undone.`)) return;
      window.spartan
        .call("git_discard", { project_root: root, path })
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

  // Amend rewrites the last commit's message (and folds in any staged
  // changes), rather than adding a new commit -- a real, destructive
  // history rewrite, so it's confirmed before running.
  const amend = useCallback(() => {
    if (!message.trim()) return;
    if (
      !window.confirm(
        "Amend the last commit? This rewrites its message and history and cannot be undone."
      )
    )
      return;
    setAmending(true);
    window.spartan
      .call("git_commit_amend", { project_root: root, message })
      .then(() => {
        setMessage("");
        refresh();
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setAmending(false));
  }, [root, message, refresh]);

  // Revert creates a NEW commit undoing the named one (never rewrites
  // history), so it's safe on already-pushed commits -- but it's still a
  // real commit, so confirm before running. Refreshes both status and the
  // history list afterward.
  const revert = useCallback(
    (oid: string, summary: string, e: React.MouseEvent) => {
      e.stopPropagation();
      if (
        !window.confirm(
          `Revert commit ${oid.slice(0, 7)} ("${summary}")? This adds a new commit that undoes it.`
        )
      )
        return;
      window.spartan
        .call("git_revert_commit", { project_root: root, oid })
        .then(() => {
          refresh();
          return window.spartan.call("git_log", { project_root: root, max: 25 });
        })
        .then((result) => setCommits((result as { commits: CommitInfo[] }).commits))
        .catch((err: Error) => setError(err.message));
    },
    [root, refresh]
  );

  // Real per-hunk fetch (task #271), reused both when opening an unstaged
  // row's expansion and after a hunk is staged (whose own staging changes
  // the real remaining hunk list, so it's always refetched fresh, never
  // patched client-side).
  const refreshHunks = useCallback(
    (path: string) => {
      window.spartan
        .call("git_diff_hunks", { project_root: root, path })
        .then((result) => setHunks((result as { hunks: HunkInfo[] }).hunks))
        .catch((err: Error) => setHunksError(err.message));
    },
    [root]
  );

  const toggleDiff = useCallback(
    (path: string, staged: boolean, e: React.MouseEvent) => {
      e.stopPropagation();
      if (expandedDiff && expandedDiff.path === path && expandedDiff.staged === staged) {
        setExpandedDiff(null);
        setDiffContent(null);
        setDiffError(null);
        setHunks(null);
        setHunksError(null);
        return;
      }
      setExpandedDiff({ path, staged });
      setDiffContent(null);
      setDiffError(null);
      setHunks(null);
      setHunksError(null);
      setDiffLoading(true);
      window.spartan
        .call("git_diff", { project_root: root, path, staged })
        .then((result) => setDiffContent((result as { diff: string }).diff))
        .catch((err: Error) => setDiffError(err.message))
        .finally(() => setDiffLoading(false));
      // Hunks only make sense for an *unstaged* row -- a staged diff has
      // nothing left to hunk-stage.
      if (!staged) refreshHunks(path);
    },
    [root, expandedDiff, refreshHunks]
  );

  // Real "stage this one hunk" -- stages, then refreshes both overall
  // status (staged/unstaged counts, and whether the file leaves "Changes"
  // entirely once its last hunk is staged) and the real remaining hunk
  // list for this same path.
  const stageHunk = useCallback(
    (path: string, hunkIndex: number) => {
      setHunkBusy(true);
      setHunksError(null);
      window.spartan
        .call("git_stage_hunk", { project_root: root, path, hunk_index: hunkIndex })
        .then(() => {
          refresh();
          refreshHunks(path);
        })
        .catch((err: Error) => setHunksError(err.message))
        .finally(() => setHunkBusy(false));
    },
    [root, refresh, refreshHunks]
  );

  const toggleBranches = useCallback(() => {
    if (showBranches) {
      setShowBranches(false);
      setBranchError(null);
      return;
    }
    // Fetched fresh on every open, never cached -- branches can change
    // out from under the panel (a terminal `git branch`, Leo's own
    // checkpointing) between opens.
    setShowBranches(true);
    setBranchError(null);
    window.spartan
      .call("git_branches", { project_root: root })
      .then((result) => setBranches((result as { branches: BranchInfo[] }).branches))
      .catch((e: Error) => setBranchError(e.message));
    // Remote-tracking branches (task #251) -- a repo with no remotes just
    // returns an empty list, never an error.
    window.spartan
      .call("git_remote_branches", { project_root: root })
      .then((result) => setRemoteBranches((result as { branches: string[] }).branches))
      .catch(() => setRemoteBranches([]));
  }, [root, showBranches]);

  const checkoutBranch = useCallback(
    (name: string) => {
      setSwitching(true);
      setBranchError(null);
      window.spartan
        .call("git_checkout", { project_root: root, branch: name })
        .then(() => {
          setShowBranches(false);
          setBranches(null);
          refresh();
        })
        // A real safe-checkout refusal (a conflicting dirty change)
        // surfaces libgit2's own real error here, with the repository
        // untouched -- shown, never force-resolved.
        .catch((e: Error) => setBranchError(e.message))
        .finally(() => setSwitching(false));
    },
    [root, refresh]
  );

  // Real check out of a remote branch (task #251): creates a local tracking
  // branch if needed, then a safe checkout (same refusal-on-conflict rule).
  const checkoutRemoteBranch = useCallback(
    (remoteBranch: string) => {
      setSwitching(true);
      setBranchError(null);
      window.spartan
        .call("git_checkout_remote", { project_root: root, branch: remoteBranch })
        .then(() => {
          setShowBranches(false);
          setBranches(null);
          setRemoteBranches(null);
          refresh();
        })
        .catch((e: Error) => setBranchError(e.message))
        .finally(() => setSwitching(false));
    },
    [root, refresh]
  );

  // Real, live "browse this branch's own commits" toggle (task #272) --
  // works for both local branch names and remote-tracking ones (e.g.
  // `origin/feature`), matching `git_log_for_ref`'s own two-namespace
  // resolution. Fetched fresh on every open, matching every other
  // expandable list this panel already has.
  const toggleBranchLog = useCallback(
    (refName: string, e: React.MouseEvent) => {
      e.stopPropagation();
      if (expandedBranchLog === refName) {
        setExpandedBranchLog(null);
        setBranchLogCommits(null);
        setBranchLogError(null);
        return;
      }
      setExpandedBranchLog(refName);
      setBranchLogCommits(null);
      setBranchLogError(null);
      setCherryPickError(null);
      window.spartan
        .call("git_log_for_ref", { project_root: root, ref_name: refName, max: 10 })
        .then((result) => setBranchLogCommits((result as { commits: CommitInfo[] }).commits))
        .catch((e2: Error) => setBranchLogError(e2.message));
    },
    [root, expandedBranchLog]
  );

  // Real `git cherry-pick <oid>` -- applies a commit from a browsed
  // branch's own log onto the current branch. Refreshes overall status
  // (the working tree just changed) and, if the History section is
  // already open, its own commit list too, so the real new commit shows
  // up without needing a manual re-toggle.
  const cherryPick = useCallback(
    (oid: string) => {
      setCherryPickBusy(true);
      setCherryPickError(null);
      window.spartan
        .call("git_cherry_pick", { project_root: root, oid })
        .then(() => {
          refresh();
          if (showHistory) {
            return window.spartan.call("git_log", { project_root: root, max: 25 });
          }
          return null;
        })
        .then((result) => {
          if (result) setCommits((result as { commits: CommitInfo[] }).commits);
        })
        .catch((e: Error) => setCherryPickError(e.message))
        .finally(() => setCherryPickBusy(false));
    },
    [root, refresh, showHistory]
  );

  const createBranch = useCallback(() => {
    const name = newBranchName.trim();
    if (!name) return;
    setBranchError(null);
    window.spartan
      .call("git_create_branch", { project_root: root, branch: name })
      .then(() => {
        setNewBranchName("");
        // Re-fetch so the real new branch shows up immediately;
        // deliberately does NOT auto-switch, matching `git branch`'s own
        // real behavior (switching stays one explicit click away).
        return window.spartan.call("git_branches", { project_root: root });
      })
      .then((result) => setBranches((result as { branches: BranchInfo[] }).branches))
      .catch((e: Error) => setBranchError(e.message));
  }, [root, newBranchName]);

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
    window.spartan
      .call("git_log", { project_root: root, max: 25 })
      .then((result) => setCommits((result as { commits: CommitInfo[] }).commits))
      .catch((e: Error) => setHistoryError(e.message));
  }, [root, showHistory]);

  const toggleCommit = useCallback(
    (oid: string) => {
      // Collapsing the open commit; also clears any drilled-into file.
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
      window.spartan
        .call("git_commit_files", { project_root: root, oid })
        .then((result) => setCommitFiles((result as { files: ChangedFile[] }).files))
        .catch((e: Error) => setCommitFilesError(e.message));
    },
    [root, expandedCommit]
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
      window.spartan
        .call("git_commit_diff", { project_root: root, oid, path })
        .then((result) => setCommitFileDiff((result as { diff: string }).diff))
        .catch((e: Error) => setCommitFileDiffError(e.message))
        .finally(() => setCommitFileDiffLoading(false));
    },
    [root, expandedCommitFile]
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
            <React.Fragment key={b.name}>
              <div
                className="git-row"
                onClick={() => {
                  if (!b.current && !switching) checkoutBranch(b.name);
                }}
                title={b.current ? "Current branch" : `Switch to ${b.name}`}
              >
                <span className="git-status-glyph mono">{b.current ? "✓" : ""}</span>
                <span className="mono git-row-path">{b.name}</span>
                {!b.current && (
                  <>
                    <button
                      type="button"
                      className="editor-find-btn"
                      disabled={mergeBusy || Boolean(mergeStatus?.inProgress)}
                      onClick={(e) => mergeBranch(b.name, e)}
                      title={`Merge ${b.name} into ${status.branch ?? "the current branch"}`}
                    >
                      Merge
                    </button>
                    <button
                      type="button"
                      className="editor-find-btn"
                      onClick={(e) => toggleBranchLog(b.name, e)}
                      title={`Browse ${b.name}'s own commits`}
                    >
                      {expandedBranchLog === b.name ? "▾" : "▸"} Commits
                    </button>
                  </>
                )}
              </div>
              {expandedBranchLog === b.name && (
                <div className="git-hunk-block">
                  {branchLogError && (
                    <div className="git-panel-empty mono">{branchLogError}</div>
                  )}
                  {branchLogCommits === null && !branchLogError && (
                    <div className="git-panel-empty mono">Loading commits…</div>
                  )}
                  {branchLogCommits?.map((c) => (
                    <div key={c.oid} className="git-hunk-header mono">
                      <span>
                        {c.oid.slice(0, 7)} {c.summary}
                      </span>
                      <button
                        type="button"
                        className="editor-find-btn"
                        disabled={cherryPickBusy}
                        onClick={() => cherryPick(c.oid)}
                        title={`Cherry-pick onto ${status.branch ?? "the current branch"}`}
                      >
                        Cherry-pick
                      </button>
                    </div>
                  ))}
                  {cherryPickError && (
                    <div className="git-panel-empty mono">{cherryPickError}</div>
                  )}
                </div>
              )}
            </React.Fragment>
          ))}
          {remoteBranches && remoteBranches.length > 0 && (
            <>
              <div className="git-section-label mono">Remote branches</div>
              {remoteBranches.map((rb) => (
                <React.Fragment key={rb}>
                  <div
                    className="git-row"
                    onClick={() => {
                      if (!switching) checkoutRemoteBranch(rb);
                    }}
                    title={`Check out ${rb} (creates a local tracking branch)`}
                  >
                    <span className="git-status-glyph mono">⑃</span>
                    <span className="mono git-row-path">{rb}</span>
                    <button
                      type="button"
                      className="editor-find-btn"
                      onClick={(e) => toggleBranchLog(rb, e)}
                      title={`Browse ${rb}'s own commits`}
                    >
                      {expandedBranchLog === rb ? "▾" : "▸"} Commits
                    </button>
                  </div>
                  {expandedBranchLog === rb && (
                    <div className="git-hunk-block">
                      {branchLogError && (
                        <div className="git-panel-empty mono">{branchLogError}</div>
                      )}
                      {branchLogCommits === null && !branchLogError && (
                        <div className="git-panel-empty mono">Loading commits…</div>
                      )}
                      {branchLogCommits?.map((c) => (
                        <div key={c.oid} className="git-hunk-header mono">
                          <span>
                            {c.oid.slice(0, 7)} {c.summary}
                          </span>
                          <button
                            type="button"
                            className="editor-find-btn"
                            disabled={cherryPickBusy}
                            onClick={() => cherryPick(c.oid)}
                            title={`Cherry-pick onto ${status.branch ?? "the current branch"}`}
                          >
                            Cherry-pick
                          </button>
                        </div>
                      ))}
                      {cherryPickError && (
                        <div className="git-panel-empty mono">{cherryPickError}</div>
                      )}
                    </div>
                  )}
                </React.Fragment>
              ))}
            </>
          )}
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

      {mergeStatus?.inProgress && (
        <div className="git-section git-merge-conflict-panel">
          <div className="git-section-label mono">
            Merge in progress — {mergeStatus.conflicts.length} conflict
            {mergeStatus.conflicts.length === 1 ? "" : "s"}
          </div>
          {mergeError && <div className="git-panel-empty mono">{mergeError}</div>}
          {mergeStatus.conflicts.map((c) => (
            <div key={c.path} className="git-merge-conflict-entry">
              <div className="mono git-row-path">{c.path}</div>
              <div style={{ display: "flex", gap: 4 }}>
                <button
                  type="button"
                  className="editor-find-btn"
                  disabled={mergeBusy || c.ours === null}
                  onClick={() => resolveConflictWith(c.path, c.ours ?? "")}
                  title="Keep our (current branch's) version"
                >
                  Take ours
                </button>
                <button
                  type="button"
                  className="editor-find-btn"
                  disabled={mergeBusy || c.theirs === null}
                  onClick={() => resolveConflictWith(c.path, c.theirs ?? "")}
                  title="Keep their (merged branch's) version"
                >
                  Take theirs
                </button>
              </div>
              <textarea
                className="git-commit-input mono git-merge-conflict-textarea"
                placeholder="Manually edit the resolved content…"
                value={conflictEdits[c.path] ?? c.ours ?? c.theirs ?? ""}
                onChange={(e) =>
                  setConflictEdits((prev) => ({ ...prev, [c.path]: e.target.value }))
                }
              />
              <button
                type="button"
                className="editor-find-btn"
                disabled={mergeBusy}
                onClick={() =>
                  resolveConflictWith(c.path, conflictEdits[c.path] ?? c.ours ?? c.theirs ?? "")
                }
              >
                Resolve with this content
              </button>
            </div>
          ))}
          <div style={{ display: "flex", gap: 4, marginTop: 4 }}>
            <input
              className="git-commit-input mono"
              placeholder="Merge commit message"
              value={mergeCommitMessage}
              onChange={(e) => setMergeCommitMessage(e.target.value)}
              style={{ flex: 1, minHeight: "auto", height: 28 }}
            />
            <button
              type="button"
              className="git-commit-button"
              disabled={mergeBusy || mergeStatus.conflicts.length > 0}
              onClick={commitMerge}
              title={
                mergeStatus.conflicts.length > 0
                  ? "Resolve every conflict first"
                  : "Complete the merge with a real two-parent commit"
              }
            >
              Complete Merge
            </button>
            <button
              type="button"
              className="editor-find-btn"
              disabled={mergeBusy}
              onClick={abortMerge}
              title="Discard the merge and reset to HEAD"
            >
              Abort
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
      <div style={{ display: "flex", gap: 4 }}>
        <button
          className="git-commit-button"
          style={{ flex: 1 }}
          disabled={staged.length === 0 || !message.trim() || committing || amending}
          onClick={commit}
        >
          {committing ? "Committing…" : `Commit (${staged.length})`}
        </button>
        <button
          className="git-commit-button"
          type="button"
          title="Rewrite the last commit's message (and fold in staged changes)"
          disabled={!message.trim() || committing || amending}
          onClick={amend}
        >
          {amending ? "Amending…" : "Amend"}
        </button>
      </div>

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
          <input
            type="text"
            className="git-commit-input mono"
            style={{ flex: 2, minWidth: 0 }}
            placeholder="Stash message (optional)"
            value={stashMessage}
            disabled={stashBusy}
            onChange={(e) => setStashMessage(e.target.value)}
          />
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
              title="Apply and drop this stash"
            >
              Pop
            </button>
            <button
              type="button"
              className="editor-find-btn"
              disabled={stashBusy}
              onClick={() => stashAction("git_stash_apply", s.index)}
              title="Apply this stash but keep it in the list"
            >
              Apply
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

      {tags.length > 0 && (
        <div className="git-section">
          <div className="git-section-label mono">Tags ({tags.length})</div>
          {tags.map((t) => (
            <div key={t.name} className="git-row" style={{ cursor: "default" }}>
              <span className="mono git-row-path" title={t.target}>
                {t.annotated ? "🏷 " : ""}
                {t.name}
              </span>
              <span
                className="mono"
                style={{ opacity: 0.6, fontSize: 11, whiteSpace: "nowrap" }}
              >
                {t.target.slice(0, 7)}
              </span>
              <button
                type="button"
                className="editor-find-btn"
                onClick={() => deleteTag(t.name)}
                title="Delete this tag"
              >
                ✕
              </button>
            </div>
          ))}
        </div>
      )}

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
              <button
                type="button"
                className="editor-find-btn"
                title="Discard changes"
                onClick={(e) => {
                  e.stopPropagation();
                  discard(entry.path);
                }}
              >
                ⤺
              </button>
            </div>
            {expandedDiff?.path === entry.path && expandedDiff.staged === false && (
              <div onClick={(e) => e.stopPropagation()}>
                {diffLoading && <div className="git-panel-empty mono">Loading diff…</div>}
                {diffError && <div className="git-panel-empty mono">{diffError}</div>}
                {diffContent !== null && <DiffView diff={diffContent} />}
                {hunksError && <div className="git-panel-empty mono">{hunksError}</div>}
                {hunks?.map((h) => (
                  <div key={h.index} className="git-hunk-block">
                    <div className="git-hunk-header mono">
                      <span>{h.header}</span>
                      <button
                        type="button"
                        className="editor-find-btn"
                        disabled={hunkBusy}
                        onClick={() => stageHunk(entry.path, h.index)}
                        title="Stage only this hunk"
                      >
                        Stage this hunk
                      </button>
                    </div>
                    <DiffView diff={h.body} />
                  </div>
                ))}
              </div>
            )}
          </React.Fragment>
        ))}
      </div>

      {status.entries.length === 0 && (
        <div className="git-panel-empty mono">No changes.</div>
      )}

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
                <button
                  type="button"
                  className="editor-find-btn"
                  title="Tag this commit"
                  onClick={(e) => tagCommit(c.oid, e)}
                >
                  🏷
                </button>
                <button
                  type="button"
                  className="editor-find-btn"
                  title="Revert this commit (adds a new commit undoing it)"
                  onClick={(e) => revert(c.oid, c.summary, e)}
                >
                  ⟲
                </button>
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
