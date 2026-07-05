# Spartan IDE Agent Deck Console (legacy)

**This is the prior product in this repository, preserved for reference.** It has been replaced
by the from-scratch Spartan IDE architecture documented at
[`/docs/architecture-spec.md`](../../docs/architecture-spec.md). See
[§55 of that spec](../../docs/architecture-spec.md) for exactly how each feature below maps into
the new design (§52 External Agent Fleet, §53 Neural Link, §54 Ops Cockpit). Nothing here is on
the new architecture's build path — it's kept unmodified as the working parity reference until
each row in §55's matrix is actually reimplemented natively.

---

Spartan IDE Agent Deck Console is a high-performance, terminal-first command center for
orchestrating AI CLIs via [Agent Deck](https://github.com/asheshgoplani/agent-deck). Designed for
operators who demand speed and transparency, it provides a unified interface for the world's most
powerful AI engines with integrated resource management.

## Key Features

- **Multi-Engine Fleet:** Seamlessly switch between Gemini (default), Gemma, agy, Codex, Vibe, Copilot, OpenAI, and ChatGPT.
- **Usage Tracker & Auto-Switcher:** Real-time token usage monitoring with automatic failover to fallback models when limits are reached.
- **Dynamic Cockpit:** A Node.js-powered web dashboard for live fleet monitoring, configuration, and usage metrics.
- **Neural Link:** Safe, local data analysis bridge for IDE workspace auditing.
- **Cyber-Ops Aesthetics:** Compact glass panels and operator-focused controls inspired by high-stakes security suites.

## What This Integrates

The launcher routes every configured AI CLI through Agent Deck's `-c/--cmd` contract. Gemini CLI is the default engine for all new Spartan sessions.

Configured tools live in [config/ai-clis.tsv](config/ai-clis.tsv):

- Claude Code
- OpenAI Codex
- Google Gemini CLI
- OpenCode
- Aider
- OpenAI CLI
- GitHub Copilot CLI
- Cursor Agent
- Qwen Code
- Amp
- Goose
- Crush
- Continue
- OpenHands

## Run The Terminal GUI

```bash
./bin/spartan-agent-deck
```

Useful direct commands:

```bash
./bin/spartan-agent-deck doctor
./bin/spartan-agent-deck launch
./bin/spartan-agent-deck status
./bin/spartan-agent-deck list
./bin/spartan-agent-deck web
./bin/spartan-agent-deck settings
./bin/spartan-agent-deck neural-link
```

Import all local AI skills into Agent Deck's skill pool:

```bash
./scripts/import-ai-skills.sh
./bin/spartan-agent-deck skills
```

The launcher expects `agent-deck` to be installed. This workspace was verified
with Agent Deck v1.9.32 installed at `/home/creator/.local/bin/agent-deck`; the
launcher also checks that path automatically when it is not on `PATH`.

## Static Cockpit Preview

Open [web/index.html](web/index.html) in a browser for the Spartan-styled
cockpit mockup. It does not execute commands from the browser; it documents the
same command model used by the terminal GUI.

The cockpit includes a settings cog with panels for Agent Deck, API key
environment variables, MCP server configuration, local AI instruction files, and
the AI CLI/skill registries.

## Neural Link

The Neural Link connects this IDE to
`C:\GitHub\Spartan_Hub_Master` for local data analysis and BrainBridge-ready
assimilation queues. It does not start Jarvis or run autonomous network,
credential, prompt-injection, or lateral-movement routines.

```bash
./scripts/neural-link.py status
./scripts/neural-link.py analyze .
./scripts/neural-link.py analyze /mnt/c/GitHub/Spartan_Hub_Master
./scripts/neural-link.py feed
```

## Agent Deck Patterns

Launch a detected CLI:

```bash
agent-deck launch . -g Spartan/agents -t "Codex Workspace" -c "codex"
```

Launch a custom CLI:

```bash
agent-deck launch . -g Spartan/agents -t "Aider Workspace" -c "aider"
```

Attach to a session:

```bash
agent-deck session attach "Codex Workspace"
```

Start the upstream web UI:

```bash
agent-deck web --listen 127.0.0.1:8420
```
