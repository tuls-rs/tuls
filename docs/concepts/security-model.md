---
title: Security model
description: What tuls enforces, what it does not, and how to deploy it safely.
---

# Security model

`tuls` separates **tool authorization** from **runtime containment**.

## What tuls enforces

- strict built-in capability/tool policy;
- default-deny child MCP tool policy for agents;
- tool removal from discovery plus call-time enforcement;
- strict public MCP JSON inputs;
- agent field validation;
- environment-based provider credentials;
- minimal environment inheritance for spawned commands/stdio child MCPs;
- bounded tool/provider/network outputs;
- filesystem root checks for filesystem operations;
- conservative public-network fetch policy;
- no automatic HTTP redirects in fetch/provider/child HTTP clients;
- explicit timeout handling.

## What tuls does not claim to enforce

::: danger `tuls shell` is not an OS sandbox

Directory roots do not restrict the syscalls made by a spawned executable.

:::

Filesystem path validation is designed to prevent ordinary path/symlink escape,
but path validation is not a replacement for a kernel-enforced capability
filesystem when hostile concurrent processes can mutate paths during an
operation.

## Deployment recommendations

For hostile or highly autonomous workloads, deploy `tuls` inside a real sandbox
and restrict:

- writable filesystem paths;
- readable secret paths;
- network destinations;
- process execution;
- environment variables;
- operating-system identity and privileges.

See [SECURITY.md](https://github.com/tuls-rs/tuls/blob/main/SECURITY.md) for the
security boundary and deployment recommendations.

::: tip Related

- [Limits & bounded behavior](./limits) — every safety boundary in one table.
- [Agent profiles](../configuration/agent-profiles) — least-privilege shapes.
- [Workspace trust](../servers/agents#workspace-trust) — treat agent definitions as executable config.

:::
