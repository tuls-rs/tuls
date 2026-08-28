---
title: Subagent configuration
description: Write canonical TOML agent definitions.
---

# Subagent configuration

Canonical TOML is the recommended format for repository-owned agents because it
is explicit, strict, and easy to review.

## Minimal OpenAI agent

Create:

```text
.agents/agents/reviewer.toml
```

```toml
name = "reviewer"
description = "Reviews workspace code without modifying files"
instructions = "Review the requested code and report concrete correctness, security, and maintainability issues."

model_provider = "openai"
model = "YOUR_OPENAI_MODEL"

allow_tools = ["filesystem/*"]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = ["filesystem", ".", "--allow", "filesystem.read"]
```

Before starting `tuls agents`, make sure the provider credential exists in its
environment:

```bash
export OPENAI_API_KEY='...'
tuls agents . --allow agents.run
```

## Minimal Anthropic agent

```toml
name = "reviewer-anthropic"
description = "Reviews workspace code using Anthropic"
instructions = "Review the requested code. Do not modify files."

model_provider = "anthropic"
model = "YOUR_ANTHROPIC_MODEL"

allow_tools = ["filesystem/*"]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = ["filesystem", ".", "--allow", "filesystem.read"]
```

Credential:

```bash
export ANTHROPIC_API_KEY='...'
```

## Canonical agent fields

| Field              | Required    | Default          | Description                                                                         |
| ------------------ | ----------- | ---------------- | ----------------------------------------------------------------------------------- |
| `name`             | Yes         | —                | Stable local agent identifier, 1–64 chars                                           |
| `description`      | Yes         | —                | Catalog description shown to the parent model, at most 4 KiB                        |
| `instructions`     | TOML: yes   | —                | System/task instructions; Markdown uses body text                                   |
| `model_provider`   | Yes         | —                | `openai`, `anthropic`, `openrouter`, or `custom`                                    |
| `model`            | Yes         | —                | Provider model identifier                                                           |
| `base_url`         | Custom: yes | provider default | Provider API prefix/root; rejected for first-class providers                        |
| `env_key`          | Custom: yes | provider default | Environment variable holding the API credential; rejected for first-class providers |
| `wire_api`         | Custom: yes | provider default | `responses` or `anthropic-messages`; rejected for first-class providers             |
| `temperature`      | No          | provider default | `0..=2` for Responses, `0..=1` for Anthropic Messages                               |
| `reasoning_effort` | No          | provider default | Wire-specific reasoning effort                                                      |
| `max_turns`        | No          | `32`             | Provider/tool loop limit, `1..=128`                                                 |
| `allow_tools`      | No          | empty            | Explicit child MCP grants; empty means no child tools                               |
| `deny_tools`       | No          | empty            | Explicit child MCP denials; deny wins                                               |
| `skills`           | No          | empty            | Skills injected into the agent's system context                                     |
| `mcp_servers`      | No          | empty            | Named stdio or HTTP child MCP servers                                               |

::: danger Strict validation

Unknown canonical fields are rejected.

:::

## Reasoning effort

For `wire_api = "responses"`:

```text
none
minimal
low
medium
high
xhigh
```

For `wire_api = "anthropic-messages"`:

```text
low
medium
high
xhigh
max
```

::: tip Only set what the model supports

`tuls` validates the wire-level vocabulary, while the upstream provider remains
the authority on model-specific support.

:::

::: tip Related

- [Provider configuration](./providers) — endpoints, credentials, and wire APIs.
- [Child MCP servers](./child-mcp) — transports and tool selectors.
- [Agent profiles](./agent-profiles) — ready-made, least-privilege policies.

:::
