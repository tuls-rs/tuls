---
title: Recommended agent profiles
description: Ready-made least-privilege agent policies.
---

# Recommended agent profiles

## Code reviewer

Goal: inspect repository content without modifying it.

```toml
allow_tools = ["filesystem/*"]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = ["filesystem", ".", "--allow", "filesystem.read"]
```

## Web researcher

Goal: public-web access without filesystem or shell access.

```toml
allow_tools = ["fetch/*"]

[mcp_servers.fetch]
type = "stdio"
command = "tuls"
args = ["fetch", "--allow", "network.fetch"]
```

## Implementer

Goal: read and edit repository files without arbitrary process execution.

```toml
allow_tools = ["filesystem/*"]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = [
  "filesystem",
  ".",
  "--allow",
  "filesystem.read",
  "--allow",
  "filesystem.write",
]
```

## Test runner

Goal: run commands in addition to reading repository content.

```toml
allow_tools = [
  "filesystem/*",
  "shell/*",
]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = ["filesystem", ".", "--allow", "filesystem.read"]

[mcp_servers.shell]
type = "stdio"
command = "tuls"
args = ["shell", ".", "--allow", "process.execute"]
```

::: danger Most privileged profile

This profile is substantially more privileged because `shell` is arbitrary
local process execution under the OS account. Prefer a real OS/container
sandbox when exposing it to an autonomous model.

:::

## Research + implementation split

For larger workflows, prefer multiple narrow agents instead of one all-powerful
agent:

```text
parent
├── researcher      -> fetch only
├── reviewer        -> filesystem.read only
└── implementer     -> filesystem.read + filesystem.write
```

This reduces tool confusion and limits the blast radius of a bad tool choice.

::: tip Related

- [Least-privilege example](../concepts/least-privilege-example) — the full layered setup.
- [Security model](../concepts/security-model) — what each layer guarantees.

:::
