---
title: Skills server
description: Discover and activate workspace skills.
---

# Skills server

Skills are discovered from the workspace:

```text
.agents/skills/<skill-name>/SKILL.md
.claude/skills/<skill-name>/SKILL.md
```

The canonical project location is `.agents/skills`. The `.claude/skills`
location is supported as a vendor adapter; canonical definitions take
precedence on collisions.

Start the server:

```bash
tuls skills /work/project \
  --allow skills.read
```

## Tool

| Tool             | Capability    | Purpose                                                             |
| ---------------- | ------------- | ------------------------------------------------------------------- |
| `activate_skill` | `skills.read` | Load one discovered skill's full instructions and resource manifest |

::: tip Supporting files are not auto-injected

A skill's supporting files are not automatically injected into the model
context. The activation result provides resource paths so the model can read
only what is needed.

:::

## Example layout

```text
.agents/
└── skills/
    └── rust-review/
        ├── SKILL.md
        ├── checklist.md
        └── examples/
            └── review.md
```

## Activation result

`activate_skill` returns structured content with the skill's name, description,
`skillDir`, full `instructions`, and a `resources` manifest of supporting files.

::: tip Related

- [Workspace layout](../concepts/workspace-layout) — where skills live.
- [Limits & bounded behavior](../concepts/limits) — skill and catalog limits.

:::
