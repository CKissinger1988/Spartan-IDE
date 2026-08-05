import React, { useCallback, useState } from "react";
import NewProjectWizard from "./NewProjectWizard";

interface OnboardingScreenProps {
  currentRoot: string;
  onDone: () => void;
}

interface FeatureHighlight {
  title: string;
  body: string;
}

/** Real, specific descriptions of what this app actually does today --
 * matching this project's own standing discipline against simulated or
 * aspirational copy in user-facing text. */
const FEATURES: FeatureHighlight[] = [
  {
    title: "Leo",
    body: "A real agentic coding assistant, docked on the right in every screen. Plan → approve → execute → verify: Leo proposes one real tool call at a time (read, search, edit, run) and every destructive one needs your explicit click before it touches a file.",
  },
  {
    title: "Editor",
    body: "A real, hand-built rope-buffer text editor with syntax highlighting, a real Git panel, and a real integrated terminal — no Monaco or CodeMirror code anywhere in this app.",
  },
  {
    title: "Dev Containers",
    body: "Detect a project's devcontainer.json and run it in a real, isolated Docker container — test a different OS/toolchain without touching your host machine.",
  },
  {
    title: "Workflows",
    body: "A real node-graph canvas for launching and monitoring external coding CLIs (Claude, Codex, Gemini) side by side.",
  },
  {
    title: "Updates",
    body: "Spartan checks its official GitHub Releases. The desktop installer uses the platform updater; downloads and restarts require your explicit approval, and Settings always shows the current update state.",
  },
];

/**
 * Real §75.76 first-run onboarding, user-requested ("IDE first run
 * onboarding"). Gated by the real, persisted `onboarding_completed`
 * settings flag (§75.76's own `spartan-settings` addition) -- shown
 * once, until the user picks one of the three real exits below, each of
 * which marks it complete via a real `settings_set` call before
 * proceeding so a reload never shows this again.
 */
export default function OnboardingScreen({
  currentRoot,
  onDone,
}: OnboardingScreenProps): React.ReactElement {
  const [showWizard, setShowWizard] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const markComplete = useCallback(() => {
    // Real, deliberate read-before-write: `settings_set`'s own real
    // `gpu_enabled` param is mandatory, not optional (§75.48's original
    // shape) -- hardcoding it here would silently clobber a real,
    // already-chosen GPU setting the very first time this app runs.
    return window.spartan.call("settings_get", {}).then((current) => {
      const c = current as { gpu_offload: { enabled: boolean; layers: number | null } };
      return window.spartan.call("settings_set", {
        gpu_enabled: c.gpu_offload.enabled,
        gpu_layers: c.gpu_offload.layers ?? undefined,
        onboarding_completed: true,
      });
    });
  }, []);

  const skip = useCallback(() => {
    setBusy(true);
    markComplete()
      .then(onDone)
      .catch((e: Error) => {
        setError(e.message);
        setBusy(false);
      });
  }, [markComplete, onDone]);

  const openExisting = useCallback(() => {
    window.spartan
      .pickFolder()
      .then((result) => {
        const r = result as { canceled: boolean; path: string | null };
        if (r.canceled || !r.path) return;
        setBusy(true);
        return markComplete().then(() => window.spartan.openProject(r.path as string));
      })
      .catch((e: Error) => {
        setError(e.message);
        setBusy(false);
      });
  }, [markComplete]);

  if (showWizard) {
    return (
      <NewProjectWizard
        defaultParentDir={currentRoot}
        onClose={() => {
          // Whether they created a project or cancelled, onboarding
          // itself is done either way -- a cancelled wizard shouldn't
          // trap the user in a loop back to this screen.
          markComplete()
            .then(() => setShowWizard(false))
            .then(onDone)
            .catch((e: Error) => setError(e.message));
        }}
        onCreated={(root) =>
          // Real bug fix: this used to never run at all -- the wizard's
          // old success path called `openProject` directly, reloading
          // the window before `markComplete()` (below) ever got a
          // chance to persist `onboarding_completed: true`, so a user
          // who created their first project right from onboarding would
          // see onboarding again on every future launch. Matches
          // `openExisting`'s own already-correct sequencing: persist
          // completion first, then navigate. Deliberately returns the
          // real promise chain (not caught here) so `NewProjectWizard`
          // itself can surface a real failure and offer a real retry --
          // catching and swallowing it here was the exact second bug an
          // earlier version of this fix introduced.
          markComplete().then(() => window.spartan.openProject(root).then(() => {}))
        }
      />
    );
  }

  return (
    <div className="onboarding-screen">
      <div className="onboarding-panel sf-chamfer">
        <div className="onboarding-brand mono">
          <span className="nav-brand-glyph" aria-hidden="true" />
          SPARTAN IDE
        </div>
        <div className="onboarding-tagline mono">
          A from-scratch, agent-first desktop IDE. Real Rust core, real Leo agent, no forked
          editor.
        </div>
        <div className="onboarding-features">
          {FEATURES.map((f) => (
            <div className="onboarding-feature" key={f.title}>
              <div className="onboarding-feature-title mono">{f.title}</div>
              <div className="onboarding-feature-body mono">{f.body}</div>
            </div>
          ))}
        </div>
        {error && <div className="leo-error mono">{error}</div>}
        <div className="onboarding-actions">
          <button
            className="leo-btn leo-btn-approve sf-chamfer-sm"
            disabled={busy}
            onClick={() => setShowWizard(true)}
          >
            New Project
          </button>
          <button className="leo-btn leo-btn-reject" disabled={busy} onClick={openExisting}>
            Open Existing Folder
          </button>
          <button className="onboarding-skip mono" disabled={busy} onClick={skip}>
            Skip — start editing {currentRoot}
          </button>
        </div>
      </div>
    </div>
  );
}
