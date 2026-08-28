---
title: Filesystem server
description: Read, inspect, search, edit, and move workspace files.
---

# Filesystem server

Start the server:

```bash
tuls filesystem <DIR>... [--allow SELECTOR]... [--deny SELECTOR]...
```

## Tools

| Tool                        | Capability         | Purpose                                            |
| --------------------------- | ------------------ | -------------------------------------------------- |
| `read_text_file`            | `filesystem.read`  | Read bounded UTF-8 text; supports `head` or `tail` |
| `read_media_file`           | `filesystem.read`  | Read a bounded media file into typed MCP content   |
| `read_multiple_files`       | `filesystem.read`  | Read a bounded batch of files                      |
| `list_directory`            | `filesystem.read`  | List directory entries                             |
| `list_directory_with_sizes` | `filesystem.read`  | List entries with sizes and sort by name/size      |
| `directory_tree`            | `filesystem.read`  | Build a bounded tree representation                |
| `search_files`              | `filesystem.read`  | Search by glob-style pattern                       |
| `get_file_info`             | `filesystem.read`  | Return filesystem metadata                         |
| `list_allowed_directories`  | `filesystem.read`  | Show configured roots                              |
| `write_file`                | `filesystem.write` | Atomically replace file contents                   |
| `edit_file`                 | `filesystem.write` | Apply structured edits or preview a diff           |
| `create_directory`          | `filesystem.write` | Create directories                                 |
| `move_file`                 | `filesystem.write` | Move a file or directory inside allowed scope      |

## MCP JSON naming

Public tool JSON uses camelCase:

```json
{
  "path": "src/main.rs",
  "head": 120
}
```

```json
{
  "path": ".",
  "sortBy": "size"
}
```

```json
{
  "path": ".",
  "pattern": "**/*.rs",
  "excludePatterns": ["target/**"]
}
```

::: danger Fail closed

Unknown fields are rejected rather than ignored.

:::

## Path behavior

- All paths must remain under at least one allowed root.
- Relative paths resolve against the first configured root.
- Existing paths are canonicalized before authorization.
- Symlink entries are listed as symlinks; size listing does not follow a
  symlink just to obtain target metadata.
- Writes use a same-directory temporary file and rename for atomic replacement.
- `edit_file` matches `oldText` exactly, with no whitespace or line-ending
  normalization (`\n` never matches `\r\n`), and each `oldText` must occur
  exactly once in the file; a missing or ambiguous match fails the call
  without writing.

## Example scopes

Review-only scope:

```bash
tuls filesystem /work/project \
  --allow filesystem.read
```

Implementation scope without moves:

```bash
tuls filesystem /work/project \
  --allow filesystem.read \
  --allow filesystem.write \
  --deny filesystem/move_file
```

::: tip Related

- [Capability policy](../guide/capability-policy) — how selectors work.
- [Limits & bounded behavior](../concepts/limits) — file, batch, and result limits.
- [Security model](../concepts/security-model) — what path validation does and does not do.

:::
