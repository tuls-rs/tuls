---
title: Provider configuration
description: Endpoints, credentials, and wire APIs for subagent providers.
---

# Provider configuration

## Provider matrix

| `model_provider` | Default base URL               | Default `env_key`    | Default wire         | Authentication sent by tuls            |
| ---------------- | ------------------------------ | -------------------- | -------------------- | -------------------------------------- |
| `openai`         | `https://api.openai.com/v1`    | `OPENAI_API_KEY`     | `responses`          | `Authorization: Bearer ...`            |
| `anthropic`      | `https://api.anthropic.com`    | `ANTHROPIC_API_KEY`  | `anthropic-messages` | `x-api-key: ...` + `anthropic-version` |
| `openrouter`     | `https://openrouter.ai/api/v1` | `OPENROUTER_API_KEY` | `responses`          | `Authorization: Bearer ...`            |
| `custom`         | none                           | none                 | none                 | Determined by `wire_api`               |

::: danger Overrides are rejected for first-class providers

First-class providers (`openai`, `anthropic`, `openrouter`) reject `base_url`,
`env_key`, and `wire_api` overrides: each has a fixed endpoint, credential
variable, and wire contract. `custom` is the only way to reach a differently
shaped endpoint, and it requires `base_url`, `env_key`, and `wire_api` all
explicitly.

:::

## `base_url` semantics

`tuls` appends the wire endpoint to `base_url`.

For Responses:

```text
base_url + /responses
```

Examples:

```text
https://api.openai.com/v1       -> https://api.openai.com/v1/responses
https://openrouter.ai/api/v1    -> https://openrouter.ai/api/v1/responses
```

For Anthropic Messages:

```text
base_url + /v1/messages
```

Example:

```text
https://api.anthropic.com -> https://api.anthropic.com/v1/messages
```

::: tip Do not duplicate `/v1`

Do not include `/v1` in a custom Anthropic-style `base_url` unless the target
API specifically expects a duplicated path segment.

:::

## Custom Responses-compatible provider

Use this only for an endpoint that implements the OpenAI **Responses API** shape
used by `tuls`. Chat Completions compatibility alone is not sufficient.

```toml
model_provider = "custom"
model = "vendor/model"
base_url = "https://gateway.example/api/v1"
env_key = "GATEWAY_API_KEY"
wire_api = "responses"
```

The credential is sent as:

```text
Authorization: Bearer <credential>
```

Responses runs are stateless: every turn replays the full conversation history
as request `input`, instructions are sent as a `developer` item, and
`store: false` is set on each request.

## Custom Anthropic-Messages-compatible provider

```toml
model_provider = "custom"
model = "vendor-model"
base_url = "https://gateway.example"
env_key = "GATEWAY_API_KEY"
wire_api = "anthropic-messages"
```

The credential is sent with the Anthropic-style headers used by the runtime.
This mode is suitable only when the gateway accepts that authentication and
Messages API contract.

## Provider secrets

Do not put provider API keys into an agent TOML/Markdown file.

This is intentionally invalid:

```toml
api_key = "secret"
```

Use an environment-variable name instead:

```toml
env_key = "OPENROUTER_API_KEY"
```

and provide the secret to the `tuls agents` process:

```bash
export OPENROUTER_API_KEY='...'
tuls agents . --allow agents.run
```

::: tip Related

- [OpenRouter subagents](./openrouter) — a first-class walkthrough.
- [Subagent configuration](./subagents) — all canonical fields.

:::
