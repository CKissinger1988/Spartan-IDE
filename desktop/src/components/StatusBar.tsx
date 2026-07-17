import React from "react";
import type { LspDiagnostic } from "./Editor";

/** Real shape of `spartan-backend`'s `android_detect` method (§75.91) --
 * whether the open project root looks like an Android/Gradle project, plus
 * whatever real Android SDK/Gradle toolchain paths were found on `$PATH`/
 * `$ANDROID_HOME`, independent of that project check. */
export interface AndroidDetectResult {
  isAndroidProject: boolean;
  sdkRoot: string | null;
  adbPath: string | null;
  emulatorPath: string | null;
  sdkmanagerPath: string | null;
  avdmanagerPath: string | null;
  gradlePath: string | null;
  gradleVersion: string | null;
}

/** Real client-side state for the "Build APK" action (task #144) -- a
 * real `assembleDebug` gradle build streamed from `spartan-backend`'s own
 * `android_build_apk`/`android_build_progress`/`android_build_ready`/
 * `android_build_failed` events. */
export type AndroidBuildState =
  | { phase: "idle" }
  | { phase: "building"; lastLine?: string }
  | { phase: "ready"; apkPath: string }
  | { phase: "failed"; error: string };

interface StatusBarProps {
  fileCount: number;
  activePath: string | null;
  /** Real, live LSP diagnostics for the active file, if any -- undefined
   * (not an empty array) means "no LSP session for this file at all"
   * (no server configured, no project root found), rendered differently
   * from a real, genuinely clean file (an empty array). */
  diagnostics?: LspDiagnostic[];
  /** Real, one-shot `android_detect` result for the open project root, or
   * `null` before it resolves / on a real non-Android project. Only ever
   * renders a badge when `isAndroidProject` is true -- a non-Android
   * project (the common case) shows nothing extra. */
  androidInfo?: AndroidDetectResult | null;
  /** Real, live state of an in-progress/finished `assembleDebug` build --
   * `undefined` when no build has ever been triggered this session. */
  androidBuild?: AndroidBuildState;
  /** Clicking the Android badge triggers a real build -- a no-op prop
   * (not rendered as clickable) when omitted, matching this component's
   * own existing "no callback, no interactivity" convention elsewhere. */
  onBuildApk?: () => void;
}

export default function StatusBar({
  fileCount,
  activePath,
  diagnostics,
  androidInfo,
  androidBuild,
  onBuildApk,
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
      {androidInfo?.isAndroidProject && (
        <button
          className="status-android-badge"
          type="button"
          disabled={androidBuild?.phase === "building"}
          onClick={onBuildApk}
          title={`Gradle: ${androidInfo.gradlePath ?? "not found"}${
            androidInfo.gradleVersion ? ` (${androidInfo.gradleVersion})` : ""
          } | SDK: ${androidInfo.sdkRoot ?? "not found"} | adb: ${
            androidInfo.adbPath ?? "not found"
          }${
            androidBuild?.phase === "building" && androidBuild.lastLine
              ? `\n${androidBuild.lastLine}`
              : androidBuild?.phase === "ready"
                ? `\nBuilt: ${androidBuild.apkPath}`
                : androidBuild?.phase === "failed"
                  ? `\n${androidBuild.error}`
                  : "\nClick to build a real debug APK (gradle assembleDebug)."
          }`}
        >
          {androidBuild?.phase === "building"
            ? "🤖 Building…"
            : androidBuild?.phase === "ready"
              ? "🤖 ✓ built"
              : androidBuild?.phase === "failed"
                ? "🤖 ✗ failed"
                : "🤖 Android"}
        </button>
      )}
    </div>
  );
}
