---
title: Development
description: Building, testing, and contributing to tuls.
---

# Development

## Required commands

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo package
cargo build --release
```

The crate forbids unsafe Rust and denies several panic-oriented Clippy patterns
in non-test builds.

## Live provider tests

Live end-to-end tests run against the real compiled binary and a real model:

```bash
export OPENROUTER_API_KEY='...'
export TULS_LIVE_MODEL='openai/gpt-5.6-luna'
TULS_LIVE=1 cargo test --test live_provider -- --nocapture
```

`TULS_LIVE` is the required gate; without it the live tests report themselves
as skipped and `cargo test` stays green offline.

## README conformance tests

The `readme` integration test suite (`tests/readme.rs`) pins documented behavior
against the real compiled binary. It is deterministic and offline: the
LLM-driven parts use a loopback mock provider instead of a real model.

## Source layout

```text
src/
├── agents/
│   ├── activity.rs
│   ├── child_mcp.rs
│   ├── definition.rs
│   ├── discovery.rs
│   ├── markdown.rs
│   ├── provider.rs
│   ├── runtime.rs
│   ├── timeouts.rs
│   └── toml.rs
├── fetch/
│   ├── http.rs
│   └── mod.rs
├── fs/
│   ├── edit.rs
│   ├── format.rs
│   ├── search.rs
│   └── mod.rs
├── memory/
│   ├── graph.rs
│   └── mod.rs
├── shell/
│   ├── drain.rs
│   └── mod.rs
├── skills/
│   ├── discovery.rs
│   ├── manifest.rs
│   ├── parser.rs
│   └── mod.rs
├── support/
│   ├── access.rs
│   └── mod.rs
├── cli.rs
├── main.rs
└── policy.rs
```

Tests live beside their corresponding modules in `tests.rs` files.

## Documentation

This site is built from `docs/` with Node tooling. The tooling lives
entirely under `docs/` (`docs/package.json`), keeping the Rust crate root
clean. Commands (from the repository root):

```bash
npm --prefix docs install
npm --prefix docs run docs:dev      # local development server
npm --prefix docs run docs:build    # production build
npm --prefix docs run docs:preview  # preview the production build
```

The equivalent commands inside `docs/` are `npm install`, `npm run docs:dev`,
`npm run docs:build`, and `npm run docs:preview`.
