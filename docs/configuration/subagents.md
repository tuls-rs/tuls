---
title: Agent configuration
description: Write agent definitions as Markdown with YAML frontmatter.
---

# Agent configuration

Agents are Markdown files with YAML frontmatter, placed under
`.agents/agents/`. The frontmatter defines the agent; the Markdown body is the
instruction text. Instructions live in the body only — there is no
`instructions` frontmatter field. Discovery is recursive, and kebab-case
filenames are recommended.

Supported recursive layouts include:

```text
.agents/agents/reviewer.md
.agents/agents/security/reviewer.md
.agents/agents/reviewer/agent.md
```

## Minimal OpenAI agent

Create:

```text
.agents/agents/code-reviewer.md
```

```markdown
---
name: code-reviewer
description: Reviews workspace code without modifying files
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

Review the requested code and report concrete correctness, security, and
maintainability issues.
```

Before starting `tuls agents`, make sure the provider credential exists in its
environment:

```bash
export OPENAI_API_KEY='...'
tuls agents . --allow agents.run
```

## Minimal Anthropic agent

```markdown
---
name: code-reviewer-anthropic
description: Reviews workspace code using Anthropic
provider: anthropic
model: YOUR_ANTHROPIC_MODEL
tools:
  - filesystem/*
mcp_servers:
  filesystem:
    type: stdio
    command: tuls
    args: ["filesystem", ".", "--allow", "filesystem.read"]
---

Review the requested code. Do not modify files.
```

Credential:

```bash
export ANTHROPIC_API_KEY='...'
```

## Agent fields

| Field              | Required    | Default          | Description                                                                         |
| ------------------ | ----------- | ---------------- | ----------------------------------------------------------------------------------- |
| `name`             | Yes         | —                | Stable local agent identifier, 1–64 chars                                           |
| `description`      | Yes         | —                | Catalog description shown to the parent model, at most 4 KiB                        |
| `subagent`         | No          | `true`           | Whether `tuls agents` may expose and launch this definition as a subagent           |
| `provider`         | Yes         | —                | `openai`, `anthropic`, `openrouter`, or `custom`                                    |
| `model`            | Yes         | —                | Provider model identifier                                                           |
| `base_url`         | Custom: yes | provider default | Provider API prefix/root; rejected for first-class providers                        |
| `credential_env`   | Custom: yes | provider default | Environment variable holding the API credential; rejected for first-class providers |
| `api`              | Custom: yes | provider default | `responses` or `anthropic-messages`; rejected for first-class providers             |
| `temperature`      | No          | provider default | `0..=2` for Responses, `0..=1` for Anthropic Messages                               |
| `reasoning_effort` | No          | provider default | Wire-specific reasoning effort                                                      |
| `max_turns`        | No          | `32`             | Provider/tool loop limit, `1..=128`                                                 |
| `tools`            | No          | empty            | Explicit child MCP grants; empty means no child tools                               |
| `disallowed_tools` | No          | empty            | Explicit child MCP denials; deny wins                                               |
| `skills`           | No          | empty            | Skills injected into the agent's system context                                     |
| `mcp_servers`      | No          | empty            | Named stdio or HTTP child MCP servers                                               |

::: danger Strict validation

Unknown frontmatter fields are rejected.

:::

## Subagent eligibility

Set `subagent: false` on a leader definition that a surrounding AI client uses
as its main system prompt but that `tuls agents` must not expose or launch:

```markdown
---
name: leader
description: Coordinates work and delegates specialized tasks
provider: openai
model: YOUR_OPENAI_MODEL
subagent: false
---

You are the main agent for this workspace.
Delegate focused work to specialists when useful.
```

The default is `true`, so ordinary specialist definitions can omit the field.
`subagent: false` removes the definition from the `spawn_agent` schema and
catalog, and direct calls using that name are rejected as unknown. The entire
file remains subject to the normal required-field, provider, MCP, secret, and
bounds validation.

This setting controls only eligibility in the `tuls agents` MCP server. It does
not prevent another AI client from reading the Markdown file and using its body
as a primary/system prompt. Eligibility is static configuration; `tuls` does
not infer the currently active client agent.

## Instructions: Markdown body only

There is no `instructions` frontmatter field. Everything after the closing
`---` of the YAML frontmatter is the agent's instruction text, and it must be
nonempty. Keep the body focused on task/system instructions; keep metadata in
the frontmatter.

## Default deny

An agent has **no child MCP tools** until `tools` grants them. Declaring a
child server in `mcp_servers` never grants access by itself — every grant is
explicit and per-server:

```markdown
---
name: reviewer
description: Reviews workspace code without modifying files
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

Review the requested code.
```

## Reasoning effort

For `api: responses`:

```text
none
minimal
low
medium
high
xhigh
```

For `api: anthropic-messages`:

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
