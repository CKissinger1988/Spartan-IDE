// Real Electron main process for the new desktop shell (user-requested
// pivot away from the wgpu-native `spartan-editor-core` UI, keeping that
// crate and every sibling Rust crate as the real, tested backend). This
// process owns window creation and the real `spartan-backend` subprocess;
// the renderer never talks to either directly -- `preload.ts`'s
// `contextBridge` is the one real, narrow API surface it gets, matching
// this workspace's own §9 "least privilege" posture even though this is
// a new UI stack.

import { app, BrowserWindow, ipcMain, shell, dialog } from "electron";
import * as path from "node:path";
import * as fs from "node:fs";
import * as os from "node:os";
import { BackendClient } from "./backend-client.js";
import { GuiBuilderClient } from "./gui-builder-client.js";

const isDev = !app.isPackaged;

/**
 * Real, shared packaged-vs-dev resource resolver -- found duplicated
 * (near-identical, by a code-review pass) across
 * `resolveBackendBinaryPath`/`resolveGuiBuilderCliPath` below. Real §75.76
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

function resolveGuiBuilderCliPath(): string {
  // Real, already-built `gui-builder/` npm project (§75.38-§75.53) -- the
  // actual GUI Builder AST-sync/bundling engine, a sibling of `desktop/`
  // at the repo root, not a dependency of it (deliberately its own
  // separate npm project since day one, see its own README.md).
  return resolveResourcePath(
    "gui-builder CLI",
    ["gui-builder", "dist", "cli.js"],
    ["gui-builder", "cli.js"],
    'run "npm run build" inside gui-builder/ first.'
  );
}

let backend: BackendClient | null = null;
let guiBuilder: GuiBuilderClient | null = null;

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

  // Real initial file-tree root -- the repo checkout itself in dev, so
  // opening the app immediately shows real, familiar project files
  // rather than an arbitrary empty directory.
  const rootDir = process.env.SPARTAN_ROOT ?? path.resolve(import.meta.dirname, "..", "..");
  loadRootIntoWindow(win, rootDir);
}

app.whenReady().then(() => {
  backend = new BackendClient(resolveBackendBinaryPath());
  guiBuilder = new GuiBuilderClient(resolveGuiBuilderCliPath());

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
    "lsp_signature_help",
    "leo_status",
    "leo_start_task",
    "leo_approve_plan",
    "leo_reject_plan",
    "leo_next_step",
    "leo_approve_call",
    "leo_reject_call",
    "leo_retry",
    "pty_spawn",
    "pty_input",
    "pty_resize",
    "pty_close",
    "dap_launch",
    "dap_continue",
    "dap_step_over",
    "dap_step_into",
    "dap_disconnect",
    "leo_cancel",
    "git_status",
    "git_stage",
    "git_unstage",
    "git_commit",
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

  // Real GUI Builder wiring (§75.62, user-requested: "the visual GUI
  // Builder and live app preview are mandatory") -- routed to the real
  // `gui-builder/` CLI, not `spartan-backend`, since it's pure Node/TS
  // with zero Rust dependency; going through the Rust process would add
  // a pointless extra hop.
  ipcMain.handle("spartan:design_parse", async (_event, params: { path: string }) => {
    if (!guiBuilder) throw new Error("gui-builder not ready");
    return guiBuilder.parseComponent(params.path);
  });
  ipcMain.handle("spartan:design_bundle", async (_event, params: { path: string }) => {
    if (!guiBuilder) throw new Error("gui-builder not ready");
    return guiBuilder.bundleComponent(params.path);
  });
  ipcMain.handle(
    "spartan:design_apply_edit",
    async (_event, params: { edit: unknown; source: string }) => {
      if (!guiBuilder) throw new Error("gui-builder not ready");
      return guiBuilder.applyEdit(JSON.stringify(params.edit), params.source);
    }
  );

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
    await shell.openExternal("https://github.com/CKissinger1988/Spartan-IDE");
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
