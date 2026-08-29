---
title: OpenRouter agents
description: A complete first-class OpenRouter walkthrough.
---

# OpenRouter agents

`openrouter` is a first-class provider: the endpoint, credential variable, and
wire are fixed (`https://openrouter.ai/api/v1` + `/responses`,
`OPENROUTER_API_KEY`, Responses), so agent files never repeat them.

## 1. Export the credential

```bash
export OPENROUTER_API_KEY='...'
```

The API key is read by the `tuls agents` process when an agent is spawned.
It does not need to be copied into the child filesystem/fetch MCP processes.

## 2. Create an OpenRouter agent

Create:

```text
.agents/agents/web-researcher.md
```

```markdown
---
name: web-researcher
description: Researches public web sources through OpenRouter
provider: openrouter
model: openai/gpt-5.6-luna
reasoning_effort: high
max_turns: 32
tools:
  - fetch/*
mcp_servers:
  fetch:
    type: stdio
    command: tuls
    args: ["fetch", "--allow", "network.fetch"]
---

Research the requested topic. Use fetch when evidence is needed and return
concise, source-oriented findings.
```

The resulting provider request goes to:

```text
POST https://openrouter.ai/api/v1/responses
Authorization: Bearer $OPENROUTER_API_KEY
```

::: tip Model identifiers

OpenRouter model identifiers use provider-qualified names. Replace
`openai/gpt-5.6-luna` with the OpenRouter model you actually want to run.

:::

## 3. Start the agents MCP server

From the workspace root:

```bash
tuls agents . --allow agents.run
```

The parent MCP client will discover `web-researcher` in the
`spawn_agent` catalog.

## 4. Spawn it from the parent model

Conceptually:

```json
{
  "name": "web-researcher",
  "task": "Compare the current Rust MCP ecosystem and identify the most relevant libraries."
}
```

The call returns a task handle. Poll `tasks/get` with the returned `taskId`
until the task settles, then read the terminal task result for the agent's
`agentId`, name, and final response.

## OpenRouter implementer with filesystem access

```markdown
---
name: openrouter-implementer
description: Implements scoped code changes through OpenRouter
provider: openrouter
model: openai/gpt-5.6-luna
reasoning_effort: high
max_turns: 48
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

Implement the requested changes. Keep edits scoped to the workspace and
preserve project conventions.
```

## OpenRouter researcher with fetch + read-only workspace

```markdown
---
name: openrouter-investigator
description: Combines public-web research with read-only workspace inspection
provider: openrouter
model: openai/gpt-5.6-luna
reasoning_effort: high
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

Investigate the task using repository evidence and public sources. Do not
modify workspace files.
```

## Why Responses for OpenRouter?

`tuls` sends Responses credentials using Bearer authentication, matching
OpenRouter's Responses endpoint at `/api/v1/responses`. The wire is fixed:
`openrouter` rejects `api` overrides.

::: warning Do not expect anthropic-messages for OpenRouter

That wire also changes the HTTP authentication contract to Anthropic-style
`x-api-key`, so it is intended for endpoints that explicitly implement that
contract.

:::

::: tip Related

- [Provider configuration](./providers) — the full provider matrix.
- [Agent profiles](./agent-profiles) — recommended least-privilege shapes.

:::
