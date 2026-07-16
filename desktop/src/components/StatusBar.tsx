import React from "react";
import type { LspDiagnostic } from "./Editor";

interface StatusBarProps {
  fileCount: number;
  activePath: string | null;
  /** Real, live LSP diagnostics for the active file, if any -- undefined
   * (not an empty array) means "no LSP session for this file at all"
   * (no server configured, no project root found), rendered differently
   * from a real, genuinely clean file (an empty array). */
  diagnostics?: LspDiagnostic[];
}

export default function StatusBar({
  fileCount,
  activePath,
  diagnostics,
}: StatusBarProps): React.ReactElement {
  const ext = activePath?.split(".").pop() ?? "";
  const errorCount = diagnostics?.filter((d) => d.severity === "error").length ?? 0;
  const warningCount = diagnostics?.filter((d) => d.severity === "warning").length ?? 0;

  return (
    <div className="status-bar mono">
      <span>{activePath ? activePath.split("/").pop() : "No file"}</span>
      <span>{ext}</span>
      <span>
        {fileCount} file{fileCount === 1 ? "" : "s"}
      </span>
      {diagnostics !== undefined && (
        <span
          className="status-lsp-summary"
          title={`${errorCount} error${errorCount === 1 ? "" : "s"}, ${warningCount} warning${
            warningCount === 1 ? "" : "s"
          }`}
        >
          {errorCount > 0 && <span className="status-lsp-errors">⛔ {errorCount}</span>}
          {warningCount > 0 && <span className="status-lsp-warnings">⚠ {warningCount}</span>}
          {errorCount === 0 && warningCount === 0 && <span className="status-lsp-clean">✓ LSP</span>}
        </span>
      )}
    </div>
  );
}
