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
  "lsp_hover",
  "lsp_completion",
  "lsp_definition",
  "lsp_type_definition",
  "lsp_signature_help",
  "lsp_references",
  "lsp_rename",
  "lsp_document_symbol",
  "lsp_document_highlight",
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
  "leo_cancel",
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
    // Real, deliberate isolation: Electron's own IPC emitter (like Node's
    // plain EventEmitter) doesn't catch per-listener exceptions, so one
    // registered listener throwing on a malformed/unexpected payload could
    // otherwise stop delivery to every other real listener for this same
    // event -- the same class of risk `BackendClient`'s own dispatch loop
    // guards against in `web/`.
    const handler = (_e: unknown, eventName: string, data: unknown) => {
      try {
        listener(eventName, data);
      } catch (err) {
        console.error("spartan.onEvent listener threw:", err);
      }
    };
    ipcRenderer.on("spartan:event", handler);
    return () => ipcRenderer.removeListener("spartan:event", handler);
  },
  // Two real, narrow, hardcoded-target main-process actions (§75.76) --
  // deliberately separate from `call`/`ALLOWED_METHODS` above since
  // neither is a `spartan-backend` protocol method; both open something
  // main.ts itself decided, never a renderer-supplied path/URL.
  openCrashReportsFolder: (): Promise<unknown> =>
    ipcRenderer.invoke("spartan:open_crash_reports_folder"),
  openRepositoryPage: (): Promise<unknown> => ipcRenderer.invoke("spartan:open_repository_page"),
  openPullRequestUrl: (url: string): Promise<unknown> =>
    ipcRenderer.invoke("spartan:open_pull_request_url", { url }),
  openProject: (root: string): Promise<unknown> =>
    ipcRenderer.invoke("spartan:open_project", { root }),
  pickFolder: (): Promise<unknown> => ipcRenderer.invoke("spartan:pick_folder"),
  pickFile: (filters?: { name: string; extensions: string[] }[]): Promise<unknown> =>
    ipcRenderer.invoke("spartan:pick_file", { filters }),
  // Real two-way unsaved-changes-on-close handshake with main.ts (whose
  // window `close` event is prevented until the renderer confirms) --
  // deliberately separate from `call`/`ALLOWED_METHODS`: this isn't a
  // `spartan-backend` protocol method at all, it's renderer-local dirty
  // state that only `App.tsx` knows. `onCloseRequested` follows the exact
  // same subscribe/return-unsubscribe convention `onEvent` itself uses.
  onCloseRequested: (listener: () => void): (() => void) => {
    const handler = () => {
      try {
        listener();
      } catch (err) {
        console.error("spartan.onCloseRequested listener threw:", err);
      }
    };
    ipcRenderer.on("spartan:close-requested", handler);
    return () => ipcRenderer.removeListener("spartan:close-requested", handler);
  },
  confirmClose: (): void => ipcRenderer.send("spartan:close-confirmed"),
});
