// Real native application menu (File/View/Window/Help), closing the
// `docs/FUTURE_FEATURES.md`-named "Native application menu" gap that §240
// deliberately deferred over a real Edit-accelerator conflict risk.
//
// That risk turned out to be *already present*, not merely hypothetical:
// before this file existed, `main.ts` never called
// `Menu.setApplicationMenu()`, so Electron auto-created its own implicit
// default menu -- and that default menu's Edit submenu (Undo Ctrl+Z, Redo
// Ctrl+Shift+Z, Cut Ctrl+X, Copy Ctrl+C, Paste Ctrl+V, Select All Ctrl+A)
// registers the *exact same accelerators* `Editor.tsx`'s own JS keydown
// handler already claims -- confirmed live, not assumed: launching the
// real Electron window and clicking "Edit" showed those five items with
// those exact keys, side by side with the app's own real, backend-routed
// undo/redo/clipboard keybindings.
//
// The fix is structural, not defensive: calling `Menu.setApplicationMenu`
// with *any* real template replaces Electron's implicit default menu
// entirely -- so this menu deliberately has **no Edit menu at all**. Every
// real editing operation this app supports (undo/redo/cut/copy/paste/find/
// replace/etc.) is already fully keyboard-accessible through the editor's
// own real keybindings; a redundant native Edit menu would only reintroduce
// the exact conflict this file exists to remove, for zero real discoverability
// gain (nobody currently reaches Undo via a menu click -- they press Ctrl+Z,
// which only behaves correctly once the implicit default menu is gone).
//
// A second, real, deliberate safety choice: two items that can silently
// discard in-memory state -- "Open Folder..." (reloads the whole renderer
// at a new root) and "Reload"/"Force Reload" (the standard Electron
// role-based reload, which discards every open tab/unsaved edit with no
// warning) -- are given **no accelerator**, click-only. This project's own
// unsaved-changes-on-close/switch gap is real and still open (see
// `docs/FUTURE_FEATURES.md`), so these actions are already silently
// destructive; the fix here is not compounding that with an easy-to-hit
// keybinding, not pretending the underlying gap is closed.
//
// All three of the above are also gated behind one shared confirmation
// dialog (`confirmDestructiveReplace`) that always warns before proceeding
// -- unconditional, not dirty-state-aware, since there's no cheap way to
// query the renderer's real dirty state from the main process yet. A full
// dirty-state IPC contract would need to cover every other real
// state-discarding path this app has (closing a tab, switching files),
// which is the same pre-existing unsaved-changes-on-close/switch gap named
// above -- real, separate, larger scope than this one menu file.
//
// Every async dialog/external-link action (`showOpenDialog`,
// `showMessageBox`, `shell.openExternal`) is wrapped so a real rejection
// (an interrupted native dialog, a failed launch) surfaces as a visible
// error dialog instead of an unhandled promise rejection.
//
// macOS gets its own conventional app-name menu (About/Quit) per Electron's
// documented pattern, since Quit lives there by OS convention, not under
// File. Its About command reuses the exact same `showAboutDialog` the
// non-mac Help menu uses (rather than the native `role: "about"`, which
// can't show this app's own version/Electron/Chromium/Node detail) --
// and the Help menu's own "About Spartan IDE" item is only added on
// non-mac platforms, so there is never more than one About command on
// any platform. Written for correctness but **not independently verified
// live**; this project has never had access to real Apple hardware (see
// `README.md`'s own standing "no macOS/iOS builds in project history"
// note). Everything else here was verified live on Linux via a real
// launched window, matching the exact real accelerators inventoried from
// every `onKeyDown`/`addEventListener("keydown", ...)` call site in `src/`.

import { app, BrowserWindow, Menu, dialog, shell } from "electron";
import type { MenuItemConstructorOptions } from "electron";

export const REPO_URL = "https://github.com/Spartan-Software-Enterprises/Spartan-IDE";

function focusedOrMainWindow(mainWindow: BrowserWindow | null): BrowserWindow | null {
  return BrowserWindow.getFocusedWindow() ?? mainWindow;
}

/**
 * Surfaces a real, unexpected menu-action failure (a rejected native dialog,
 * a failed external-link launch) as a visible error dialog instead of a
 * silent unhandled promise rejection.
 */
function reportMenuActionFailure(title: string, err: unknown): void {
  const message = err instanceof Error ? err.message : String(err);
  dialog.showErrorBox(title, message);
}

/**
 * Real confirmation gate for the three menu actions that replace or reload
 * the renderer, discarding any open tabs/unsaved edits with no chance to
 * save first. Deliberately a plain "are you sure" dialog, not a dirty-state
 * query -- see this file's own header comment for why the fuller fix is
 * real, separate, larger scope, not something to bolt onto a menu PR.
 * `defaultId: 1` makes Cancel the button Enter activates, the safer default
 * for a destructive confirmation.
 */
async function confirmDestructiveReplace(
  win: BrowserWindow,
  message: string
): Promise<boolean> {
  const result = await dialog.showMessageBox(win, {
    type: "warning",
    buttons: ["Continue", "Cancel"],
    defaultId: 1,
    cancelId: 1,
    message,
    detail: "Any unsaved changes in open tabs will be lost.",
  });
  return result.response === 0;
}

/**
 * The one real About dialog, shared by both the macOS app-name menu and the
 * non-mac Help menu -- see this file's own header comment on why it's never
 * rendered from both places on the same platform at once.
 */
