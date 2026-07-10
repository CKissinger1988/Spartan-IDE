import React from "react";

interface StatusBarProps {
  fileCount: number;
  activePath: string | null;
}

export default function StatusBar({ fileCount, activePath }: StatusBarProps): React.ReactElement {
  const ext = activePath?.split(".").pop() ?? "";
  return (
    <div className="status-bar mono">
      <span>{activePath ? activePath.split("/").pop() : "No file"}</span>
      <span>{ext}</span>
      <span>
        {fileCount} file{fileCount === 1 ? "" : "s"}
      </span>
    </div>
  );
}
