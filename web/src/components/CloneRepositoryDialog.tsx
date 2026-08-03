import React, { useCallback, useState } from "react";

interface CloneRepositoryDialogProps {
  defaultParentDir: string;
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
  /** Real native "choose a folder" dialog -- desktop provides it via
   * `window.spartan.pickFolder()`; the web shell has no native picker, so
   * the parent directory there is a plain text field. */
  pickFolder?: () => Promise<{ canceled: boolean; path: string | null }>;
  /** What "the clone succeeded" means. Desktop reloads this window at the
   * new project root via `window.spartan.openProject`; the web shell has
   * no root-switch mechanism (its devserver project root is fixed at
   * startup), so callers omit this and the dialog just reports the path. */
  onCreated?: (projectRoot: string) => Promise<void>;
  onClose: () => void;
}

/** Derives a default directory name from a git URL the way `git clone`'s
 * own default does: the last path segment, with a trailing `.git` and any
 * trailing slash stripped. Handles https, `ssh://`, and scp-style
 * `git@host:owner/repo.git` shapes. Falls back to "" when the URL is
 * unparsable, so the user fills a name in rather than guessing. */
function deriveNameFromUrl(url: string): string {
  const trimmed = url.trim().replace(/\/+$/, "");
  let path = trimmed;
  if (trimmed.startsWith("git@")) {
    const afterColon = trimmed.indexOf(":");
    if (afterColon >= 0) path = trimmed.slice(afterColon + 1);
  } else {
    path = trimmed.slice(trimmed.lastIndexOf("/") + 1);
  }
  path = path.slice(path.lastIndexOf("/") + 1);
  return path.replace(/\.git$/, "");
}

/**
 * Real "Clone repository" dialog (the `docs/FUTURE_FEATURES.md` follow-up
 * the remote push/pull/fetch pass named): enter a URL, a destination
 * directory name (derived from the URL when left blank), and a parent
 * directory, then clone through `spartan-backend`'s real `git_clone`
 * IPC method. Modeled on `NewProjectWizard.tsx` -- the same `np-*` overlay
 * chrome, the same real-error display, and the same "navigate after
 * creation" handoff via an optional `onCreated` (the caller decides what
 * "the clone succeeded" means; App.tsx/GitPanel just opens it).
 *
 * Clone is a real, potentially slow network operation, and it is
 * synchronous on the backend (like the other git dispatch methods), so the
 * button shows a real "Cloning…" state while the call is in flight; a real
 * failure (bad URL, auth, non-empty destination) surfaces the real error
 * verbatim.
 */
export default function CloneRepositoryDialog({
  defaultParentDir,
  call,
  pickFolder,
  onCreated,
  onClose,
}: CloneRepositoryDialogProps): React.ReactElement {
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [parentDir, setParentDir] = useState(defaultParentDir);
  const [cloning, setCloning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Real, deliberate state: set only once `git_clone` has actually
  // succeeded. Its presence is what lets `clone()` below tell "never
  // attempted" apart from "the repo exists on disk, only the navigation
  // step failed" -- the latter must retry `onCreated` alone, never
  // `git_clone` again (which would now correctly, but unhelpfully, refuse
  // the real, already-cloned, non-empty directory).
  const [clonedRoot, setClonedRoot] = useState<string | null>(null);
  const [clonedName, setClonedName] = useState("");

  const clone = useCallback(() => {
    if (clonedRoot) {
      if (!onCreated) {
        onClose();
        return;
      }
      setCloning(true);
      setError(null);
      onCreated(clonedRoot).catch((e: Error) => {
        setError(e.message);
        setCloning(false);
      });
      return;
    }
    const trimmedUrl = url.trim();
    if (!trimmedUrl) {
      setError("Enter a repository URL to clone.");
      return;
    }
    const targetName = name.trim() || deriveNameFromUrl(trimmedUrl);
    if (!targetName) {
      setError("Enter a directory name to clone into.");
      return;
    }
    setCloning(true);
    setError(null);
    call("git_clone", { parent_dir: parentDir, url: trimmedUrl, name: targetName })
      .then((result) => {
        const r = result as { project_root: string; name: string };
        setClonedRoot(r.project_root);
        setClonedName(r.name);
        if (onCreated) return onCreated(r.project_root);
      })
      .catch((e: Error) => {
        setError(e.message);
        setCloning(false);
      });
  }, [url, name, parentDir, call, onCreated, onClose, clonedRoot]);

  const defaultName = deriveNameFromUrl(url);

  return (
    <div className="np-overlay" onClick={onClose}>
      <div className="np-panel sf-chamfer" onClick={(e) => e.stopPropagation()}>
        <div className="np-title mono">Clone Repository</div>
        <div className="np-row">
          <label className="settings-label mono">URL</label>
          <input
            className="settings-select mono np-input"
            type="text"
            autoFocus
            disabled={cloning || clonedRoot !== null}
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="https://github.com/owner/repo.git"
          />
        </div>
        <div className="np-row">
          <label className="settings-label mono">Directory</label>
          <input
            className="settings-select mono np-input"
            type="text"
            disabled={cloning || clonedRoot !== null}
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={defaultName || "my-repo"}
          />
        </div>
        <div className="np-row">
          <label className="settings-label mono">Clone into</label>
          <div className="np-input-with-browse">
            <input
              className="settings-select mono np-input"
              type="text"
              disabled={cloning || clonedRoot !== null}
              value={parentDir}
              onChange={(e) => setParentDir(e.target.value)}
            />
            {pickFolder && (
              <button
                className="settings-button mono"
                type="button"
                disabled={cloning || clonedRoot !== null}
                onClick={() => {
                  pickFolder()
                    .then((result) => {
                      if (!result.canceled && result.path) setParentDir(result.path);
                    })
                    .catch((e: Error) => setError(e.message));
                }}
              >
                Browse…
              </button>
            )}
          </div>
        </div>
        <div className="settings-note mono">
          {clonedRoot ? (
            <>
              Cloned <strong>{clonedName}</strong> to <strong>{clonedRoot}</strong>
              {onCreated
                ? ", but opening it here failed. Nothing further will be cloned — retrying only reopens it."
                : ". The web shell's project root is fixed by its devserver, so open it there in a new session."}
            </>
          ) : (
            <>
              Clones the repository into {parentDir}/{name.trim() || defaultName || "<name>"}
              {onCreated ? ", then opens it here." : "."}
            </>
          )}
        </div>
        {error && <div className="leo-error mono">{error}</div>}
        <div className="np-actions">
          <button className="leo-btn leo-btn-reject" disabled={cloning} onClick={onClose}>
            {clonedRoot && !onCreated ? "Close" : "Cancel"}
          </button>
          <button
            className="leo-btn leo-btn-approve sf-chamfer-sm"
            disabled={cloning}
            onClick={clone}
          >
            {cloning
              ? "Cloning…"
              : clonedRoot && onCreated
                ? "Retry Opening"
                : clonedRoot && !onCreated
                  ? "Done"
                  : "Clone Repository"}
          </button>
        </div>
      </div>
    </div>
  );
}
