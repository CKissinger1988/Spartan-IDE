// Real, narrow contextBridge surface (§9 least-privilege posture) --
// the renderer gets exactly one function, `window.spartan.call`, which
// only ever reaches the six real `spartan:*` IPC channels `main.ts`
// registers. No raw `ipcRenderer`, no Node API of any kind is exposed
// to the renderer -- `nodeIntegration: false` + `contextIsolation: true`
// in `main.ts`'s `BrowserWindow` config make that the only real path in.

import { contextBridge, ipcRenderer } from "electron";

const ALLOWED_METHODS = new Set([
  "list_dir",
  "open_file",
  "edit",
  "save_file",
  "undo",
  "close_file",
]);

contextBridge.exposeInMainWorld("spartan", {
  call: (method: string, params: Record<string, unknown> = {}): Promise<unknown> => {
    if (!ALLOWED_METHODS.has(method)) {
      return Promise.reject(new Error(`spartan.call: method "${method}" is not allowed`));
    }
    return ipcRenderer.invoke(`spartan:${method}`, params);
  },
});
