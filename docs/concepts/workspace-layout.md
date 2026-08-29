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
│   │   ├── code-reviewer.md
│   │   ├── web-researcher.md
│   │   └── release/
│   │       └── release-checker.md
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

| Surface | Root              | Format                                     |
| ------- | ----------------- | ------------------------------------------ |
| Agents  | `.agents/agents/` | Markdown with YAML frontmatter (recursive) |
| Skills  | `.agents/skills/` | `SKILL.md` with YAML frontmatter           |

Agent discovery is recursive: any `.md` file with YAML frontmatter under
`.agents/agents/` becomes an agent, so nested folders are fine. Use kebab-case
filenames (`code-reviewer.md`, not `code_reviewer.md`).

A workspace can keep `leader.md`, `reviewer.md`, and `researcher.md` together
under this root. Set `subagent: false` in the leader definition to keep it out
of the `tuls agents` spawn catalog; specialists are spawnable by default.

::: tip Related

- [Skills server](../servers/skills) — SKILL.md discovery details.
- [Agents server](../servers/agents) — agent discovery and workspace trust.

:::
