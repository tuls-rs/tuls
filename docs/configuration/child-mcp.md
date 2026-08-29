---
title: Child MCP servers
description: stdio and HTTP child MCP servers for agents, with tool selectors and defense in depth.
---

# Child MCP servers

Agents can use named child MCP servers, declared in YAML frontmatter.

Two transport types are supported:

```text
stdio
http
```

## stdio child MCP

```yaml
mcp_servers:
  filesystem:
    type: stdio
    command: tuls
    args: ["filesystem", ".", "--allow", "filesystem.read"]
```

The child process:

- runs locally as a direct exec of `command`, with the OS identity of the
  `tuls` process;
- starts in the agent workspace;
- has `kill_on_drop` enabled;
- receives a minimal inherited environment;
- does not implicitly inherit unrelated credentials;
- uses MCP `2026-07-28` discovery.

## Explicit child environment

If a child MCP server itself needs a credential, pass only that credential:

```yaml
mcp_servers:
  external:
    type: stdio
    command: external-mcp
    args: ["serve"]
    env: { EXTERNAL_API_KEY: "${EXTERNAL_API_KEY}" }
```

`${NAME}` placeholders selectively expose individual variables from the
`tuls agents` process environment to the child; nothing else is inherited. A
missing variable fails the child startup.

## HTTP child MCP

```yaml
mcp_servers:
  issues:
    type: http
    url: https://mcp.example.com/mcp
    headers: { Authorization: "Bearer ${ISSUE_MCP_TOKEN}" }
```

HTTP child MCP clients use bounded timeouts and do not follow redirects.
Header values support the same `${NAME}` environment interpolation.

## Child tool selectors

Selector format:

```text
server/tool
server/*
```

Example:

```yaml
tools:
  - filesystem/read_text_file
  - filesystem/search_files
  - fetch/*
disallowed_tools:
  - fetch/some_tool_name
```

## Selector rules

1. Empty `tools` means **no child MCP tools** (default deny).
2. `server/*` grants all tools advertised by that named child server.
3. `server/tool` grants one exact child tool.
4. `disallowed_tools` always overrides `tools`.
5. A selector referencing an unknown configured server is rejected.
6. After connection, an exact selector referencing a tool not actually
   advertised by that child server is rejected.
7. Authorization is based on this explicit policy, not child-provided
   read-only/destructive annotations.
8. A child tool's reported `isError` is preserved and committed to the agent
   conversation as an error output; the run continues.
9. A call that times out or fails after dispatch has an ambiguous outcome
   (the tool may have executed) and the session is marked **non-resumable**.
10. Completed sessions are resumable: `send_input` on a completed agent starts
    a new run that continues the retained conversation.

## Defense in depth

For built-in child servers, restrict both layers.

Good:

```yaml
tools:
  - filesystem/*
mcp_servers:
  filesystem:
    type: stdio
    command: tuls
    args: ["filesystem", ".", "--allow", "filesystem.read"]
```

This means:

- the child filesystem process itself exposes only read operations;
- the agent policy grants only tools from that child server.

::: danger Don't rely on one layer for high-risk tools

For high-risk tools, restrict the child process **and** the agent policy.
Do not rely on only one of those layers.

:::

::: tip Related

- [Agent configuration](./subagents) — the `tools`/`disallowed_tools` fields.
- [Agent profiles](./agent-profiles) — layered example profiles.

:::
