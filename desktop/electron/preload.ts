// Real, narrow contextBridge surface (§9 least-privilege posture) --
// the renderer gets exactly `window.spartan.call` and `window.spartan.
// onEvent`, which only ever reach the real `spartan:*` IPC channels
// `main.ts` registers. No raw `ipcRenderer`, no Node API of any kind is
// exposed to the renderer -- `nodeIntegration: false` +
// `contextIsolation: true` in `main.ts`'s `BrowserWindow` config make
// that the only real path in.

import { contextBridge, ipcRenderer } from "electron";

const ALLOWED_METHODS = new Set([
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
  "design_parse",
  "design_bundle",
  "design_apply_edit",
  "pty_spawn",
  "pty_input",
  "pty_resize",
  "pty_close",
]);

contextBridge.exposeInMainWorld("spartan", {
  call: (method: string, params: Record<string, unknown> = {}): Promise<unknown> => {
    if (!ALLOWED_METHODS.has(method)) {
      return Promise.reject(new Error(`spartan.call: method "${method}" is not allowed`));
    }
    return ipcRenderer.invoke(`spartan:${method}`, params);
  },
  // Real, unprompted backend events (Leo's async plan-ready/plan-failed
  // notifications, relayed by `main.ts`) -- returns a real unsubscribe
  // function, the same convention `BackendClient.onEvent` itself uses.
  onEvent: (listener: (event: string, data: unknown) => void): (() => void) => {
    const handler = (_e: unknown, eventName: string, data: unknown) => listener(eventName, data);
    ipcRenderer.on("spartan:event", handler);
    return () => ipcRenderer.removeListener("spartan:event", handler);
  },
});
