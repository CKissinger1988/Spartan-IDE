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
// macOS gets its own conventional app-name menu (About/Quit) per Electron's
// documented pattern, since Quit lives there by OS convention, not under
// File -- written for correctness but **not independently verified live**;
// this project has never had access to real Apple hardware (see
// `README.md`'s own standing "no macOS/iOS builds in project history" note).
// Everything else here was verified live on Linux via a real launched
// window, matching the exact real accelerators inventoried from every
// `onKeyDown`/`addEventListener("keydown", ...)` call site in `src/`.

import { app, BrowserWindow, Menu, dialog, shell } from "electron";
import type { MenuItemConstructorOptions } from "electron";

export const REPO_URL = "https://github.com/CKissinger1988/Spartan-IDE";

function focusedOrMainWindow(mainWindow: BrowserWindow | null): BrowserWindow | null {
  return BrowserWindow.getFocusedWindow() ?? mainWindow;
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
          const result = await dialog.showOpenDialog(win, {
            properties: ["openDirectory", "createDirectory"],
          });
          if (result.canceled || !result.filePaths[0]) return;
          loadRootIntoWindow(win, result.filePaths[0]);
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
        click: () => focusedOrMainWindow(getMainWindow())?.webContents.reload(),
      },
      {
        label: "Force Reload",
        click: () =>
          focusedOrMainWindow(getMainWindow())?.webContents.reloadIgnoringCache(),
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
        click: () => shell.openExternal(REPO_URL),
      },
      {
        label: "Report an Issue",
        click: () => shell.openExternal(`${REPO_URL}/issues/new`),
      },
      { type: "separator" },
      {
        label: "About Spartan IDE",
        click: () => {
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
          if (win) {
            void dialog.showMessageBox(win, options);
          } else {
            void dialog.showMessageBox(options);
          }
        },
      },
    ],
  };

  const template: MenuItemConstructorOptions[] = isMac
    ? [
        {
          label: app.name,
          submenu: [
            { role: "about" },
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
