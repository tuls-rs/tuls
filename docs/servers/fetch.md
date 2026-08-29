---
title: Fetch server
description: Bounded HTTP(S) fetches with explicit network policy.
---

# Fetch server

Start the server:

```bash
tuls fetch [OPTIONS]
```

## Tool

| Tool    | Capability      | Input                                   |
| ------- | --------------- | --------------------------------------- |
| `fetch` | `network.fetch` | `url`, `maxLength`, `startIndex`, `raw` |

Example tool arguments:

```json
{
  "url": "https://example.com/docs",
  "maxLength": 20000,
  "startIndex": 0,
  "raw": false
}
```

- `raw: false` converts HTML to Markdown where applicable.
- `raw: true` returns page content without that simplification.
- `maxLength` accepts 1–50,000 characters (default 5,000) and is a character
  window for the rendered result, **not** a network safety limit; the raw
  response body is bounded independently (8 MiB).
- Truncated results include a continuation hint; call `fetch` again with the
  suggested `startIndex` to retrieve more content.

For web search, fetch DuckDuckGo Lite results with
`https://lite.duckduckgo.com/lite/?q=<query>&kl=en-us&kp=0` (`kl` is the
region, `kp` is the safe-search policy; `kp=0` disables safe search).

## Default network posture

```text
robots: ignore
network: public
redirects: disabled
DNS resolution timeout: 10 seconds
request timeout: 30 seconds
raw response body limit: 8 MiB
```

`--network public` permits only globally routable public destinations and
blocks special-purpose destinations such as:

- loopback addresses;
- RFC1918/private IPv4;
- IPv6 unique-local and link-local ranges;
- IPv6 site-local, translation, transition, and discard-only ranges;
- multicast and unspecified addresses;
- documentation/test ranges;
- local hostnames such as `localhost` and `.local` names;
- IPv4-mapped IPv6 representations of blocked IPv4 addresses.

For hostnames, addresses are resolved and validated **before** the request and
the validated addresses are pinned to the request client. Redirects are
disabled, so a request cannot authorize one destination and then automatically
follow a redirect to another.

## Unrestricted mode

Use unrestricted mode only when a separate network boundary already provides
the required containment:

```bash
tuls fetch \
  --network unrestricted \
  --allow network.fetch
```

A proxy is accepted only in unrestricted mode:

```bash
tuls fetch \
  --network unrestricted \
  --proxy-url https://proxy.example \
  --allow network.fetch
```

::: warning Why the restriction?

This restriction exists because proxy-side DNS/routing cannot be constrained by
the local public-destination check. Treat unrestricted network access as a
meaningful privilege increase.

:::

## robots.txt

Autonomous `fetch` tool calls obey the configured `--robots` policy. When
`--robots respect` is active, fetches fail closed on unavailable or redirected
robots policies.

The MCP fetch prompt represents an explicit user-initiated fetch and does not
apply robots.txt, while still using the configured network policy and response
limits.

::: tip Related

- [CLI reference](../guide/cli-reference) — `--robots`, `--network`, `--user-agent`, `--proxy-url`.
- [Limits & bounded behavior](../concepts/limits) — fetch limits in one table.

:::
