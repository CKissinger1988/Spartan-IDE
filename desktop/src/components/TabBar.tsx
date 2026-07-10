import React from "react";
import type { OpenFile } from "./Editor";

interface TabBarProps {
  files: OpenFile[];
  activeIndex: number;
  onSelect: (index: number) => void;
  onClose: (index: number) => void;
}

export default function TabBar({ files, activeIndex, onSelect, onClose }: TabBarProps): React.ReactElement {
  return (
    <div className="tab-bar">
      {files.map((file, i) => {
        const name = file.path.split("/").pop() ?? file.path;
        return (
          <div
            key={file.path}
            className={`tab ${i === activeIndex ? "tab-active" : ""}`}
            onClick={() => onSelect(i)}
          >
            <span className="mono">
              {name}
              {file.dirty ? " *" : ""}
            </span>
            <span
              className="tab-close"
              onClick={(e) => {
                e.stopPropagation();
                onClose(i);
              }}
            >
              ×
            </span>
          </div>
        );
      })}
    </div>
  );
}
