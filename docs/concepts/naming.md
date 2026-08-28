---
title: Naming conventions
description: How tuls names things at every interface boundary.
---

# Naming conventions

`tuls` intentionally uses different naming conventions at different interface
boundaries instead of mixing styles within one interface.

| Interface                | Convention                | Example                     |
| ------------------------ | ------------------------- | --------------------------- |
| Rust identifiers         | `snake_case`              | `max_length`                |
| Canonical TOML           | `snake_case`              | `allow_tools`               |
| CLI flags                | `--kebab-case`            | `--user-agent`              |
| MCP JSON fields          | `camelCase`               | `maxLength`                 |
| MCP tool names           | `snake_case`              | `read_text_file`            |
| Capabilities             | `domain.action`           | `filesystem.read`           |
| Built-in exact selectors | `server/tool`             | `filesystem/read_text_file` |
| Child MCP selectors      | `server/tool`, `server/*` | `fetch/*`                   |

::: tip Provider-facing tool names

Provider-facing tool names are internally qualified so tools from different
child MCP servers remain distinguishable. Policy configuration should always
use the canonical `server/tool` form documented above.

:::
