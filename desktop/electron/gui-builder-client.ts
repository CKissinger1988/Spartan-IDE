// Real client for the already-real, already-tested `gui-builder/` npm
// project's CLI (`gui-builder/dist/cli.js`, §75.38-§75.53) -- the actual
// GUI Builder AST-sync/bundling engine, unwired into any real UI until
// now (user-requested: "the visual GUI Builder and live app preview are
// mandatory"). Spawned directly from Electron's own main process (also
// Node) rather than routed through the Rust `spartan-backend` -- unlike
// every other real backend concern in this shell, this one has zero Rust
// dependency, so adding an unnecessary Rust hop would only add latency
// and a second process to keep alive for no real benefit. Each call is a
// real, one-shot subprocess (matching the CLI's own "no persistent
// server, no file watching" v1 contract, see its own doc comment) --
// simpler than `BackendClient`'s long-lived stdio session, since there's
// no real background state to keep across calls.

import { execFile } from "node:child_process";

function runCli(cliPath: string, args: string[], stdin?: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = execFile(
      "node",
      [cliPath, ...args],
      { maxBuffer: 32 * 1024 * 1024 },
      (error, stdout, stderr) => {
        if (error) {
          // Real, honest error surfaced from the CLI's own stderr JSON
          // (`{"error": "..."}`) when present, matching its own
          // documented "never mixed into stdout" contract -- falls back
          // to the raw stderr text if it wasn't real JSON for some
          // unexpected reason.
          try {
            const parsed = JSON.parse(stderr);
            reject(new Error(parsed.error ?? stderr));
          } catch {
            reject(new Error(stderr || error.message));
          }
          return;
        }
        resolve(stdout);
      }
    );
    if (stdin !== undefined) {
      child.stdin?.write(stdin);
      child.stdin?.end();
    }
  });
}

export class GuiBuilderClient {
  constructor(private cliPath: string) {}

  async parseComponent(path: string): Promise<unknown> {
    const stdout = await runCli(this.cliPath, [path]);
    return JSON.parse(stdout);
  }

  async bundleComponent(path: string): Promise<unknown> {
    const stdout = await runCli(this.cliPath, ["bundle", path]);
    return JSON.parse(stdout);
  }

  async applyEdit(editJson: string, source: string): Promise<unknown> {
    const stdout = await runCli(this.cliPath, ["apply", editJson], source);
    return JSON.parse(stdout);
  }
}
