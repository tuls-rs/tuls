---
title: Workspace layout
description: The recommended repository structure for agents and skills.
---

# Workspace layout

Recommended repository structure:

```text
project/
├── .agents/
│   ├── agents/
│   │   ├── reviewer.toml
│   │   ├── researcher.toml
│   │   └── implementer.toml
│   └── skills/
│       ├── rust-review/
│       │   ├── SKILL.md
│       │   └── checklist.md
│       └── release-check/
│           └── SKILL.md
├── src/
├── tests/
└── Cargo.toml
```

## Discovery roots

| Surface | Canonical root    | Vendor adapter               |
| ------- | ----------------- | ---------------------------- |
| Agents  | `.agents/agents/` | `.claude/agents/` (Markdown) |
| Skills  | `.agents/skills/` | `.claude/skills/`            |

Canonical definitions take precedence on collisions.

::: tip Related

- [Skills server](../servers/skills) — SKILL.md discovery details.
- [Agents server](../servers/agents) — agent discovery and workspace trust.

:::
