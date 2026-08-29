---
title: Troubleshooting
description: Common problems and their fixes.
---

# Troubleshooting

## `unknown capability` or `unknown tool policy selector`

Policy selectors are strict.

Correct:

```text
filesystem.read
filesystem/read_text_file
```

Incorrect examples:

```text
filesystem-read
filesystem.read_text_file
filesystem/read-file
```

Use a capability from the [capability table](./guide/capability-policy) or an
exact `server/tool` ID.

## Agent appears in the catalog but has no tools

This is expected when `tools` is empty (default deny).

Declaring:

```yaml
mcp_servers:
  filesystem:
    type: stdio
    command: tuls
    args: ["filesystem", ".", "--allow", "filesystem.read"]
```

does **not** grant access. Add an explicit policy:

```yaml
tools:
  - filesystem/*
```

## `child tool selector references unknown MCP server`

The part before `/` must exactly match a key under `mcp_servers`.

This must match:

```yaml
tools:
  - repo/*
mcp_servers:
  repo:
    type: stdio
    command: repo-mcp
    args: ["serve"]
```

## `child tool selector references unavailable tool`

An exact selector points to a tool that the child did not advertise.

Check both:

1. the exact child tool name;
2. whether the child process's own `--allow`/`--deny` policy disabled it.

## Agent reports missing environment variable

`OPENROUTER_API_KEY` is the default credential variable for
`provider: openrouter`, so it must exist in the environment of the
**`tuls agents` process**.

Check before launching the MCP client/process:

```bash
printenv OPENROUTER_API_KEY
```

For GUI MCP clients, configure secrets using that client's environment/secret
mechanism rather than assuming the GUI inherited your terminal session.

## OpenRouter returns an HTTP error

`openrouter` is first-class: the request goes to
`https://openrouter.ai/api/v1/responses` with Bearer auth and the credential
from `OPENROUTER_API_KEY`. Overrides are rejected, so a custom-style
`base_url`/`credential_env`/`api` in the agent file is a configuration error.

Verify that `OPENROUTER_API_KEY` is set in the `tuls agents` process and that
the selected OpenRouter model supports the behavior needed by the agent,
especially tool calling and any requested reasoning parameters.

## Custom provider returns an error at `/responses`

`api: responses` requires a Responses-compatible endpoint, not merely
an OpenAI Chat Completions-compatible endpoint.

## Child MCP cannot see an environment variable

stdio child MCP processes deliberately start with a minimal environment. Pass
required variables explicitly:

```yaml
env: { TOKEN: "${TOKEN}" }
```

## `shell` command works in a terminal but not through tuls

Remember:

- `program` is an executable name;
- `args` are separate argv entries;
- no shell syntax is interpreted unless you explicitly run a shell;
- spawned processes use a reduced environment.

For example, use:

```json
{
  "program": "cargo",
  "args": ["test"]
}
```

not:

```json
{
  "program": "cargo test"
}
```

## Relative paths resolve somewhere unexpected

For `filesystem` and `shell`, relative paths resolve against the first root:

```bash
tuls filesystem /work/project /work/shared
```

Here `src/main.rs` resolves relative to `/work/project`.

## Fetch cannot access localhost/private services

That is the default `--network public` policy.

For an explicitly trusted deployment that needs private network access:

```bash
tuls fetch \
  --network unrestricted \
  --allow network.fetch
```

Treat unrestricted network access as a meaningful privilege increase.
