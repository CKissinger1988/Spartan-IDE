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

let backend: BackendClient | null = null;

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
    "close_file",
    "leo_status",
    "leo_start_task",
    "leo_approve_plan",
    "leo_reject_plan",
  ];
  for (const method of methods) {
    ipcMain.handle(`spartan:${method}`, async (_event, params: Record<string, unknown>) => {
      if (!backend) throw new Error("backend not ready");
      return backend.call(method, params);
    });
  }

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
