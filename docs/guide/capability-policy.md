---
title: Capability policy
description: How --allow and --deny shape the tool surface of every tuls server.
---

# Capability policy

Every built-in tool belongs to a **capability**. Capabilities are the coarse
unit of grant, and exact tool selectors are the fine unit.

## Canonical capabilities

| Capability         | Server     | Scope                                           |
| ------------------ | ---------- | ----------------------------------------------- |
| `filesystem.read`  | filesystem | Read/list/search/inspect files                  |
| `filesystem.write` | filesystem | Create/edit/write/move files and directories    |
| `network.fetch`    | fetch      | Fetch tool and fetch prompt                     |
| `memory.read`      | memory     | Read/search/open graph data and memory resource |
| `memory.write`     | memory     | Mutate graph data                               |
| `process.execute`  | shell      | Execute local programs                          |
| `skills.read`      | skills     | Activate discovered skills                      |
| `agents.run`       | agents     | Spawn agent tasks and send follow-up input      |

## Exact built-in selectors

Exact selectors use:

```text
server/tool_name
```

Examples:

```text
filesystem/read_text_file
filesystem/write_file
memory/search_nodes
agents/spawn_agent
```

## Policy behavior

With no `--allow`, a directly launched server exposes all of its own tools:

```bash
tuls filesystem .
```

As soon as at least one `--allow` is present, the policy becomes an allowlist:

```bash
tuls filesystem . \
  --allow filesystem.read
```

Grant two capabilities:

```bash
tuls filesystem . \
  --allow filesystem.read \
  --allow filesystem.write
```

Grant read/write but remove one exact operation:

```bash
tuls filesystem . \
  --allow filesystem.read \
  --allow filesystem.write \
  --deny filesystem/move_file
```

Grant a single exact tool:

```bash
tuls filesystem . \
  --allow filesystem/read_text_file
```

::: danger Strict selectors

Invalid capabilities, misspelled tool IDs, and capabilities that do not belong
to the selected server are **rejected at startup**. Selectors are
case-sensitive.

:::

## Auxiliary surfaces (prompts & resources)

An auxiliary MCP surface — the `fetch` prompt, the `memory://knowledge-graph`
resource — is enabled only when:

1. the controlling capability is itself granted **and**
2. the controlling tool is not explicitly denied.

| Grant                                          | `fetch` tool | `fetch` prompt / memory resource |
| ---------------------------------------------- | ------------ | -------------------------------- |
| none (default)                                 | yes          | yes                              |
| exact tool (`--allow fetch/fetch`)             | yes          | **no**                           |
| capability (`--allow network.fetch`)           | yes          | yes                              |
| capability + exact deny (`--deny fetch/fetch`) | **no**       | **no**                           |

Concretely:

- `--allow fetch/fetch` grants only the tool — the prompt stays disabled.
- `--allow network.fetch` grants the capability — tool and prompt are enabled.
- `--deny fetch/fetch` or `--deny memory/read_graph` disables the auxiliary
  surface even when the capability is granted.

## Next steps

- Browse the [server reference](../servers/) for per-server tool tables.
- See [troubleshooting](../troubleshooting) for common selector mistakes.
