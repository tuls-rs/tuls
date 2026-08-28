---
title: Servers overview
description: The six MCP servers packaged in the tuls binary.
---

# Servers overview

`tuls` packages six focused MCP servers into one binary:

| Server                     | Purpose                                                    | Primary capability                    |
| -------------------------- | ---------------------------------------------------------- | ------------------------------------- |
| [filesystem](./filesystem) | Read, inspect, search, edit, and move workspace files      | `filesystem.read`, `filesystem.write` |
| [fetch](./fetch)           | Fetch bounded HTTP(S) content with explicit network policy | `network.fetch`                       |
| [memory](./memory)         | Maintain a persistent JSONL knowledge graph                | `memory.read`, `memory.write`         |
| [shell](./shell)           | Execute programs with direct argv semantics                | `process.execute`                     |
| [skills](./skills)         | Discover and activate workspace skills                     | `skills.read`                         |
| [agents](./agents)         | Run local provider-backed subagents with child MCP tools   | `agents.run`                          |

Each server:

- speaks MCP `2026-07-28` over stdio;
- starts with a capability policy you control via `--allow`/`--deny`;
- bounds every network, file, and tool output it produces;
- rejects unknown JSON fields on public tool inputs.

<ServerCard
  name="filesystem"
  tagline="Read, inspect, search, edit, and move workspace files inside explicitly allowed roots."
  command="tuls filesystem /path/to/project --allow filesystem.read"
  :capabilities="['filesystem.read', 'filesystem.write']"
  link="/servers/filesystem"
  accent="#10b981"
/>

<ServerCard
  name="fetch"
  tagline="Bounded HTTP(S) fetches with robots.txt and public-network policy, no redirects by default."
  command="tuls fetch --allow network.fetch"
  :capabilities="['network.fetch']"
  link="/servers/fetch"
  accent="#0ea5e9"
/>

<ServerCard
  name="memory"
  tagline="A persistent JSONL knowledge graph with entities, relations, and observations."
  command="tuls memory --memory-file memory.jsonl --allow memory.read --allow memory.write"
  :capabilities="['memory.read', 'memory.write']"
  link="/servers/memory"
  accent="#8b5cf6"
/>

<ServerCard
  name="shell"
  tagline="Execute one program directly with an argv array — no shell parsing, bounded output."
  command="tuls shell /path/to/project --allow process.execute"
  :capabilities="['process.execute']"
  link="/servers/shell"
  accent="#f43f5e"
/>

<ServerCard
  name="skills"
  tagline="Discover and activate workspace skills with their instructions and resource manifests."
  command="tuls skills /path/to/project --allow skills.read"
  :capabilities="['skills.read']"
  link="/servers/skills"
  accent="#f59e0b"
/>

<ServerCard
  name="agents"
  tagline="Run local provider-backed subagents with their own child MCP tool policies."
  command="tuls agents /path/to/project --allow agents.run"
  :capabilities="['agents.run']"
  link="/servers/agents"
  accent="#14b8a6"
/>
