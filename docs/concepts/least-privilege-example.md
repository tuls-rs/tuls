---
title: Least-privilege example
description: A complete layered OpenRouter setup from zero to effective permissions.
---

# Example: complete least-privilege OpenRouter setup

This example gives a parent model access only to the agents orchestration
surface. The spawned agent can read repository files and fetch public web
content, but cannot write files or execute arbitrary processes.

## Workspace

```text
project/
└── .agents/
    └── agents/
        └── investigator.md
```

## `.agents/agents/investigator.md`

```markdown
---
name: investigator
description: Investigates repository issues using read-only files and public web research
provider: openrouter
model: openai/gpt-5.6-luna
reasoning_effort: high
max_turns: 32
tools:
  - filesystem/*
  - fetch/*
mcp_servers:
  filesystem:
    type: stdio
    command: tuls
    args: ["filesystem", ".", "--allow", "filesystem.read"]
  fetch:
    type: stdio
    command: tuls
    args: ["fetch", "--allow", "network.fetch"]
---

Inspect repository evidence first. Use public web research only when needed.
Do not modify files and do not execute local programs.
```

## Start environment

```bash
cd /absolute/path/to/project
export OPENROUTER_API_KEY='...'
tuls agents . --allow agents.run
```

## Effective permissions

| Layer                   | Granted                                 | Not granted                               |
| ----------------------- | --------------------------------------- | ----------------------------------------- |
| Parent MCP surface      | `agents.run`                            | filesystem, fetch, memory, shell directly |
| Agent child policy      | `filesystem/*`, `fetch/*`               | shell, memory, undeclared child servers   |
| Child filesystem server | `filesystem.read`                       | `filesystem.write`                        |
| Child fetch server      | `network.fetch`, public-network default | private network, redirects                |
| OS process boundary     | normal account permissions              | **not sandboxed by tuls**                 |

This layered model is the recommended pattern: grant the model only the tool
families it needs, and independently restrict each child MCP process to the
minimum operation set required for its role.

::: tip Related

- [Agent profiles](../configuration/agent-profiles) — narrower single-role variants.
- [Security model](./security-model) — the reasoning behind the layers.

:::
