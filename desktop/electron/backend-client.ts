// Real client for the spartan-backend subprocess (crates/spartan-backend)
// -- spawns the real Rust binary and speaks its real newline-delimited
// JSON protocol over stdin/stdout. Runs in Electron's main process (not
// the renderer) since Node's child_process API isn't available in a
// sandboxed renderer, and shouldn't be exposed there directly anyway --
// the preload script's contextBridge is the real security boundary.

import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import * as readline from "node:readline";

interface PendingRequest {
  resolve: (value: unknown) => void;
  reject: (reason: Error) => void;
}

export type EventListener = (event: string, data: unknown) => void;

export class BackendClient {
  private proc: ChildProcessWithoutNullStreams;
  private nextId = 1;
  private pending = new Map<number, PendingRequest>();
  private eventListeners = new Set<EventListener>();

  constructor(binaryPath: string) {
    this.proc = spawn(binaryPath, [], { stdio: ["pipe", "pipe", "pipe"] });
    const rl = readline.createInterface({ input: this.proc.stdout });
    rl.on("line", (line) => this.handleLine(line));
    this.proc.stderr.on("data", (chunk: Buffer) => {
      // Real stderr passthrough for visibility during development --
      // the backend itself never writes protocol data here, only real
      // Rust panics/diagnostics would land here.
      console.error(`[spartan-backend] ${chunk.toString()}`);
    });
    this.proc.on("exit", (code) => {
      for (const [, pending] of this.pending) {
        pending.reject(new Error(`spartan-backend exited (code ${code}) before responding`));
      }
      this.pending.clear();
    });
  }

  private handleLine(line: string): void {
    if (!line.trim()) return;
    let parsed: { id?: number; result?: unknown; error?: string; event?: string; data?: unknown };
    try {
      parsed = JSON.parse(line);
    } catch (e) {
      console.error(`[spartan-backend] malformed response line: ${line}`, e);
      return;
    }
    // Real, unprompted server-initiated messages (`spartan_backend::Event`,
    // e.g. Leo's own async plan-ready/plan-failed notifications) carry an
    // `event` field and no `id` -- routed to real subscribers instead of
    // resolving a pending request, since nothing is waiting on them.
    if (typeof parsed.event === "string") {
      for (const listener of this.eventListeners) {
        listener(parsed.event, parsed.data);
      }
      return;
    }
    if (typeof parsed.id !== "number") return;
    const pending = this.pending.get(parsed.id);
    if (!pending) return;
    this.pending.delete(parsed.id);
    if (parsed.error) {
      pending.reject(new Error(parsed.error));
    } else {
      pending.resolve(parsed.result);
    }
  }

  call(method: string, params: Record<string, unknown> = {}): Promise<unknown> {
    const id = this.nextId++;
    const request = { id, method, params };
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.proc.stdin.write(`${JSON.stringify(request)}\n`);
    });
  }

  onEvent(listener: EventListener): () => void {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  }

  dispose(): void {
    this.proc.kill();
  }
}
