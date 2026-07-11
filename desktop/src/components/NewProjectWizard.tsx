import React, { useCallback, useState } from "react";

interface Template {
  id: string;
  label: string;
}

/** Real Tier 1 templates (§35.4's original six plus C#, §75.51) --
 * matches `spartan-languages`' own real registry ids exactly, and each
 * one's real backend scaffold (`spartan-backend::project_template_files`)
 * is confirmed, by test, to be detectable by that same registry. */
const TEMPLATES: Template[] = [
  { id: "rust", label: "Rust (Cargo)" },
  { id: "typescript", label: "TypeScript" },
  { id: "javascript", label: "JavaScript" },
  { id: "python", label: "Python" },
  { id: "kotlin", label: "Kotlin (Gradle)" },
  { id: "java", label: "Java (Maven)" },
  { id: "go", label: "Go" },
  { id: "csharp", label: "C# (.NET)" },
];

interface NewProjectWizardProps {
  defaultParentDir: string;
  onClose: () => void;
  /**
   * Real bug fix, found live by a code-review pass: this used to call
   * `window.spartan.openProject` itself the instant `create_project`
   * succeeded, entirely bypassing `onClose` -- harmless for the plain
   * `App.tsx` usage (closing the modal is optional once the window is
   * about to reload anyway), but silently broke `OnboardingScreen.tsx`,
   * whose *only* call to `markComplete()` (persisting
   * `onboarding_completed: true`) was wired through `onClose`. A user
   * who created their first project via onboarding's own "New Project"
   * button would see onboarding again on every future launch, since the
   * window reloaded before that write could ever happen. Now the parent
   * decides what "a project was created" means -- `App.tsx` just opens
   * it; `OnboardingScreen.tsx` persists completion *first*, then opens
   * it, matching what its `onClose`-based Skip/Open-Existing paths
   * already correctly did.
   *
   * Returns a real `Promise` (not fire-and-forget) so this component can
   * distinguish and surface a failure here -- a second real bug caught
   * before it shipped: an earlier version of this same fix made
   * `onCreated` void/fire-and-forget, which meant a real failure in the
   * caller's own post-creation step (e.g. `openProject` rejecting) was
   * silently swallowed with no error shown and no way to retry, strictly
   * worse than the original bug. Since `create_project` itself already
   * succeeded by the time `onCreated` runs, a real retry must not call
   * it again (its own real non-empty-directory guard would then refuse
   * the already-created folder) -- see `createdRoot` below.
   */
  onCreated: (projectRoot: string) => Promise<void>;
}

/**
 * Real §75.76 "New Project" quick-start wizard, user-requested
 * ("Create options for new project quick start"). Talks to
 * `spartan-backend`'s real `create_project` IPC method (a real,
 * synchronous scaffold-and-write-to-disk operation), then hands the new
 * project's real root to the caller via `onCreated` -- the caller
 * decides when/whether to call `window.spartan.openProject` (a real
 * main-process action that reloads this same window at the new
 * project's root, a real, deliberate "single window, switch what it's
 * pointed at" UX rather than opening a second window).
 *
 * The first real modal overlay in this Electron shell -- every prior
 * confirm/detail surface in this shell (the Git commit box, Leo's plan
 * card, Dev Containers' progress log) has been an inline panel, not a
 * dim-background overlay; this is the real, minimal first instance of
 * that pattern here, styled to match the rest of the Sci-Fi theme
 * (`sf-chamfer`, `sf-glow-accent`) rather than a plain browser dialog.
 */
export default function NewProjectWizard({
  defaultParentDir,
  onClose,
  onCreated,
}: NewProjectWizardProps): React.ReactElement {
  const [name, setName] = useState("");
  const [template, setTemplate] = useState("rust");
  const [parentDir, setParentDir] = useState(defaultParentDir);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Real, deliberate state: set only once `create_project` has actually
  // succeeded. Its presence is what lets `create()` below tell "never
  // attempted" apart from "the files exist on disk, only the navigation
  // step failed" -- the latter must retry `onCreated` alone, never
  // `create_project` again (which would now correctly, but unhelpfully,
  // refuse the real, already-created, non-empty directory).
  const [createdRoot, setCreatedRoot] = useState<string | null>(null);

  const create = useCallback(() => {
    if (createdRoot) {
      setCreating(true);
      setError(null);
      onCreated(createdRoot).catch((e: Error) => {
        setError(e.message);
        setCreating(false);
      });
      return;
    }
    if (!name.trim()) {
      setError("Give the project a name first.");
      return;
    }
    setCreating(true);
    setError(null);
    window.spartan
      .call("create_project", { parent_dir: parentDir, template, name })
      .then((result) => {
        const r = result as { project_root: string };
        setCreatedRoot(r.project_root);
        return onCreated(r.project_root);
      })
      .catch((e: Error) => {
        setError(e.message);
        setCreating(false);
      });
  }, [name, parentDir, template, onCreated, createdRoot]);

  return (
    <div className="np-overlay" onClick={onClose}>
      <div
        className="np-panel sf-chamfer"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="np-title mono">New Project</div>
        <div className="np-row">
          <label className="settings-label mono">Name</label>
          <input
            className="settings-select mono np-input"
            type="text"
            autoFocus
            disabled={creating || createdRoot !== null}
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="my-project"
          />
        </div>
        <div className="np-row">
          <label className="settings-label mono">Template</label>
          <select
            className="settings-select mono np-input"
            disabled={creating || createdRoot !== null}
            value={template}
            onChange={(e) => setTemplate(e.target.value)}
          >
            {TEMPLATES.map((t) => (
              <option key={t.id} value={t.id}>
                {t.label}
              </option>
            ))}
          </select>
        </div>
        <div className="np-row">
          <label className="settings-label mono">Create in</label>
          <div className="np-input-with-browse">
            <input
              className="settings-select mono np-input"
              type="text"
              disabled={creating || createdRoot !== null}
              value={parentDir}
              onChange={(e) => setParentDir(e.target.value)}
            />
            <button
              className="settings-button mono"
              type="button"
              disabled={creating || createdRoot !== null}
              onClick={() => {
                window.spartan
                  .pickFolder()
                  .then((result) => {
                    const r = result as { canceled: boolean; path: string | null };
                    if (!r.canceled && r.path) setParentDir(r.path);
                  })
                  .catch((e: Error) => setError(e.message));
              }}
            >
              Browse…
            </button>
          </div>
        </div>
        <div className="settings-note mono">
          {createdRoot ? (
            <>
              The project was created at <strong>{createdRoot}</strong>, but opening it here
              failed. Nothing further will be created — retrying only reopens it.
            </>
          ) : (
            <>
              Creates a real, runnable starter project (a real project manifest plus a real
              hello-world file) at {parentDir}/{name.trim() || "<name>"}, then opens it here.
            </>
          )}
        </div>
        {error && <div className="leo-error mono">{error}</div>}
        <div className="np-actions">
          <button className="leo-btn leo-btn-reject" disabled={creating} onClick={onClose}>
            Cancel
          </button>
          <button
            className="leo-btn leo-btn-approve sf-chamfer-sm"
            disabled={creating}
            onClick={create}
          >
            {creating ? "Creating…" : createdRoot ? "Retry Opening" : "Create Project"}
          </button>
        </div>
      </div>
    </div>
  );
}
