import React from "react";
import TerminalView from "./TerminalView";

interface ConsoleScreenProps {
  root: string;
}

/** Real integrated terminal (§75.64) -- the user's real `$SHELL` in a
 * real PTY, closing the §75.62 audit's own named Console gap. */
export default function ConsoleScreen({ root }: ConsoleScreenProps): React.ReactElement {
  return (
    <div className="console-screen">
      <TerminalView cwd={root} />
    </div>
  );
}
