---
title: Connecting from an MCP client
description: Configure stdio MCP clients to launch tuls servers.
---

# Connecting from an MCP client

`tuls` uses stdio for its built-in servers. MCP clients commonly represent a
stdio server as a command plus an argument array. The exact configuration file
and field names are client-specific, but the process configuration is
conceptually equivalent to the following:

```json
{
  "mcpServers": {
    "project-files": {
      "command": "tuls",
      "args": [
        "filesystem",
        "/absolute/path/to/project",
        "--allow",
        "filesystem.read"
      ]
    },
    "web": {
      "command": "tuls",
      "args": ["fetch", "--allow", "network.fetch"]
    },
    "subagents": {
      "command": "tuls",
      "args": ["agents", "/absolute/path/to/project", "--allow", "agents.run"]
    }
  }
}
```

The examples use `tuls` resolved through `PATH`. GUI applications may start
with a different working directory and a different `PATH` than your interactive
shell; if a client cannot resolve `tuls`, configure it with the resolved
absolute path to the installed binary.

## Parent agent vs. subagent permissions

There are two separate permission boundaries:

```text
AI client / parent model
        |
        | connects to
        v
  tuls agents <workspace>
        |
        | spawn_agent("reviewer", ...)
        v
 provider-backed subagent
        |
        | may connect only to configured child MCP servers
        v
 filesystem / fetch / other MCP servers
```

The parent model needs `agents.run` to use `spawn_agent` and `send_input`. Both
tools require the **MCP Tasks extension**: the client must declare the
`io.modelcontextprotocol/tasks` client capability, otherwise the server rejects
the call.

The spawned subagent gets **only** the child MCP tools granted by its own
`allow_tools`/`deny_tools` configuration. These are independent policies.

::: tip See also

- [Capability policy](./capability-policy) — what the parent can call.
- [Subagent configuration](../configuration/subagents) — what a subagent may call.
- [Child MCP servers](../configuration/child-mcp) — transports, selectors, and defense in depth.

:::
