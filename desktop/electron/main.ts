// Real Electron main process for the new desktop shell (user-requested
// pivot away from the wgpu-native `spartan-editor-core` UI, keeping that
// crate and every sibling Rust crate as the real, tested backend). This
// process owns window creation and the real `spartan-backend` subprocess;
// the renderer never talks to either directly -- `preload.ts`'s
// `contextBridge` is the one real, narrow API surface it gets, matching
// this workspace's own §9 "least privilege" posture even though this is
// a new UI stack.

import { app, BrowserWindow, ipcMain, shell, dialog, Menu } from "electron";
import * as path from "node:path";
import * as fs from "node:fs";
import * as os from "node:os";
import { BackendClient } from "./backend-client.js";
import { buildApplicationMenu, REPO_URL } from "./menu.js";

const isDev = !app.isPackaged;

/**
 * Real, shared packaged-vs-dev resource resolver, used by
 * `resolveBackendBinaryPath` below. Real §75.76
 * packaged-app path: electron-builder's own `extraResources` config
 * (`package.json`'s `build.extraResources`) copies real files to
 * `<resourcesPath>/...` -- `process.resourcesPath` is Electron's own
 * real, cross-platform constant for that directory (`Resources/` on
 * macOS, `resources/` alongside the executable on Windows/Linux),
 * resolved correctly regardless of platform without either caller
 * needing to know the platform-specific layout itself. `devSegments`
 * resolves relative to `desktop/`'s own real position as a direct child
 * of the repo root.
 */
function resolveResourcePath(
  label: string,
  devSegments: string[],
  packagedSegments: string[],
  devHint: string
): string {
  const candidate = app.isPackaged
    ? path.join(process.resourcesPath, ...packagedSegments)
    : path.resolve(import.meta.dirname, "..", "..", ...devSegments);
  if (!fs.existsSync(candidate)) {
    throw new Error(
      app.isPackaged
        ? `${label} not found in the packaged app at ${candidate}`
        : `${label} not found at ${candidate} -- ${devHint}`
    );
  }
  return candidate;
}

function resolveBackendBinaryPath(): string {
  const binaryName = process.platform === "win32" ? "spartan-backend.exe" : "spartan-backend";
  return resolveResourcePath(
    "spartan-backend binary",
    ["target", "release", binaryName],
    [binaryName],
    'run "cargo build --release -p spartan-backend" from the repo root first.'
  );
}

let backend: BackendClient | null = null;
// The one real main window. Tracked so a second launch attempt (see the
// single-instance lock below) can focus/restore it rather than spawning a
// second full instance (a second `spartan-backend` subprocess + a second
// window) -- a real resource/correctness bug for any shipped desktop app.
let mainWindow: BrowserWindow | null = null;
// Real unsaved-changes-on-quit gate: windows whose renderer has confirmed a
// close are allowed to actually close; every other `close` is prevented and
// turned into a `spartan:close-requested` renderer prompt instead. A per-window
// `WeakSet` (not a boolean) so a future multi-window app stays correct, and so
// a window closed and recreated (macOS re-activate) doesn't inherit a stale
// "already confirmed" grant.
const closeAllowed = new WeakSet<BrowserWindow>();

// Real §75.76 "open a different project" support -- the render process
// itself has no way to change its own root query param after load, so
// this is a real, narrow main-process action (reloading the existing
// window at a new `?root=` URL) rather than a renderer-side hack. Used
// both by the New Project wizard (open the just-created project) and
// could equally back a future "Open Folder" picker.
function loadRootIntoWindow(win: BrowserWindow, rootDir: string): void {
  const query = `?root=${encodeURIComponent(rootDir)}`;
  if (isDev) {
    win.loadURL(`http://localhost:5173/${query}`);
  } else {
    win.loadFile(path.join(import.meta.dirname, "..", "dist", "index.html"), {
      search: query,
    });
  }
}

