---
title: OpenRouter subagents
description: A complete first-class OpenRouter walkthrough.
---

# OpenRouter subagents

`openrouter` is a first-class provider: the endpoint, credential variable, and
wire are fixed (`https://openrouter.ai/api/v1` + `/responses`,
`OPENROUTER_API_KEY`, Responses), so agent files never repeat them.

## 1. Export the credential

```bash
export OPENROUTER_API_KEY='...'
```

The API key is read by the `tuls agents` process when a subagent is spawned.
It does not need to be copied into the child filesystem/fetch MCP processes.

## 2. Create an OpenRouter agent

Create:

```text
.agents/agents/openrouter-researcher.toml
```

```toml
name = "openrouter-researcher"
description = "Researches public web sources through OpenRouter"
instructions = "Research the requested topic. Use fetch when evidence is needed and return concise, source-oriented findings."

model_provider = "openrouter"
model = "openai/gpt-5.6-luna"
reasoning_effort = "high"
max_turns = 32

allow_tools = ["fetch/*"]

[mcp_servers.fetch]
type = "stdio"
command = "tuls"
args = ["fetch", "--allow", "network.fetch"]
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

The parent MCP client will discover `openrouter-researcher` in the
`spawn_agent` catalog.

## 4. Spawn it from the parent model

Conceptually:

```json
{
  "name": "openrouter-researcher",
  "task": "Compare the current Rust MCP ecosystem and identify the most relevant libraries."
}
```

The call returns an `agentId`. The parent should keep that ID and call
`wait_agent` to obtain the terminal result.

## OpenRouter implementer with filesystem access

```toml
name = "openrouter-implementer"
description = "Implements scoped code changes through OpenRouter"
instructions = "Implement the requested changes. Keep edits scoped to the workspace and preserve project conventions."

model_provider = "openrouter"
model = "openai/gpt-5.6-luna"
reasoning_effort = "high"
max_turns = 48

allow_tools = ["filesystem/*"]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = [
  "filesystem",
  ".",
  "--allow",
  "filesystem.read",
  "--allow",
  "filesystem.write",
]
```

## OpenRouter researcher with fetch + read-only workspace

```toml
name = "openrouter-investigator"
description = "Combines public-web research with read-only workspace inspection"
instructions = "Investigate the task using repository evidence and public sources. Do not modify workspace files."

model_provider = "openrouter"
model = "openai/gpt-5.6-luna"
reasoning_effort = "high"

allow_tools = [
  "filesystem/*",
  "fetch/*",
]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = ["filesystem", ".", "--allow", "filesystem.read"]

[mcp_servers.fetch]
type = "stdio"
command = "tuls"
args = ["fetch", "--allow", "network.fetch"]
```

## Why Responses for OpenRouter?

`tuls` sends Responses credentials using Bearer authentication, matching
OpenRouter's Responses endpoint at `/api/v1/responses`. The wire is fixed:
`openrouter` rejects `wire_api` overrides.

::: warning Do not expect anthropic-messages for OpenRouter

That wire also changes the HTTP authentication contract to Anthropic-style
`x-api-key`, so it is intended for endpoints that explicitly implement that
contract.

:::

::: tip Related

- [Provider configuration](./providers) — the full provider matrix.
- [Agent profiles](./agent-profiles) — recommended least-privilege shapes.

:::
