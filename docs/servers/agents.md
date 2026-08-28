---
title: Agents server
description: Run local provider-backed subagents with child MCP tools.
---

# Agents server

The `agents` server lets a parent MCP client spawn named workspace agents. Each
agent has:

- model/provider configuration;
- instructions;
- optional skills;
- an explicit child MCP tool allowlist;
- optional child MCP stdio/HTTP servers;
- a bounded maximum number of provider turns.

Start it with:

```bash
tuls agents /work/project \
  --allow agents.run
```

## Parent-facing tools

| Tool          | Capability   | Purpose                                                  |
| ------------- | ------------ | -------------------------------------------------------- |
| `spawn_agent` | `agents.run` | Start a named agent on a task and return an `agentId`    |
| `send_input`  | `agents.run` | Send follow-up input to a running/resumable agent        |
| `wait_agent`  | `agents.run` | Wait for one or more agents to complete or until timeout |

## Typical parent flow

```text
1. spawn_agent(name="reviewer", task="Review src/fetch for SSRF issues")
2. save returned agentId
3. optionally send_input(target=<agentId>, ...)
4. wait_agent(targets=[<agentId>], timeoutMs=30000)
```

The runtime supports multiple concurrent subagents up to its configured runtime
capacity.

## Sessions and resumability

- Terminal sessions stay resumable: `send_input` on a completed agent starts a
  new run that continues the retained conversation.
- Failed sessions are resumable only for resumable error kinds (interrupts,
  transient provider errors, and child MCP startup failures).
- Context limits, invalid provider requests, missing provider credentials, and
  ambiguous tool execution mark the session **non-resumable**.
- A child tool call that times out or fails after dispatch has an ambiguous
  outcome (the tool may have executed) and the session is marked
  **non-resumable**.

`wait_agent` reports per-agent progress through MCP progress notifications when
the client supplies a progress token.

## Agent discovery

Canonical definitions are discovered recursively under:

```text
.agents/agents/
```

Supported canonical formats:

```text
*.toml
*.md
```

A Claude-compatible Markdown adapter is also discovered under:

```text
.claude/agents/
```

Canonical `.agents/agents` definitions have higher precedence if the same agent
name appears in multiple discovery roots.

## Workspace trust

::: danger Executable trusted configuration

Agent definitions under `.agents/agents/` and `.claude/agents/` are
**executable trusted configuration**: they name the provider endpoint and
credential variable, and stdio child MCP entries declare commands that `tuls`
executes locally under its own OS identity. Running `tuls agents` against a
workspace therefore executes configuration shipped in that repository.

:::

A custom provider endpoint receives the credential named by that definition's
`env_key`. `${NAME}` interpolation in child MCP environment values and headers
exposes the specifically named parent-process environment value.

Point the agents server only at workspaces you trust. An untrusted repository
can define agents that make credentialed provider calls and run arbitrary local
commands, so treat an untrusted repository the same as untrusted code.

::: tip Related

- [Subagent configuration](../configuration/subagents) — write agent definitions.
- [Provider configuration](../configuration/providers) — endpoint and credential matrix.
- [Child MCP servers](../configuration/child-mcp) — transports, selectors, defense in depth.
- [Agent profiles](../configuration/agent-profiles) — ready-made policies.

:::
