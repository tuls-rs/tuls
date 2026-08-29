---
title: Provider configuration
description: Endpoints, credentials, and wire APIs for agent providers.
---

# Provider configuration

## Provider matrix

| `provider`   | Default base URL               | Default `credential_env` | Default `api`        | Authentication sent by tuls            |
| ------------ | ------------------------------ | ------------------------ | -------------------- | -------------------------------------- |
| `openai`     | `https://api.openai.com/v1`    | `OPENAI_API_KEY`         | `responses`          | `Authorization: Bearer ...`            |
| `anthropic`  | `https://api.anthropic.com`    | `ANTHROPIC_API_KEY`      | `anthropic-messages` | `x-api-key: ...` + `anthropic-version` |
| `openrouter` | `https://openrouter.ai/api/v1` | `OPENROUTER_API_KEY`     | `responses`          | `Authorization: Bearer ...`            |
| `custom`     | none                           | none                     | none                 | Determined by `api`                    |

::: danger Overrides are rejected for first-class providers

First-class providers (`openai`, `anthropic`, `openrouter`) reject `base_url`,
`credential_env`, and `api` overrides: each has a fixed endpoint, credential
variable, and wire contract. `custom` is the only way to reach a differently
shaped endpoint, and it requires `base_url`, `credential_env`, and `api` all
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

```yaml
provider: custom
model: vendor/model
base_url: https://gateway.example/api/v1
credential_env: GATEWAY_API_KEY
api: responses
```

The credential is sent as:

```text
Authorization: Bearer <credential>
```

Responses runs are stateless: every turn replays the full conversation history
as request `input`, instructions are sent as a `developer` item, and
`store: false` is set on each request.

## Custom Anthropic-Messages-compatible provider

```yaml
provider: custom
model: vendor-model
base_url: https://gateway.example
credential_env: GATEWAY_API_KEY
api: anthropic-messages
```

The credential is sent with the Anthropic-style headers used by the runtime.
This mode is suitable only when the gateway accepts that authentication and
Messages API contract.

## Provider secrets

Do not put provider API keys into an agent Markdown file.

This is intentionally invalid:

```yaml
api_key: secret
```

Use an environment-variable name instead:

```yaml
credential_env: OPENROUTER_API_KEY
```

and provide the secret to the `tuls agents` process:

```bash
export OPENROUTER_API_KEY='...'
tuls agents . --allow agents.run
```

::: tip Related

- [OpenRouter agents](./openrouter) — a first-class walkthrough.
- [Agent configuration](./subagents) — all frontmatter fields.

:::