function showAboutDialog(getMainWindow: () => BrowserWindow | null): void {
  const win = focusedOrMainWindow(getMainWindow());
  const detail = [
    `Version ${app.getVersion()}`,
    `Electron ${process.versions.electron}`,
    `Chromium ${process.versions.chrome}`,
    `Node ${process.versions.node}`,
  ].join("\n");
  const options: Electron.MessageBoxOptions = {
    type: "info",
    title: "About Spartan IDE",
    message: "Spartan IDE",
    detail,
    buttons: ["OK"],
  };
  const promise = win ? dialog.showMessageBox(win, options) : dialog.showMessageBox(options);
  promise.catch((err) => reportMenuActionFailure("Could not show the About dialog", err));
}

/**
 * Builds the real application menu. `getMainWindow` is a getter (not a
 * captured value) because `main.ts`'s own `mainWindow` binding is
 * reassigned after the menu is built once at startup -- a captured
 * snapshot would go stale the moment a window closes and a new one opens.
 * `loadRootIntoWindow` is passed in rather than duplicated here so
 * "Open Folder..." reuses the exact same real root-loading logic
 * `spartan:open_project`'s IPC handler and the New Project wizard both
 * already depend on -- one real implementation, not two to keep in sync.
 */
export function buildApplicationMenu(
  getMainWindow: () => BrowserWindow | null,
  loadRootIntoWindow: (win: BrowserWindow, rootDir: string) => void
): Menu {
  const isMac = process.platform === "darwin";

  const fileMenu: MenuItemConstructorOptions = {
    label: "File",
    submenu: [
      {
        label: "Open Folder...",
        // Deliberately no accelerator -- see this file's own header
        // comment on why a silently-state-discarding action shouldn't
        // also be one accidental keystroke away.
        click: async () => {
          const win = focusedOrMainWindow(getMainWindow());
          if (!win) return;
          try {
            const result = await dialog.showOpenDialog(win, {
              properties: ["openDirectory", "createDirectory"],
            });
            if (result.canceled || !result.filePaths[0]) return;
            const confirmed = await confirmDestructiveReplace(
              win,
              "Open a different folder?"
            );
            if (!confirmed) return;
            loadRootIntoWindow(win, result.filePaths[0]);
          } catch (err) {
            reportMenuActionFailure("Could not open a different folder", err);
          }
        },
      },
      { type: "separator" },
      isMac ? { role: "close" } : { role: "quit" },
    ],
  };

  const viewMenu: MenuItemConstructorOptions = {
    label: "View",
    submenu: [
      {
        label: "Reload",
        // Deliberately no accelerator -- same reasoning as "Open Folder..."
        // above, and now gated behind the same confirmation dialog.
        click: async () => {
          const win = focusedOrMainWindow(getMainWindow());
          if (!win) return;
          try {
            const confirmed = await confirmDestructiveReplace(win, "Reload the app?");
            if (!confirmed) return;
            win.webContents.reload();
          } catch (err) {
            reportMenuActionFailure("Could not reload", err);
          }
        },
      },
      {
        label: "Force Reload",
        click: async () => {
          const win = focusedOrMainWindow(getMainWindow());
          if (!win) return;
          try {
            const confirmed = await confirmDestructiveReplace(
              win,
              "Force reload, ignoring the cache?"
            );
            if (!confirmed) return;
            win.webContents.reloadIgnoringCache();
          } catch (err) {
            reportMenuActionFailure("Could not force reload", err);
          }
        },
      },
      { type: "separator" },
      {
        label: "Toggle Developer Tools",
        accelerator: isMac ? "Cmd+Alt+I" : "Ctrl+Shift+I",
        click: () => focusedOrMainWindow(getMainWindow())?.webContents.toggleDevTools(),
      },
      { type: "separator" },
      { role: "togglefullscreen" },
    ],
  };

  const windowMenu: MenuItemConstructorOptions = {
    label: "Window",
    submenu: [{ role: "minimize" }, { role: "close" }],
  };

  const helpMenu: MenuItemConstructorOptions = {
    role: "help",
    submenu: [
      {
        label: "Documentation",
        click: () => {
          shell
            .openExternal(REPO_URL)
            .catch((err) => reportMenuActionFailure("Could not open the documentation link", err));
        },
      },
      {
        label: "Report an Issue",
        click: () => {
          shell
            .openExternal(`${REPO_URL}/issues/new`)
            .catch((err) => reportMenuActionFailure("Could not open the issue tracker", err));
        },
      },
      // On macOS, "About Spartan IDE" already lives in the app-name menu
      // below -- adding it here too would give macOS two About commands.
      ...(isMac
        ? []
        : ([
            { type: "separator" },
            {
              label: "About Spartan IDE",
              click: () => showAboutDialog(getMainWindow),
            },
          ] as MenuItemConstructorOptions[])),
    ],
  };

  const template: MenuItemConstructorOptions[] = isMac
    ? [
        {
          label: app.name,
          submenu: [
            // The real custom About dialog (with actual version/Electron/
            // Chromium/Node detail), not `role: "about"` -- that role's
            // native dialog can't show this app's own detail text.
            { label: "About Spartan IDE", click: () => showAboutDialog(getMainWindow) },
            { type: "separator" },
            { role: "services" },
            { type: "separator" },
            { role: "hide" },
            { role: "hideOthers" },
            { role: "unhide" },
            { type: "separator" },
            { role: "quit" },
          ],
        },
        fileMenu,
        viewMenu,
        windowMenu,
        helpMenu,
      ]
    : [fileMenu, viewMenu, windowMenu, helpMenu];

  return Menu.buildFromTemplate(template);
}