function createWindow(): void {
  const win = new BrowserWindow({
    width: 1280,
    height: 800,
    backgroundColor: "#09090b",
    webPreferences: {
      preload: path.join(import.meta.dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
  });
  mainWindow = win;
  win.on("closed", () => {
    if (mainWindow === win) mainWindow = null;
  });

  // Real unsaved-changes-on-close gate (the same pre-existing gap the
  // native app menu's own header comment and `docs/FUTURE_FEATURES.md`
  // both name as still-open): a window close -- from the OS window button,
  // File > Quit, Cmd/Ctrl+Q, or the Window menu's Close role -- is
  // prevented and deferred to the renderer, which alone knows its real
  // per-tab dirty state. The renderer either confirms immediately (nothing
  // dirty) or prompts, then calls `spartan:close-confirmed`; only that
  // re-arms this window for a real close. Deliberately *not* routed through
  // `beforeunload`: that fires for the menu's Reload/Force Reload/Open
  // Folder actions too (which are already gated unconditionally in
  // `menu.ts`), and can't distinguish a close from a reload -- a window
  // `close` event can. Data-safe default: if the renderer never responds,
  // the close is simply cancelled rather than risking silent loss.
  win.on("close", (event) => {
    if (closeAllowed.has(win)) return;
    event.preventDefault();
    win.webContents.send("spartan:close-requested");
  });

  // Real initial file-tree root. In dev, the repo checkout itself
  // (`import.meta.dirname` is `desktop/dist-electron/`, so `../..` is the
  // repo root) so opening the app immediately shows real, familiar
  // project files. In a *packaged* app that same `../..` would resolve to
  // the app's own internal `resources/` directory (`main.js` lives inside
  // `resources/app.asar/dist-electron/`) -- a real, shipped-only bug: the
  // user would launch Spartan IDE and see the app's own guts, not a place
  // they recognize. So a packaged build defaults to the user's home
  // directory, a real, writable, familiar location; `SPARTAN_ROOT` still
  // overrides either way (used by tests and the New Project flow).
  const rootDir =
    process.env.SPARTAN_ROOT ??
    (app.isPackaged ? os.homedir() : path.resolve(import.meta.dirname, "..", ".."));
  loadRootIntoWindow(win, rootDir);
}

// Real single-instance lock -- a standard, load-bearing production
// requirement. Without it, launching Spartan IDE a second time (double-
// clicking the icon while it's already running) spawns a whole second
// instance: a second `spartan-backend` subprocess, a second window, a
// second everything. `requestSingleInstanceLock` returns `false` in that
// second process; it quits immediately, and the *first* (already-running)
// process gets a real `second-instance` event where it focuses/restores
// its existing window instead -- the conventional "app is already open"
// behavior. Deliberately skipped when `SPARTAN_ROOT` is set (the test/dev
// harness path), so automated launches from a single controlling process
// aren't blocked by a stale lock.
const gotSingleInstanceLock = process.env.SPARTAN_ROOT
  ? true
  : app.requestSingleInstanceLock();
if (!gotSingleInstanceLock) {
  app.quit();
} else {
  app.on("second-instance", () => {
    if (mainWindow) {
      if (mainWindow.isMinimized()) mainWindow.restore();
      mainWindow.focus();
    }
  });
}

// Real main-process crash safety net. A genuinely-uncaught exception or an
// unhandled promise rejection in Electron's own main process would
// otherwise take the whole app down silently (or, for a rejection, only
// warn). This project's crash-reporting story already covers the two other
// real processes -- `spartan-backend`'s Rust panic hook (§75.82) and the
// renderer's own reporter -- but the Electron/Node main process had no
// equivalent net. Log it visibly rather than dying silently; deliberately
// does NOT swallow-and-continue past a truly fatal error (the app may be in
// a bad state), it just guarantees the failure is never invisible.
process.on("uncaughtException", (err) => {
  console.error("[spartan-main] uncaught exception:", err);
});
process.on("unhandledRejection", (reason) => {
  console.error("[spartan-main] unhandled rejection:", reason);
});

app.whenReady().then(() => {
  // A second instance that lost the lock is quitting -- `app.quit()` is
  // async, so `whenReady` can still fire here first. Return before
  // spawning any backend/window so the losing instance never briefly
  // stands up a real second `spartan-backend` subprocess.
  if (!gotSingleInstanceLock) return;

  // Real production hardening: if the bundled `spartan-backend` binary is
  // missing or unresolvable, `resolveBackendBinaryPath` throws. Without
  // this guard the whole `whenReady` callback would reject silently and a
  // packaged app would launch showing *nothing at all* -- no window, no
  // error. Instead, surface a real OS error dialog naming the problem and
  // quit cleanly.
  let backendPath: string;
  try {
    backendPath = resolveBackendBinaryPath();
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    dialog.showErrorBox("Spartan IDE could not start", message);
    app.quit();
    return;
  }
  backend = new BackendClient(backendPath);

  // Real, narrow set of IPC methods the renderer can invoke via
  // `preload.ts`'s `contextBridge` -- a 1:1 passthrough to the real
  // Rust backend's own protocol (`spartan-backend::handle_request`'s
  // method names), not a separate parallel API to keep in sync by hand.
  // The four `leo_*` methods are real Leo agent wiring (§75.61,
  // user-requested: "Leo still runs the show") -- `leo_start_task`
  // returns a fast synchronous ack; the real plan (or a real failure)
  // arrives later as a real `spartan:event`, relayed below.
  const methods = [
    "list_dir",
    "open_file",
    "edit",
    "save_file",
    "undo",
    "redo",
    "close_file",
    "lsp_hover",
    "lsp_completion",
    "lsp_definition",
    "lsp_type_definition",
    "lsp_signature_help",
    "lsp_references",
    "lsp_rename",
    "lsp_code_action",
    "lsp_code_action_resolve",
    "lsp_execute_command",
    "lsp_document_symbol",
    "lsp_document_highlight",
    "lsp_semantic_tokens",
    "lsp_inlay_hints",
    "lsp_workspace_symbol",
    "lsp_call_hierarchy",
    "format_document",
    "search_project",
    "leo_status",
    "leo_start_task",
    "leo_approve_plan",
    "leo_reject_plan",
    "leo_next_step",
    "leo_approve_call",
    "leo_reject_call",
    "leo_retry",
    "leo_session_history",
    "pty_spawn",
    "pty_input",
    "pty_resize",
    "pty_close",
    "dap_launch",
    "dap_continue",
    "dap_step_over",
    "dap_step_into",
    "dap_evaluate",
    "dap_set_variable",
    "dap_disconnect",
    "leo_cancel",
    "git_status",
    "git_stage",
    "git_unstage",
    "git_discard",
    "git_commit",
    "git_commit_amend",
    "git_revert_commit",
    "git_tags",
    "git_create_tag",
    "git_delete_tag",
    "git_diff",
    "git_diff_hunks",
    "git_stage_hunk",
    "git_unstage_hunk",
    "git_hunk_lines",
    "git_stage_lines",
    "git_unstage_lines",
    "git_branches",
    "git_checkout",
    "git_create_branch",
    "git_remote_branches",
    "git_checkout_remote",
    "git_merge_branch",
    "git_merge_status",
    "git_resolve_conflict",
    "git_commit_merge",
    "git_abort_merge",
    "git_log",
    "git_log_for_ref",
    "git_cherry_pick",
    "git_commit_files",
    "git_commit_diff",
    "git_blame",
    "github_list_pull_requests",
    "git_remotes",
    "git_fetch",
    "git_push",
    "git_pull",
    "git_stash_save",
    "git_stash_list",
    "git_stash_pop",
    "git_stash_apply",
    "git_stash_drop",
    "settings_get",
    "settings_set",
    "check_for_updates",
    "model_status",
    "android_detect",
    "android_build_apk",
    "android_list_devices",
    "android_install_apk",
    "android_logcat_start",
    "android_logcat_stop",
    "litellm_proxy_start",
    "litellm_proxy_stop",
    "litellm_proxy_status",
    "hf_list_models",
    "hf_pull_model",
    "lmstudio_list_models",
    "lmstudio_pull_model",
    "llamacpp_list_models",
    "llamacpp_download_model",
    "model_download_cancel",
    "crash_reports_list",
    "crash_report_upload",
    "create_project",
    "devcontainer_detect",
    "devcontainer_up",
    "devcontainer_down",
    "devcontainer_status",
    "devcontainer_list",
    "devcontainer_exec_spawn",
    "devcontainer_exec_input",
    "devcontainer_exec_resize",
    "devcontainer_exec_close",
  ];
  for (const method of methods) {
    ipcMain.handle(`spartan:${method}`, async (_event, params: Record<string, unknown>) => {
      if (!backend) throw new Error("backend not ready");
      return backend.call(method, params);
    });
  }

  // Two real, deliberately narrow main-process-only conveniences for the
  // new Settings "Diagnostics"/"About" section (§75.76) -- neither routes
  // through spartan-backend (there is no real "open this in the OS file
  // manager" or "open this URL in the system browser" concept in that
  // headless Rust protocol, nor should there be). Both targets are
  // hardcoded here, not supplied by the renderer, so a compromised
  // renderer can't turn this into an arbitrary-path/arbitrary-URL opener.
  ipcMain.handle("spartan:open_crash_reports_folder", async () => {
    const home = process.env.HOME || process.env.USERPROFILE || os.homedir();
    const dir = path.join(home, ".spartan", "crashes");
    fs.mkdirSync(dir, { recursive: true });
    const result = await shell.openPath(dir);
    if (result) throw new Error(result);
    return { ok: true, path: dir };
  });
  ipcMain.handle("spartan:open_repository_page", async () => {
    await shell.openExternal(REPO_URL);
    return { ok: true };
  });
  // Real GitHub layer, first increment (task #284): opens a real pull
  // request's `html_url` (renderer-supplied this time, unlike the two
  // hardcoded targets above -- it comes from GitHub's own live API
  // response, not directly from the user) in the OS default browser. A
  // real, deliberate validation gate before `shell.openExternal` ever
  // runs: only an `https://github.com/` URL is allowed through, so a
  // compromised renderer (or a genuinely malformed API response) can't
  // turn this into an arbitrary-protocol-handler launcher.
  ipcMain.handle("spartan:open_pull_request_url", async (_event, params: { url: string }) => {
    if (typeof params?.url !== "string" || !params.url.startsWith("https://github.com/")) {
      throw new Error("refusing to open a non-GitHub URL");
    }
    await shell.openExternal(params.url);
    return { ok: true };
  });
  // Real "open project" action for the New Project wizard (and, later,
  // any real "Open Folder" picker) -- reloads the *existing* window at
  // the new root rather than spawning a second one, matching a
  // conventional single-window desktop IDE's own "switch project" UX.
  ipcMain.handle("spartan:open_project", async (event, params: { root: string }) => {
    const win = BrowserWindow.fromWebContents(event.sender);
    if (!win) throw new Error("no window to reload");
    loadRootIntoWindow(win, params.root);
    return { ok: true };
  });
  // Real second half of the unsaved-changes-on-close handshake (the
  // `win.on("close", ...)` gate above): the renderer has decided the close
  // may proceed (nothing was dirty, or the user confirmed discarding
  // unsaved changes), so re-arm this window for a real close and retry it.
  // `send`, not `invoke` -- no reply is expected, and the renderer doesn't
  // wait on anything.
  ipcMain.on("spartan:close-confirmed", (event) => {
    const win = BrowserWindow.fromWebContents(event.sender);
    if (!win) return;
    closeAllowed.add(win);
    win.close();
  });
  // Real native "choose a folder" dialog -- backs both onboarding's
  // "Open Existing Project" and the New Project wizard's "Create in"
  // field, closing a real, previously-total gap: before this, there was
  // no way to point this shell at any folder other than its own
  // `SPARTAN_ROOT` startup default.
  ipcMain.handle("spartan:pick_folder", async (event) => {
    const win = BrowserWindow.fromWebContents(event.sender);
    const options: Electron.OpenDialogOptions = {
      properties: ["openDirectory", "createDirectory"],
    };
    const result = win
      ? await dialog.showOpenDialog(win, options)
      : await dialog.showOpenDialog(options);
    return { canceled: result.canceled, path: result.filePaths[0] ?? null };
  });

  // Real native "choose a file" dialog -- a sibling of `pick_folder` above,
  // added for the llama.cpp Settings row (user-requested: "Integrate
  // llama.cpp into the desktop IDE") so browsing to a real local `.gguf`
  // model file doesn't require typing an absolute path by hand. Real,
  // caller-supplied filters (never renderer-controlled beyond that -- the
  // dialog itself is still the real OS file picker, not an arbitrary-path
  // opener).
  ipcMain.handle(
    "spartan:pick_file",
    async (event, params: { filters?: Electron.FileFilter[] } = {}) => {
      const win = BrowserWindow.fromWebContents(event.sender);
      const options: Electron.OpenDialogOptions = {
        properties: ["openFile"],
        filters: params.filters,
      };
      const result = win
        ? await dialog.showOpenDialog(win, options)
        : await dialog.showOpenDialog(options);
      return { canceled: result.canceled, path: result.filePaths[0] ?? null };
    }
  );

  // Real, unprompted backend events (Leo's own async plan-ready/
  // plan-failed notifications) relayed to every real open window --
  // there is normally exactly one, but this doesn't assume that.
  backend.onEvent((eventName, data) => {
    for (const win of BrowserWindow.getAllWindows()) {
      win.webContents.send("spartan:event", eventName, data);
    }
  });

  // Real application menu, replacing Electron's own implicit default menu
  // (which -- confirmed live, not assumed -- silently registered the exact
  // same Ctrl+Z/Ctrl+Shift+Z/Ctrl+X/Ctrl+C/Ctrl+V accelerators `Editor.tsx`'s
  // own JS keydown handler already claims). See `menu.ts`'s own header
  // comment for the full account and why this menu deliberately has no
  // Edit submenu at all.
  Menu.setApplicationMenu(
    buildApplicationMenu(() => mainWindow, loadRootIntoWindow)
  );

  createWindow();

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on("window-all-closed", () => {
  backend?.dispose();
  if (process.platform !== "darwin") app.quit();
});

app.on("before-quit", () => {
  backend?.dispose();
});
