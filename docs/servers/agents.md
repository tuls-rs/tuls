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

Both tools require the **MCP Tasks extension**: the client must declare the
`io.modelcontextprotocol/tasks` client capability, otherwise the server rejects
the call. Each call returns immediately with a standard task handle
(`resultType: "task"`) instead of a final result.

| Tool          | Capability   | Purpose                                                   |
| ------------- | ------------ | --------------------------------------------------------- |
| `spawn_agent` | `agents.run` | Start a named agent on a task as an MCP Task              |
| `send_input`  | `agents.run` | Continue an existing agent conversation as a new MCP Task |

## Typical parent flow

```text
1. spawn_agent(name="reviewer", task="Review src/fetch for SSRF issues")
2. poll tasks/get(taskId) until the task settles
3. read the terminal task result: agentId, agent name, final response
4. optionally send_input(target=<agentId>, ...) for follow-up turns
5. cancel an in-flight turn with tasks/cancel
```

The task handle returned by `spawn_agent` carries a `taskId` that is distinct
from the agent session `agentId`. The session `agentId` appears in the
structured content of the terminal task result and is the target for
`send_input`.

While a task is running, `tasks/get` reports `statusMessage` updates as the
agent works (starting child MCP servers, waiting for the model, running a tool,
collecting the final response). A completed turn settles `status: "completed"`
with a `CallToolResult` payload:

- successful runs: `isError: false`, `structuredContent` carries
  `{agentId, name, result}`;
- failed runs: `isError: true`, `structuredContent` carries
  `{agentId, name, kind, message, resumable}`;
- `tasks/cancel` settles the task as `cancelled`.

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
- A session is resumable only after its last turn has settled; starting a
  replacement turn with `send_input` while a turn is still running is rejected.

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
