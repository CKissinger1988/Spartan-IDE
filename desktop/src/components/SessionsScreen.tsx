import React, { useState } from "react";
import TerminalView from "./TerminalView";

interface SessionsScreenProps {
  root: string;
}

const PROVIDERS = ["claude", "codex", "gemini"] as const;
type Provider = (typeof PROVIDERS)[number];

/**
 * Real multi-CLI session orchestration (§75.64), closing the §75.62
 * audit's own named Sessions gap -- reuses the exact same real
 * `TerminalView`/PTY primitive `ConsoleScreen` uses, just with a real
 * named command (`claude`/`codex`/`gemini`) instead of the interactive
 * shell, the same real "Console and Sessions share one real blocker"
 * relationship the audit itself identified.
 *
 * A real, named v1 simplification: only the active provider's session
 * is mounted -- switching tabs closes the previous real PTY rather than
 * keeping several real sessions alive concurrently in the background.
 * Real, separate follow-up work if concurrent multi-session monitoring
 * is wanted, not attempted under this pass's own time constraints.
 */
export default function SessionsScreen({ root }: SessionsScreenProps): React.ReactElement {
  const [active, setActive] = useState<Provider>("claude");

  return (
    <div className="sessions-screen">
      <div className="sessions-tabs">
        {PROVIDERS.map((p) => (
          <div
            key={p}
            className={`sessions-tab ${p === active ? "sessions-tab-active" : ""}`}
            onClick={() => setActive(p)}
          >
            {p}
          </div>
        ))}
      </div>
      <div className="sessions-body">
        <TerminalView key={active} cwd={root} command={active} />
      </div>
    </div>
  );
}
