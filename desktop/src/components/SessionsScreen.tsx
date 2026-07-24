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
 * Real concurrent multi-session monitoring (closing the "one active PTY
 * at a time" gap this component itself named since §75.64): a provider
 * is added to `mounted` the first time its tab is clicked (a real,
 * deliberate lazy-spawn choice -- opening this screen never spawns three
 * real CLI processes on its own) and, once mounted, its `TerminalView`
 * -- and the real PTY session underneath it -- stays alive across every
 * later tab switch, hidden via CSS rather than unmounted. Output from an
 * inactive session keeps arriving and is written into its own real
 * xterm.js scrollback the whole time; switching back shows it exactly
 * as it would have looked had the tab been visible the entire time.
 */
export default function SessionsScreen({ root }: SessionsScreenProps): React.ReactElement {
  const [active, setActive] = useState<Provider>("claude");
  const [mounted, setMounted] = useState<Set<Provider>>(() => new Set(["claude"]));

  const activate = (p: Provider) => {
    setActive(p);
    setMounted((prev) => (prev.has(p) ? prev : new Set(prev).add(p)));
  };

  return (
    <div className="sessions-screen">
      <div className="sessions-tabs">
        {PROVIDERS.map((p) => (
          <div
            key={p}
            className={`sessions-tab ${p === active ? "sessions-tab-active" : ""}`}
            onClick={() => activate(p)}
          >
            {p}
            {mounted.has(p) && p !== active && <span className="sessions-tab-live-dot" />}
          </div>
        ))}
      </div>
      <div className="sessions-body">
        {PROVIDERS.filter((p) => mounted.has(p)).map((p) => (
          <div
            key={p}
            className="sessions-pty-slot"
            style={{ display: p === active ? "flex" : "none" }}
          >
            <TerminalView cwd={root} command={p} active={p === active} />
          </div>
        ))}
      </div>
    </div>
  );
}
