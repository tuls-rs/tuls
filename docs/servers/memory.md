---
title: Memory server
description: A persistent JSONL knowledge graph.
---

# Memory server

Start the server:

```bash
tuls memory \
  --memory-file /work/state/memory.jsonl \
  --allow memory.read \
  --allow memory.write
```

If `--memory-file` is omitted:

1. `MEMORY_FILE_PATH` is used when present;
2. otherwise `memory.jsonl` is used in the process working directory.

## Tools

| Tool                  | Capability     | Purpose                               |
| --------------------- | -------------- | ------------------------------------- |
| `read_graph`          | `memory.read`  | Read the graph                        |
| `search_nodes`        | `memory.read`  | Search names, types, and observations |
| `open_nodes`          | `memory.read`  | Open named entities                   |
| `create_entities`     | `memory.write` | Add entities                          |
| `create_relations`    | `memory.write` | Add relations                         |
| `add_observations`    | `memory.write` | Append observations to entities       |
| `delete_entities`     | `memory.write` | Delete entities                       |
| `delete_observations` | `memory.write` | Delete selected observations          |
| `delete_relations`    | `memory.write` | Delete relations                      |

## Knowledge graph resource

The server also exposes the knowledge graph as:

```text
memory://knowledge-graph
```

Grant `memory.read` when resource access is required. The resource is an
auxiliary surface: an exact single-tool grant like `--allow memory/read_graph`
keeps the tool but hides the resource (see
[capability policy](../guide/capability-policy)).

## Persistence

The graph is stored as JSONL and mutations rewrite a complete validated graph
atomically. Existing Unix file permissions are preserved when replacing an
existing regular file.

No-op mutations — duplicate entity/relation/observation creates, or deletes
that match nothing — change nothing: they do not rewrite the file and do not
send resource-update notifications.

Read-only memory:

```bash
tuls memory \
  --memory-file /work/state/memory.jsonl \
  --allow memory.read
```

::: tip Related

- [Limits & bounded behavior](../concepts/limits) — memory file and text field limits.

:::
