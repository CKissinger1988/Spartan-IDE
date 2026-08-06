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
//
// Real production-packaging fix (§75.77 named this gap explicitly): this
// used to spawn a bare `"node"` off `$PATH`, which a packaged end-user
// machine has no guarantee of having installed at all. Electron's own
// binary already bundles a full Node runtime -- `process.execPath` (the
// real Electron binary itself) run with `ELECTRON_RUN_AS_NODE=1` in its
// environment makes Electron behave as a plain Node executable for this
// one child process, the standard, documented technique for exactly this
// problem. `process.env` is spread first since setting `env` at all
// replaces the child's entire environment rather than extending it --
// dropping it would have broken PATH-dependent lookups inside the CLI
// itself (none exist today, but silently relying on that would be
// fragile). Real, executed verification: `npm run build:electron` (tsc)
// clean, and the exact same real fixture round trip §75.62's own
// verification used (parse/bundle/apply against a real `Card.jsx`) was
// re-run manually against the compiled output and produced identical
// real results to before this change -- confirming the behavior, not
// just the compile, is unaffected for a real dev machine that does still
// have `node` on PATH.

import { execFile } from "node:child_process";

function runCli(cliPath: string, args: string[], stdin?: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = execFile(
      process.execPath,
      [cliPath, ...args],
      {
        maxBuffer: 32 * 1024 * 1024,
        env: { ...process.env, ELECTRON_RUN_AS_NODE: "1" },
      },
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

  async parseComponentSource(path: string, source: string): Promise<unknown> {
    const stdout = await runCli(this.cliPath, ["parse-source", path], source);
    return JSON.parse(stdout);
  }

  async bundleComponentSource(path: string, source: string): Promise<unknown> {
    const stdout = await runCli(this.cliPath, ["bundle-source", path], source);
    return JSON.parse(stdout);
  }

  async applyEdit(editJson: string, source: string): Promise<unknown> {
    const stdout = await runCli(this.cliPath, ["apply", editJson], source);
    return JSON.parse(stdout);
  }

  /** Real component-library discovery (task #278). `fromFile` lets each
   * result carry the relative module specifier an import in that file
   * would actually need, so inserting a component from another file can
   * bring its import with it. */
  async discoverComponents(rootDir: string, fromFile?: string): Promise<unknown> {
    const args = fromFile ? ["components", rootDir, fromFile] : ["components", rootDir];
    const stdout = await runCli(this.cliPath, args);
    return JSON.parse(stdout);
  }

  async discoverComponentsFromSource(path: string, fromFile: string, source: string): Promise<unknown> {
    const stdout = await runCli(this.cliPath, ["component-source", path, fromFile], source);
    return JSON.parse(stdout);
  }

  async discoverAssets(rootDir: string, fromFile?: string, sourceOverrides: Record<string, string> = {}): Promise<unknown> {
    const args = fromFile ? ["assets", rootDir, fromFile, "--source-overrides"] : ["assets", rootDir, "", "--source-overrides"];
    const stdout = await runCli(this.cliPath, args, JSON.stringify(sourceOverrides));
    return JSON.parse(stdout);
  }

  async readAssetSource(path: string): Promise<unknown> {
    const stdout = await runCli(this.cliPath, ["asset-source", path]);
    return JSON.parse(stdout);
  }

  async discoverTokens(rootDir: string, sourceOverrides: Record<string, string> = {}): Promise<unknown> {
    const stdout = await runCli(this.cliPath, ["tokens", rootDir, "--source-overrides"], JSON.stringify(sourceOverrides));
    return JSON.parse(stdout);
  }

  async discoverTokensFromSource(path: string, rootDir: string, source: string): Promise<unknown> {
    const stdout = await runCli(this.cliPath, ["token-source", path, rootDir], source);
    return JSON.parse(stdout);
  }

  async applyTokenValue(path: string, name: string, value: string, source: string): Promise<unknown> {
    const stdout = await runCli(this.cliPath, ["token-apply", path, name, value], source);
    return JSON.parse(stdout);
  }

  async defineTokenValue(path: string, name: string, value: string, source: string): Promise<unknown> {
    const stdout = await runCli(this.cliPath, ["token-define", path, name, value], source);
    return JSON.parse(stdout);
  }

  async removeTokenValue(path: string, name: string, source: string): Promise<unknown> {
    const stdout = await runCli(this.cliPath, ["token-remove", path, name], source);
    return JSON.parse(stdout);
  }
}
