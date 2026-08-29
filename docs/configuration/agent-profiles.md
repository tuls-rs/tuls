---
title: Recommended agent profiles
description: Ready-made least-privilege agent policies.
---

# Recommended agent profiles

Each profile is a complete agent file: YAML frontmatter plus Markdown body
instructions, placed under `.agents/agents/`.

## Code reviewer

Goal: inspect repository content without modifying it.

```markdown
---
name: code-reviewer
description: Reviews repository content
provider: openai
model: YOUR_OPENAI_MODEL
tools:
  - filesystem/*
mcp_servers:
  filesystem:
    type: stdio
    command: tuls
    args: ["filesystem", ".", "--allow", "filesystem.read"]
---

Review the requested code and report concrete issues.
```

## Web researcher

Goal: public-web access without filesystem or shell access.

```markdown
---
name: web-researcher
description: Researches public web sources
provider: openai
model: YOUR_OPENAI_MODEL
tools:
  - fetch/*
mcp_servers:
  fetch:
    type: stdio
    command: tuls
    args: ["fetch", "--allow", "network.fetch"]
---

Research the requested topic and return source-oriented findings.
```

## Implementer

Goal: read and edit repository files without arbitrary process execution.

```markdown
---
name: implementer
description: Implements scoped code changes
provider: openai
model: YOUR_OPENAI_MODEL
tools:
  - filesystem/*
mcp_servers:
  filesystem:
    type: stdio
    command: tuls
    args:
      - filesystem
      - .
      - --allow
      - filesystem.read
      - --allow
      - filesystem.write
---

Implement the requested changes and keep edits scoped.
```

## Test runner

Goal: run commands in addition to reading repository content.

```markdown
---
name: test-runner
description: Runs tests and reads repository content
provider: openai
model: YOUR_OPENAI_MODEL
tools:
  - filesystem/*
  - shell/*
mcp_servers:
  filesystem:
    type: stdio
    command: tuls
    args: ["filesystem", ".", "--allow", "filesystem.read"]
  shell:
    type: stdio
    command: tuls
    args: ["shell", ".", "--allow", "process.execute"]
---

Run the requested tests and report results.
```

::: danger Most privileged profile

This profile is substantially more privileged because `shell` is arbitrary
local process execution under the OS account. Prefer a real OS/container
sandbox when exposing it to an autonomous model.

:::

## Research + implementation split

For larger workflows, prefer multiple narrow agents instead of one all-powerful
agent:

```text
parent
├── researcher      -> fetch only
├── reviewer        -> filesystem.read only
└── implementer     -> filesystem.read + filesystem.write
```

This reduces tool confusion and limits the blast radius of a bad tool choice.

::: tip Related

- [Least-privilege example](../concepts/least-privilege-example) — the full layered setup.
- [Security model](../concepts/security-model) — what each layer guarantees.

:::
