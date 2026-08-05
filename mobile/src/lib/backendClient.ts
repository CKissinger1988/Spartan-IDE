// Real WebSocket client for spartan-devserver's transport — the React Native
// port of web/src/backendClient.ts, adapted for React Native's WebSocket
// API (no Origin header sent, so the devserver accepts by token alone).
//
// The flow: fetch GET /__spartan/session from the devserver's static port
// to obtain { wsPort, wsToken }, then open a WebSocket to that port with
// that token. React Native's WebSocket does not send an Origin header, so
// the devserver's Origin allowlist check is skipped — authenticated by
// token alone, exactly the same as any non-browser client.
//
// **Optional by construction**: the app works fully with mock data when no
// backend is present. connect() rejects cleanly, and the app stays in its
// existing mock-data mode. When a backend IS present, this is the seam
// real backend capabilities flow through.

export interface BackendSession {
  wsPort: number;
  wsToken: string;
  projectRoot: string | null;
}

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
 * A live connection to a local spartan-devserver over WebSocket.
 * Construct via `BackendClient.connect()`. Requests are correlated to
 * responses by a monotonically increasing id; unsolicited events (no id)
 * fan out to `onEvent` listeners.
 */
export class BackendClient {
  private readonly ws: WebSocket;
  private nextId = 1;
  private readonly pending = new Map<number, PendingCall>();
  private readonly listeners = new Set<EventListener>();
  readonly projectRoot: string | null;

  private constructor(ws: WebSocket, projectRoot: string | null) {
    this.ws = ws;
    this.projectRoot = projectRoot;
    ws.onmessage = (ev) => this.onMessage(ev);
    ws.onclose = () => this.rejectAllPending('connection closed');
  }

  /**
   * Fetch the session handoff from the devserver and open the WebSocket.
   * `baseUrl` defaults to `http://127.0.0.1:3000` (the devserver's
   * default static port). Override for different host/port configurations.
   */
  static async connect(opts?: {
    baseUrl?: string;
    pairingToken?: string | null;
  }): Promise<BackendClient> {
    const baseUrl = opts?.baseUrl ?? 'http://127.0.0.1:3000';
    const endpoint = new URL(baseUrl);

    const pairingToken = opts?.pairingToken?.trim();
    const resp = await fetch(baseUrl + '/__spartan/session', {
      headers: pairingToken ? { 'X-Spartan-Mobile-Pairing': pairingToken } : undefined,
    });
    if (!resp.ok) {
      throw new Error(`session handoff failed: HTTP ${resp.status}`);
    }
    const session = (await resp.json()) as BackendSession;
    // A phone's 127.0.0.1 is the phone, not the Linux machine running
    // spartan-devserver. Keep the host from the configured handoff origin;
    // this is the mobile equivalent of web/src/backendClient.ts's wsHost.
    const wsScheme = endpoint.protocol === 'https:' ? 'wss' : 'ws';
    const url = `${wsScheme}://${endpoint.hostname}:${session.wsPort}/?token=${encodeURIComponent(session.wsToken)}`;

    const ws = new WebSocket(url);
    await new Promise<void>((resolve, reject) => {
      ws.onopen = () => resolve();
      ws.onerror = () => reject(new Error('WebSocket connection failed'));
    });
    return new BackendClient(ws, session.projectRoot ?? null);
  }

  private onMessage(ev: MessageEvent): void {
    if (typeof ev.data !== 'string') return;
    let msg: unknown;
    try {
      msg = JSON.parse(ev.data);
    } catch {
      return;
    }
    if (typeof msg !== 'object' || msg === null) return;

    // An event has an `event` field and no `id`; a response always has `id`.
    if ('event' in msg && !('id' in msg)) {
      const event = msg as BackendEvent;
      for (const listener of this.listeners) {
        try {
          listener(event);
        } catch (e) {
          console.error('BackendClient event listener threw:', e);
        }
      }
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
