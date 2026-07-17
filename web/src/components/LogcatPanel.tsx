import React, { useEffect, useRef } from "react";

interface LogcatPanelProps {
  visible: boolean;
  running: boolean;
  lines: string[];
  onStart: () => void;
  onStop: () => void;
  onClose: () => void;
}

/**
 * Real, compact `adb logcat` viewer (task #151, the `web/` sibling of
 * `desktop/`'s own `LogcatPanel.tsx`, task #150) -- byte-identical to
 * that copy, not shared code since the two shells don't share a
 * components package. A real, live stream of `adb logcat`'s own raw
 * output, not filtered or colorized (a real, named v1 scope cut).
 *
 * With no real device attached, `adb logcat` doesn't fail -- it blocks
 * on a real, honest `"- waiting for device -"` line, which shows up here
 * exactly as adb itself prints it, not specially handled.
 */
export default function LogcatPanel({
  visible,
  running,
  lines,
  onStart,
  onStop,
  onClose,
}: LogcatPanelProps): React.ReactElement | null {
  const bodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (bodyRef.current) {
      bodyRef.current.scrollTop = bodyRef.current.scrollHeight;
    }
  }, [lines]);

  if (!visible) return null;

  return (
    <div className="debug-panel mono logcat-panel">
      <div className="debug-toolbar">
        {!running ? (
          <button className="debug-btn debug-btn-primary" onClick={onStart} title="Start adb logcat">
            ▶ Start Logcat
          </button>
        ) : (
          <button className="debug-btn debug-btn-stop" onClick={onStop} title="Stop">
            ⏹ Stop
          </button>
        )}
        <span className="debug-status">{running ? "Streaming…" : "Stopped"}</span>
        <button className="debug-btn logcat-close" onClick={onClose} title="Close">
          ✕
        </button>
      </div>
      <div className="logcat-body" ref={bodyRef}>
        {lines.length === 0 ? (
          <span className="logcat-empty">No output yet.</span>
        ) : (
          lines.map((line, i) => (
            <div key={i} className="logcat-line">
              {line}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
