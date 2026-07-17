// Real client for spartan-devserver's WebSocket transport (§75.88) + the
// same-origin token handoff (`/__spartan/session`). This is the browser
// half of the answer to the question `ws_transport`'s own doc comment left
// open, and that this app's own `App.tsx` doc comment named as its reason
// LSP/DAP/Leo/git weren't wired: "how does a browser tab legitimately learn
// the per-process token and the correct origin?"
//
// The flow: the devserver serves this app's static files, so a same-origin
// `fetch("/__spartan/session")` returns `{ wsPort, wsToken }` (a cross-origin
// page is blocked from reading it by the browser's SOP -- see
// static_serve.rs). We then open the WebSocket to that port with that token;
// the browser attaches this page's Origin, which the devserver allowlisted.
//
// **Optional by construction**: the app works fully client-side with no
// backend. `BackendClient.connect()` rejects cleanly (no devserver serving
// this page -- e.g. a Vite dev server or plain static hosting), and the app
// stays in its existing client-only mode. When a backend *is* present, this
// is the seam real backend capabilities (file ops today; LSP/DAP/Leo/git as
// they land) flow through -- speaking the exact same Request/Response/Event
// wire shape `spartan-backend` already uses over stdio and Electron IPC.

/** The session-handoff path spartan-devserver serves same-origin. */
export const SESSION_PATH = "/__spartan/session";

export interface BackendSession {
  wsPort: number;
  wsToken: string;
  /** The devserver's real, absolute, already-canonicalized project root, or
   * `null` if none could be resolved (see `main.rs`/`static_serve.rs`). This
   * is what makes `git_status`/`open_file`/Leo methods usable from the
   * browser at all -- the File System Access API deliberately never exposes
   * a real OS path for a folder the user picks via `showDirectoryPicker`. */
  projectRoot: string | null;
}

/** An unsolicited event pushed by the backend (no `id`), e.g. Leo progress
 * or streaming PTY output -- the same `{ event, data }` shape the Rust side
 * sends over any transport. */
export interface BackendEvent {
  event: string;
  data: unknown;
}

export type EventListener = (event: BackendEvent) => void;

interface PendingCall {
  resolve: (result: unknown) => void;
  reject: (error: Error) => void;
}

/**
 * A live connection to a local spartan-devserver. Construct via
 * `BackendClient.connect()`. Requests are correlated to responses by a
 * monotonically increasing id; unsolicited events (no id) fan out to
 * `onEvent` listeners.
 */
export class BackendClient {
  private readonly ws: WebSocket;
  private nextId = 1;
  private readonly pending = new Map<number, PendingCall>();
  private readonly listeners = new Set<EventListener>();
  /** The devserver's advertised project root (see `BackendSession`), or
   * `null` if the connected devserver couldn't resolve one. Real
   * git/file/Leo methods all need this real absolute path. */
  readonly projectRoot: string | null;

  private constructor(ws: WebSocket, projectRoot: string | null) {
    this.ws = ws;
    this.projectRoot = projectRoot;
    ws.addEventListener("message", (ev) => this.onMessage(ev));
    ws.addEventListener("close", () => this.rejectAllPending("connection closed"));
  }

  /**
   * Fetch the same-origin session handoff and open the WebSocket with the
   * returned token. `baseUrl`/`wsHost` default to the current page (browser
   * context); they're overridable so this exact code is drivable from a Node
   * test harness against a real running devserver.
   */
  static async connect(opts?: { baseUrl?: string; wsHost?: string }): Promise<BackendClient> {
    const baseUrl = opts?.baseUrl ?? window.location.origin;
    const wsHost = opts?.wsHost ?? window.location.hostname;

    const resp = await fetch(baseUrl + SESSION_PATH);
    if (!resp.ok) {
      throw new Error(`session handoff failed: HTTP ${resp.status}`);
    }
    const session = (await resp.json()) as BackendSession;
    const url = `ws://${wsHost}:${session.wsPort}/?token=${encodeURIComponent(session.wsToken)}`;

    const ws = new WebSocket(url);
    await new Promise<void>((resolve, reject) => {
      ws.addEventListener("open", () => resolve(), { once: true });
      ws.addEventListener("error", () => reject(new Error("WebSocket connection failed")), {
        once: true,
      });
    });
    return new BackendClient(ws, session.projectRoot ?? null);
  }

  private onMessage(ev: MessageEvent): void {
    if (typeof ev.data !== "string") return;
    let msg: unknown;
    try {
      msg = JSON.parse(ev.data);
    } catch {
      return;
    }
    if (typeof msg !== "object" || msg === null) return;

    // An event has an `event` field and no `id`; a response always has `id`.
    if ("event" in msg && !("id" in msg)) {
      const event = msg as BackendEvent;
      for (const listener of this.listeners) listener(event);
      return;
    }

    const resp = msg as { id: number; result: unknown | null; error: string | null };
    const call = this.pending.get(resp.id);
    if (!call) return;
    this.pending.delete(resp.id);
    if (resp.error) call.reject(new Error(resp.error));
    else call.resolve(resp.result);
  }

  private rejectAllPending(reason: string): void {
    for (const call of this.pending.values()) call.reject(new Error(reason));
    this.pending.clear();
  }

  /** Send a request and resolve with its `result` (or reject on `error`). */
  call(method: string, params: unknown = {}): Promise<unknown> {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      try {
        this.ws.send(JSON.stringify({ id, method, params }));
      } catch (e) {
        this.pending.delete(id);
        reject(e instanceof Error ? e : new Error(String(e)));
      }
    });
  }

  /** Subscribe to unsolicited backend events. Returns an unsubscribe fn. */
  onEvent(listener: EventListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  close(): void {
    this.ws.close();
  }
}
