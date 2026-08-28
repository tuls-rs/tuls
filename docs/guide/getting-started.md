---
title: Getting started
description: Requirements, build, and installation for tuls.
---

# Getting started

`tuls` is a single Rust binary that packages six focused MCP servers. This page
covers what you need to build it and how to install it.

## Requirements

| Requirement                            | Value                  |
| -------------------------------------- | ---------------------- |
| Rust edition                           | 2024                   |
| Minimum Rust version                   | 1.98                   |
| MCP protocol                           | `2026-07-28`           |
| Primary transport for built-in servers | stdio                  |
| Child MCP transports                   | stdio, Streamable HTTP |

::: warning One protocol lifecycle only

`tuls` supports **only** the MCP lifecycle implemented for `2026-07-28` and
rejects `initialize` requests for other revisions. The MCP client used with this
binary must support the same protocol lifecycle.

:::

## Build and install

The crate is not yet published to crates.io. Once the first release is
published, install it with:

```bash
cargo install tuls
```

The `tuls` binary is then available on your `PATH`. Check the CLI:

```bash
tuls --help
```

Expected top-level commands:

```text
filesystem
fetch
memory
shell
skills
agents
```

Until the crate is published, a source checkout builds and runs now. See
[Development](../development) for the full development workflow.

## Prebuilt binaries

Every [GitHub release](https://github.com/tuls-rs/tuls/releases) attaches
prebuilt binaries for Linux, macOS, Windows, iOS, and Android targets. Install
the latest one for your platform with:

```bash
sh -c "$(curl -fsSL https://raw.githubusercontent.com/tuls-rs/tuls/main/install.sh)"
```

The `install.sh` installer is also bundled inside every release archive.
Binaries default to the MSRV build (`rust-msrv`); set
`TULS_RUST_LABEL=rust-stable` to prefer the latest stable toolchain build.

## Next steps

- Run through the [quick start](./quick-start) for copy-paste server launches.
- Learn how to [connect tuls from an MCP client](./connecting-clients).
- Understand [capability policy](./capability-policy) before granting tools.
