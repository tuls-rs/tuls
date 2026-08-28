---
title: CLI reference
description: Every command, option, and flag of the tuls binary.
---

# CLI reference

## Top-level syntax

```text
tuls <COMMAND> [OPTIONS]
```

## Server commands

| Command               | Positional input                              | Important options                                                           |
| --------------------- | --------------------------------------------- | --------------------------------------------------------------------------- |
| `filesystem [DIR]...` | Zero or more allowed directories; default `.` | `--allow`, `--deny`                                                         |
| `fetch`               | None                                          | `--robots`, `--network`, `--user-agent`, `--proxy-url`, `--allow`, `--deny` |
| `memory`              | None                                          | `--memory-file`, `--allow`, `--deny`                                        |
| `shell [DIR]...`      | Zero or more allowed directories; default `.` | `--allow`, `--deny`                                                         |
| `skills [DIR]`        | Workspace root; default `.`                   | `--allow`, `--deny`                                                         |
| `agents [DIR]`        | Workspace root; default `.`                   | `--allow`, `--deny`                                                         |

## Shared policy options

| Option             | Repeatable | Meaning                                                                                 |
| ------------------ | ---------- | --------------------------------------------------------------------------------------- |
| `--allow SELECTOR` | Yes        | Enables a capability or exact built-in tool. Once present, policy becomes an allowlist. |
| `--deny SELECTOR`  | Yes        | Denies a capability or exact built-in tool. Deny always wins.                           |

## Fetch-specific options

| Option         | Values                   | Default          | Meaning                                           |
| -------------- | ------------------------ | ---------------- | ------------------------------------------------- |
| `--robots`     | `respect`, `ignore`      | `respect`        | robots.txt policy for autonomous tool calls       |
| `--network`    | `public`, `unrestricted` | `public`         | Outbound destination policy                       |
| `--user-agent` | string                   | `tuls/<version>` | HTTP User-Agent                                   |
| `--proxy-url`  | HTTP(S) URL              | none             | Outbound proxy; requires `--network unrestricted` |

## Memory-specific options

| Option               | Default                                      | Meaning                               |
| -------------------- | -------------------------------------------- | ------------------------------------- |
| `--memory-file PATH` | `MEMORY_FILE_PATH`, otherwise `memory.jsonl` | Persistent JSONL knowledge graph path |

::: tip Relative paths

For `filesystem` and `shell`, relative paths resolve against the **first
configured directory**. If multiple roots are supplied, the first root remains
the default base rather than being reordered internally.

:::
