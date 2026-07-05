# Agent Deck Integration Notes

## Upstream Contract

Agent Deck exposes a simple command gateway:

```bash
agent-deck launch <path> -c <tool-or-command> -t <title> -g <group>
agent-deck add <path> -c <tool-or-command> -t <title> -g <group>
agent-deck list --json
agent-deck status -v
agent-deck session attach <id-or-title>
agent-deck conductor setup <name> --description "<description>"
agent-deck web --listen 127.0.0.1:8420
```

The `-c/--cmd` value may be a built-in Agent Deck tool such as `claude`,
`gemini`, `opencode`, or `codex`, or any installed CLI command with arguments.

## Spartan Layer

[../bin/spartan-agent-deck](../bin/spartan-agent-deck) reads
[../config/ai-clis.tsv](../config/ai-clis.tsv), detects which commands are
available, and launches them through Agent Deck with Spartan group defaults.

The launcher does not hide Agent Deck. It prints the exact command before
execution so every action is auditable.

## Adding Another AI CLI

Add a tab-separated row to `config/ai-clis.tsv`:

```text
mycli	My CLI	mycli --some-flag	Spartan/custom	cyan	Custom Agent Deck command.
```

Then run:

```bash
./bin/spartan-agent-deck doctor
./bin/spartan-agent-deck launch
```
