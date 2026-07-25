# Tier 0 spikes

Real, tested code — not pseudocode, not mocked servers/adapters. Every spike
here is a real Rust crate (a Cargo workspace member) except
`tree-sitter-wasm-spike` and `git-browser-spike`, real, separate npm
projects (matching `mobile/`'s own precedent for JS-native feasibility
work no Rust equivalent makes sense for). See
`docs/architecture-spec.md` §39 for what each spike is meant to validate and
§47 for the honest, ongoing execution log (what actually ran, what didn't,
and why).

| Spike | What it proves | Needs |
|---|---|---|
| `rope-spike` | Rope-vs-flat-buffer performance, snapshot cost (§2.1, §47.1) | Nothing beyond `cargo` |
| `fallback-parser-spike` | Local-model tool-call fallback parsing, adversarial cases + real local-model fidelity (§3.4, §39.3, §47.2, §47.12) | Nothing beyond `cargo`; optionally a running Ollama instance with a pulled model for the real-fidelity test |
| `dap-spike` | In-house DAP client against real debug adapters (§2.3, §39.2, §47.5, §47.7) | `rustc`; `lldb-dap`/`lldb-dap-18`; optionally `debugpy` for the cross-language test |
| `lsp-spike` | In-house LSP client against real language servers (§2.3, §39.2, §47.6, §47.7) | `rust-analyzer`; optionally `pyright-langserver` for the cross-language test |
| `render-spike` | GPU-half of rope+renderer latency (§2.2, §39.1, §47.9, companion to `rope-spike`'s CPU half) | A Vulkan/DX12/Metal-capable GPU and a display |
| `ui-shell-spike` | Native shell + embedded WebView integration risk (§8, §39.4, §47.11) | A Vulkan/DX12/Metal-capable GPU, a display, and the WebView2 Runtime (ships with Windows 11 by default) |
| `wasm-buffer-spike` | Does `spartan-buffer`'s real rope/undo-tree `Document` compile to WASM and run correctly in a real JS engine, for the planned web app (§75.85) | `rustup target add wasm32-unknown-unknown` + `wasm-bindgen-cli` (pinned to the crate's exact version) to reproduce the real Node run; `cargo test -p wasm-buffer-spike` alone needs nothing extra |
| `tree-sitter-wasm-spike` | Does real tree-sitter parsing/querying work via `web-tree-sitter` in a real JS engine, for the same planned web app (§75.86) — a real npm project, not a Cargo crate | `cd spikes/tree-sitter-wasm-spike && npm install && npm test` |
| `git-browser-spike` | Does `isomorphic-git` (pure JS, zero native `libgit2` dependency) perform real, standard-git-compatible init/add/commit/status/log operations, for the same planned web app (§75.87) — a real npm project, not a Cargo crate | `cd spikes/git-browser-spike && npm install && npm test`; one test additionally self-skips a real cross-tool check if `git` isn't on `$PATH` |

Every test in `dap-spike`/`lsp-spike` skips (prints a message, doesn't fail)
if its required tool isn't on `$PATH` — these are meant to degrade gracefully
across machines with different toolchains installed, not to gate CI on every
optional tool being present everywhere. `fallback-parser-spike`'s
`real_ollama_fidelity` test follows the same pattern: it skips if Ollama
isn't reachable at `localhost:11434` or the specific model isn't pulled,
rather than failing CI on every machine that doesn't have a local model
backend set up.

## Reproducing the tool installs this session actually used

None of this is required to build the workspace — only to exercise the
`dap-spike`/`lsp-spike` tests for real instead of having them skip. Recorded
here because rediscovering it from scratch cost real time once already:

```bash
# DAP: lldb-dap ships with the llvm-18 package on Debian/Ubuntu
apt-get install llvm-18  # provides /usr/bin/lldb-dap-18

# LSP (Rust): a real rust-analyzer binary, not the rustup stub some
# toolchains ship that errors "Unknown binary" until the component is added
rustup component add rust-analyzer

# Cross-language check (Python), both installed via pip — see the note below
# on why this is worth doing at all, not just "because we could"
pip install pyright debugpy
```

All four of the above were reachable and installable in this environment
without any egress-policy exception. A local Ollama install (for Spike 0.3,
§39.3) was **not** — `ollama.com/install.sh` returned a 403 from this
environment's own egress policy, reported rather than routed around, per
this project's own rule about not bypassing sandboxing/security controls for
convenience (§9, §36).

## The lesson §47.7 exists to generalize: test a second adapter, not just the first

`dap-spike` and `lsp-spike` were each built and fully tested against exactly
one server/adapter (`lldb-dap`, `rust-analyzer`) before anyone tried a
second one. That first pass was internally consistent and every test passed
— and still contained a real deadlock that only a second, differently-behaved
adapter (`debugpy`) exposed: `lldb-dap` answers a `launch` request
immediately, while `debugpy` defers the response until after
`configurationDone`, per a DAP-spec-legal pattern neither adapter is wrong to
use. A client that blocks synchronously on the `launch` response before
sending `configurationDone` works perfectly against the first adapter and
deadlocks against the second, and nothing about testing only the first
adapter — however thoroughly — would ever surface that.

**Practical takeaway for anyone extending these spikes, or building the real
language-profile registry (§20.1) this pattern is meant to inform**: passing
tests against one server/adapter is not evidence the client is
protocol-correct in general, only that it matches that one implementation's
particular behavior. Before trusting a new `LanguageProfile`'s LSP/DAP
client pattern, run it against at least one other real implementation of the
same protocol — ideally one from a different vendor, since that's where
divergent-but-spec-legal behavior actually shows up — rather than assuming
the first green test run generalizes. The Language Profile Conformance
Certifier idea in §19 is this lesson turned into a standing product feature
instead of a one-off finding.
