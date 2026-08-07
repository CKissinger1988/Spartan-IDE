import React from "react";
import { NAV, type ScreenId } from "../nav";

interface SidebarProps {
  active: ScreenId;
  onSelect: (id: ScreenId) => void;
  collapsed: boolean;
  onToggle: () => void;
}

export default function Sidebar({ active, onSelect, collapsed, onToggle }: SidebarProps): React.ReactElement {
  return (
    <aside className={`nav-sidebar ${collapsed ? "nav-sidebar-collapsed" : ""}`} aria-label="Main navigation">
      <div className="nav-brand mono sf-scanline">
        <span className="nav-brand-glyph" aria-hidden="true" />
        <span className="nav-brand-text">SPARTAN</span>
        <button className="nav-collapse-button" type="button" onClick={onToggle} title={collapsed ? "Expand navigation" : "Collapse navigation"} aria-label={collapsed ? "Expand navigation" : "Collapse navigation"}>
          {collapsed ? ">" : "<"}
        </button>
      </div>
      {NAV.map((group) => (
        <div className="nav-group" key={group.label}>
          <div className="nav-group-label">{group.label}</div>
          {group.items.map((item) => (
            <div
              key={item.id}
              className={`nav-item ${active === item.id ? "nav-item-active" : ""}`}
              title={item.label}
              onClick={() => onSelect(item.id)}
            >
              <span className="nav-item-icon" aria-hidden="true">{item.icon}</span>
              {item.label}
            </div>
          ))}
        </div>
      ))}
    </aside>
  );
}
