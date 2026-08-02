import React, { useEffect } from "react";

/**
 * Real unsaved-changes confirmation modal, closing the pre-existing,
 * repeatedly-named gap (the native app menu's own header comment and
 * `docs/FUTURE_FEATURES.md` both call it "still real and open"): closing a
 * dirty tab or the whole app previously discarded unsaved edits with no
 * confirmation of any kind. Mirrors the wgpu reference shell's own
 * already-verified behavior (§75.23): the modal names the affected file(s)
 * and asks for a real decision before anything is discarded.
 *
 * Deliberately offers only Discard/Cancel -- matching the wgpu shell's own
 * proven shape -- not a "Save" button. The user's real save affordance is
 * the existing per-tab Ctrl/Cmd+S in the editor; adding a second, parallel
 * save path in a modal would mean reimplementing both shells' save logic
 * (backend `save_file` vs. the web File System Access API) outside the
 * editors, for marginal value. Safer default wins: Cancel is auto-focused
 * (so Enter cancels) and Escape also cancels -- a destructive confirmation
 * should never have the destructive action as its accidental default.
 */
export default function UnsavedChangesModal({
  fileNames,
  onDiscard,
  onCancel,
}: {
  fileNames: string[];
  onDiscard: () => void;
  onCancel: () => void;
}): React.ReactElement {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onCancel]);

  return (
    <div className="np-overlay" onClick={onCancel}>
      <div className="np-panel sf-chamfer" onClick={(e) => e.stopPropagation()}>
        <div className="np-title mono">Unsaved Changes</div>
        <div className="uc-body mono">
          {fileNames.length === 1 ? (
            <p>
              <strong>{fileNames[0]}</strong> has unsaved changes.
            </p>
          ) : (
            <p>
              <strong>{fileNames.length} files</strong> have unsaved changes:
            </p>
          )}
          <ul className="uc-file-list">
            {fileNames.map((name) => (
              <li key={name}>{name}</li>
            ))}
          </ul>
          <p>Discard them and continue?</p>
        </div>
        <div className="np-actions">
          <button className="leo-btn leo-btn-approve sf-chamfer-sm" autoFocus onClick={onCancel}>
            Cancel
          </button>
          <button className="leo-btn leo-btn-reject" onClick={onDiscard}>
            Discard Changes
          </button>
        </div>
      </div>
    </div>
  );
}
