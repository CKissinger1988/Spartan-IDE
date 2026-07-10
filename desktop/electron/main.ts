// Real Electron main process for the new desktop shell (user-requested
// pivot away from the wgpu-native `spartan-editor-core` UI, keeping that
// crate and every sibling Rust crate as the real, tested backend). This
// process owns window creation and the real `spartan-backend` subprocess;
// the renderer never talks to either directly -- `preload.ts`'s
// `contextBridge` is the one real, narrow API surface it gets, matching
// this workspace's own §9 "least privilege" posture even though this is
// a new UI stack.

import { app, BrowserWindow, ipcMain } from "electron";
import * as path from "node:path";
import * as fs from "node:fs";
import { BackendClient } from "./backend-client.js";
import { GuiBuilderClient } from "./gui-builder-client.js";

const isDev = !app.isPackaged;

function resolveBackendBinaryPath(): string {
  // Real release binary built by `cargo build --release -p spartan-backend`
  // from the repo root -- `desktop/` is a direct child of the repo root,
  // so this is a fixed, real relative path during development. Packaged
  // builds will need a real bundled binary path (not attempted in this
  // first increment -- see `docs/architecture-spec.md`'s own honest
  // "what this does not confirm" list for this pass).
  const devPath = path.resolve(
    import.meta.dirname,
    "..",
    "..",
    "target",
    "release",
    process.platform === "win32" ? "spartan-backend.exe" : "spartan-backend"
  );
  if (!fs.existsSync(devPath)) {
    throw new Error(
      `spartan-backend binary not found at ${devPath} -- run "cargo build --release -p spartan-backend" from the repo root first.`
    );
  }
  return devPath;
}

function resolveGuiBuilderCliPath(): string {
  // Real, already-built `gui-builder/` npm project (§75.38-§75.53) -- the
  // actual GUI Builder AST-sync/bundling engine, a sibling of `desktop/`
  // at the repo root, not a dependency of it (deliberately its own
  // separate npm project since day one, see its own README.md).
  const cliPath = path.resolve(
    import.meta.dirname,
    "..",
    "..",
    "gui-builder",
    "dist",
    "cli.js"
  );
  if (!fs.existsSync(cliPath)) {
    throw new Error(
      `gui-builder CLI not found at ${cliPath} -- run "npm run build" inside gui-builder/ first.`
    );
  }
  return cliPath;
}

let backend: BackendClient | null = null;
let guiBuilder: GuiBuilderClient | null = null;

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
  const query = `?root=${encodeURIComponent(rootDir)}`;

  if (isDev) {
    win.loadURL(`http://localhost:5173/${query}`);
  } else {
    win.loadFile(path.join(import.meta.dirname, "..", "dist", "index.html"), {
      search: query,
    });
  }
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
    "leo_status",
    "leo_start_task",
    "leo_approve_plan",
    "leo_reject_plan",
    "pty_spawn",
    "pty_input",
    "pty_resize",
    "pty_close",
    "git_status",
    "git_stage",
    "git_unstage",
    "git_commit",
    "settings_get",
    "settings_set",
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
