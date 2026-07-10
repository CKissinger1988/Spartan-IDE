import React from "react";

export const MODES = ["Agent", "Editor", "Design", "Term", "Flow"] as const;
export type Mode = (typeof MODES)[number];

interface ModeToggleProps {
  mode: Mode;
  onChange: (mode: Mode) => void;
}

export default function ModeToggle({ mode, onChange }: ModeToggleProps): React.ReactElement {
  return (
    <div className="mode-toggle">
      {MODES.map((m, i) => (
        <React.Fragment key={m}>
          {i > 0 && <span className="mode-sep">|</span>}
          <span
            className={`mode-label ${m === mode ? "mode-active" : ""}`}
            onClick={() => onChange(m)}
          >
            {m}
          </span>
        </React.Fragment>
      ))}
    </div>
  );
}
