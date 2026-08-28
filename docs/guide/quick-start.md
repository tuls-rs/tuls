---
title: Quick start
description: Launch every tuls server in one place.
---

# Quick start

Every server speaks MCP over stdio. These commands are safe to run from a
terminal to validate behavior before wiring up a client.

## Read-only filesystem MCP

```bash
tuls filesystem /absolute/path/to/project \
  --allow filesystem.read
```

## Read/write filesystem MCP

```bash
tuls filesystem /absolute/path/to/project \
  --allow filesystem.read \
  --allow filesystem.write
```

## Public-web fetch MCP

```bash
tuls fetch \
  --allow network.fetch
```

The fetch defaults are intentionally restrictive:

```text
--robots respect
--network public
```

## Persistent memory MCP

```bash
tuls memory \
  --memory-file /absolute/path/to/memory.jsonl \
  --allow memory.read \
  --allow memory.write
```

## Local process execution MCP

```bash
tuls shell /absolute/path/to/project \
  --allow process.execute
```

## Workspace skills MCP

```bash
tuls skills /absolute/path/to/project \
  --allow skills.read
```

## Workspace subagents MCP

```bash
tuls agents /absolute/path/to/project \
  --allow agents.run
```

::: tip Next

Every launch above grants the minimum capability for its server. See the
[capability policy](./capability-policy) to understand how `--allow` and
`--deny` shape the tool surface, or jump to a specific
[server reference](../servers/).

:::
