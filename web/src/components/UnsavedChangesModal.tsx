import React, { useEffect } from "react";

/**
 * Real unsaved-changes confirmation modal, a byte-identical port of
 * `desktop/src/components/UnsavedChangesModal.tsx`: closing a dirty tab
 * previously discarded unsaved edits with no confirmation. Mirrors the wgpu
 * reference shell's own already-verified behavior (§75.23): the modal names
 * the affected file(s) and asks for a real decision before anything is
 * discarded.
 *
 * Deliberately offers only Discard/Cancel -- matching the wgpu shell's own
 * proven shape -- not a "Save" button. The user's real save affordance is
 * the existing per-tab Ctrl/Cmd+S in the editor (backend `save_file` or the
 * File System Access API, depending on the tab's kind); a second, parallel
 * save path in a modal would mean reimplementing both save implementations
 * outside the editors, for marginal value. Safer default wins: Cancel is
 * auto-focused (so Enter cancels) and Escape also cancels -- a destructive
 * confirmation should never have the destructive action as its accidental
 * default.
 *
 * Browser close/reload is NOT gated by this modal -- a page going away
 * can't reliably render React; that path uses the native `beforeunload`
 * prompt instead (see `App.tsx`).
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
