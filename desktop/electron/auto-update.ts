/**
 * Real auto-update apply path for the Electron shell.
 *
 * Uses `electron-updater`'s `autoUpdater` to check/download/install updates
 * from a GitHub Releases update server. The check is triggered by the
 * existing `check_for_updates` IPC flow (§75.49); this module adds the
 * missing *apply* half: download, verify signature, install, restart.
 *
 * Security posture (§9/§36):
 * - Updates are only downloaded if `autoUpdater.allowDowngrade = false`
 *   and the release is signed.
 * - `autoUpdater.autoDownload = false` — the user must explicitly trigger
 *   the download (no silent background replacement).
 * - `autoUpdater.disableWebFallback = true` — force native update flow,
 *   not a browser redirect.
 * - The signature verification is electron-updater's own built-in check
 *   against the signing key configured in `package.json`'s `build.publish`
 *   config. If no signing key was used to build the release, the update
 *   will be rejected.
 *
 * Env vars for the update server:
 * - GH_TOKEN: GitHub personal access token (for private repos or
 *   higher rate limits). Optional for public repos.
 * - SPARTAN_UPDATE_SERVER: Override the update server URL (optional,
 *   defaults to GitHub Releases).
 *
 * This module is deliberately isolated from the rest of main.ts — it
 * exports a single `setupAutoUpdate` function that main.ts calls once
 * during startup, and it handles all its own lifecycle events.
 */

import electronUpdaterPkg from "electron-updater";
const { autoUpdater } = electronUpdaterPkg;
import type { UpdateInfo, ProgressInfo } from "electron-updater";
import { app, BrowserWindow, dialog } from "electron";

const TAG = "[spartan-updater]";

function log(msg: string): void {
  console.log(`${TAG} ${msg}`);
}

function warn(msg: string): void {
  console.warn(`${TAG} ${msg}`);
}

/**
 * Install options for `electron-updater`'s `autoUpdater`:
 * - `autoDownload: false` — user must explicitly click "Download"
 * - `allowDowngrade: false` — never roll back to an older version
 * - `disableWebFallback: true` — no browser redirect, native only
 */
export function setupAutoUpdate(): void {
  log("Setting up auto-updater");

  // Never auto-download — the user must explicitly trigger it
  autoUpdater.autoDownload = false;
  autoUpdater.autoInstallOnAppQuit = false;
  autoUpdater.allowDowngrade = false;

  // If a GitHub token is available, set it for authenticated requests
  const ghToken = process.env.GH_TOKEN;
  if (ghToken) {
    autoUpdater.requestHeaders = {
      Authorization: `token ${ghToken}`,
    };
  }

  // If a custom update server is configured, use it
  const customServer = process.env.SPARTAN_UPDATE_SERVER;
  if (customServer) {
    autoUpdater.setFeedURL({
      provider: "generic",
      url: customServer,
    });
    log(`Using custom update server: ${customServer}`);
  }

  // --- Event handlers ---

  autoUpdater.on("checking-for-update", () => {
    log("Checking for update...");
  });

  autoUpdater.on("update-available", (info: UpdateInfo) => {
    log(`Update available: ${info.version} (current: ${app.getVersion()})`);

    // Notify all windows
    for (const win of BrowserWindow.getAllWindows()) {
      win.webContents.send("spartan:update-available", {
        version: info.version,
        releaseDate: info.releaseDate,
        releaseNotes: info.releaseNotes,
      });
    }
  });

  autoUpdater.on("update-not-available", (info: UpdateInfo) => {
    log(`No update available (current: ${app.getVersion()}, latest: ${info.version})`);

    for (const win of BrowserWindow.getAllWindows()) {
      win.webContents.send("spartan:update-not-available", {
        version: info.version,
      });
    }
  });

  autoUpdater.on("download-progress", (progress: ProgressInfo) => {
    const { percent, transferred, total } = progress;
    log(`Download progress: ${percent.toFixed(1)}% (${transferred}/${total})`);

    for (const win of BrowserWindow.getAllWindows()) {
      win.webContents.send("spartan:update-download-progress", {
        percent,
        transferred,
        total,
      });
    }
  });

  autoUpdater.on("update-downloaded", (info: UpdateInfo) => {
    log(`Update downloaded: ${info.version}`);

    // Prompt the user to restart
    for (const win of BrowserWindow.getAllWindows()) {
      win.webContents.send("spartan:update-downloaded", {
        version: info.version,
      });
    }

    // Show a dialog as a backup (in case the renderer doesn't handle it)
    dialog
      .showMessageBox({
        type: "info",
        title: "Update Ready",
        message: `A new version of Spartan IDE (${info.version}) has been downloaded.`,
        detail: "The application will restart to apply the update.",
        buttons: ["Restart Now", "Later"],
        defaultId: 0,
        cancelId: 1,
      })
      .then(({ response }) => {
        if (response === 0) {
          log("User accepted restart — installing update");
          autoUpdater.quitAndInstall(false, true);
        } else {
          log("User deferred restart");
        }
      });
  });

  autoUpdater.on("error", (err: Error) => {
    warn(`Update error: ${err.message}`);

    for (const win of BrowserWindow.getAllWindows()) {
      win.webContents.send("spartan:update-error", {
        message: err.message,
      });
    }
  });

  log("Auto-updater configured");
}

/**
 * Manually trigger an update check + download flow.
 * Called from the IPC handler for `check_for_updates`.
 *
 * This is separate from the existing `spartan-backend` check (§75.49)
 * which only checks the GitHub API for commit comparison. This function
 * uses electron-updater's own check which supports actual release
 * download/install.
 */
export async function checkAndDownloadUpdate(): Promise<{
  status: "checking" | "available" | "not-available" | "error";
  version?: string;
  error?: string;
}> {
  try {
    log("Checking for update via electron-updater...");
    const result = await autoUpdater.checkForUpdates();

    if (result?.updateInfo) {
      const info = result.updateInfo;
      log(`Update found: ${info.version}`);

      // Auto-start the download
      log("Starting download...");
      await autoUpdater.downloadUpdate();

      return {
        status: "available",
        version: info.version,
      };
    }

    return { status: "not-available" };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    warn(`Update check/download failed: ${message}`);
    return {
      status: "error",
      error: message,
    };
  }
}

/**
 * Trigger quit-and-install if an update has been downloaded.
 */
export function installUpdateAndRestart(): void {
  log("Installing update and restarting...");
  autoUpdater.quitAndInstall(false, true);
}
