---
title: Limits & bounded behavior
description: Every safety boundary enforced by tuls, in one table.
---

# Limits & bounded behavior

Selected implementation limits:

| Area                                        | Limit / behavior                           |
| ------------------------------------------- | ------------------------------------------ |
| Generic tool text result                    | 64 KiB                                     |
| Fetch raw response body                     | 8 MiB                                      |
| Fetch URL                                   | 8,192 characters                           |
| Fetch `maxLength`                           | 1–50,000 characters (default 5,000)        |
| Fetch request timeout                       | 30 seconds                                 |
| Fetch DNS resolution timeout                | 10 seconds                                 |
| Fetch redirects                             | disabled                                   |
| Media file input                            | 1 MiB                                      |
| Text file input                             | 8 MiB                                      |
| Multi-file read batch                       | 32 files                                   |
| `edit_file` batch                           | 1,024 operations and 8 MiB total edit text |
| Shell stdout capture                        | 8 KiB                                      |
| Shell stderr capture                        | 8 KiB                                      |
| Shell default timeout                       | 120 seconds                                |
| Shell maximum timeout                       | 600 seconds                                |
| Provider response body                      | 8 MiB                                      |
| Agent spawn task / `send_input` message     | 256 KiB each                               |
| Agent turn execution limit                  | 30 minutes                                 |
| Agent task TTL                              | 35 minutes                                 |
| Agent result size                           | 24 KiB                                     |
| Retained idle agent sessions                | 64                                         |
| Discovered agents                           | 256                                        |
| Agent catalog description                   | 4 KiB                                      |
| Agent file / generated catalog              | 1 MiB / 64 KiB                             |
| Discovered skills                           | 256                                        |
| Skill description / generated catalog       | 4 KiB / 64 KiB                             |
| Activated skill / resource manifest         | 1 MiB / 64 KiB                             |
| Skills per agent                            | 32                                         |
| Built agent context (instructions + skills) | 1 MiB                                      |
| Agent markdown file                         | 1 MiB                                      |
| Default provider turns                      | 32                                         |
| Maximum provider turns                      | 128                                        |
| Agent runtime concurrent capacity           | 8                                          |
| Memory file                                 | 8 MiB                                      |
| Individual memory text field                | 16 KiB                                     |

::: warning Safety boundaries, not tuning recommendations

These limits are safety boundaries, not tuning recommendations. Applications
that need materially larger payloads should change them deliberately and review
the impact on model context size, memory use, latency, and denial-of-service
exposure.

:::

::: tip Related

- [Security model](./security-model) — the enforcement model behind these limits.

:::
