import React from "react";
import { NAV, type ScreenId } from "../nav";

interface SidebarProps {
  active: ScreenId;
  onSelect: (id: ScreenId) => void;
}

export default function Sidebar({ active, onSelect }: SidebarProps): React.ReactElement {
  return (
    <div className="nav-sidebar">
      <div className="nav-brand mono sf-scanline">
        <span className="nav-brand-glyph" aria-hidden="true" />
        SPARTAN
      </div>
      {NAV.map((group) => (
        <div className="nav-group" key={group.label}>
          <div className="nav-group-label">{group.label}</div>
          {group.items.map((item) => (
            <div
              key={item.id}
              className={`nav-item ${active === item.id ? "nav-item-active" : ""}`}
              onClick={() => onSelect(item.id)}
            >
              {item.label}
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}
