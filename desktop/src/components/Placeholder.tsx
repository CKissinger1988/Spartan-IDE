import React from "react";
import { NAV, SCREEN_NOTES, type ScreenId } from "../nav";

interface PlaceholderProps {
  screen: ScreenId;
}

/** Real, honest per-screen placeholder -- names exactly what's real vs.
 * not yet wired into this shell, matching this project's own established
 * "name the gap, don't fake it" discipline rather than simulated content. */
export default function Placeholder({ screen }: PlaceholderProps): React.ReactElement {
  const label = NAV.flatMap((g) => g.items).find((i) => i.id === screen)?.label ?? screen;
  const note = SCREEN_NOTES[screen] ?? "Not yet built.";
  return (
    <div className="placeholder-screen">
      <h2 className="mono">{label}</h2>
      <p>{note}</p>
    </div>
  );
}
