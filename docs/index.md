---
layout: home

hero:
  name: "tuls"
  text: "A compact Rust MCP toolbox"
  tagline: >-
    Six focused MCP servers in one binary — filesystem access, bounded HTTP
    fetches, persistent memory, local process execution, reusable skills, and
    provider-backed local agents. Designed around explicit capabilities,
    least privilege, and bounded I/O.
  image:
    src: /logo-mark.svg
    alt: tuls
  actions:
    - theme: brand
      text: Get started
      link: /guide/getting-started
    - theme: alt
      text: Quick start
      link: /guide/quick-start
    - theme: alt
      text: Capability policy
      link: /guide/capability-policy

features:
  - title: Explicit capabilities
    details: >-
      Grant filesystem.read without writes, or network.fetch without process
      execution. A denied tool is removed from discovery and rejected again at
      call time.
  - title: Six servers, one binary
    details: >-
      filesystem, fetch, memory, shell, skills, and agents ship together and
      share one strict policy engine, one protocol lifecycle (MCP 2026-07-28),
      and one security model.
  - title: Default-deny agents
    details: >-
      Declaring a child MCP server never grants its tools to an agent.
      Every grant is explicit, per-server, and per-tool.
  - title: Bounded I/O everywhere
    details: >-
      Files, media, fetch bodies, tool results, process output, and provider
      responses all carry explicit limits — safety boundaries, not tuning
      suggestions.
  - title: Secrets stay outside config
    details: >-
      Provider credentials come from environment variables. Literal secret
      fields in agent definitions are rejected.
  - title: Strict input schemas
    details: >-
      Unknown JSON fields fail closed on public MCP tool inputs, and unknown
      fields are rejected in agent definitions.
---

<div class="home-section">

## The six servers

One binary, six focused MCP servers. Each one exposes only the capability you
grant it.

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
  tagline="Run local provider-backed agents with their own child MCP tool policies."
  command="tuls agents /path/to/project --allow agents.run"
  :capabilities="['agents.run']"
  link="/servers/agents"
  accent="#14b8a6"
/>

</div>

<div class="home-section">

## Core design rules

<small class="home-section-subtitle">tuls is intentionally small and opinionated. These rules shape every server.</small>

<div class="design-rules">

<div class="rule-card">

<strong>Tools are grouped by stable capabilities</strong>

A client can grant `filesystem.read` without granting writes, or `network.fetch`
without shell execution.

</div>

<div class="rule-card">

<strong>A denied tool is not merely hidden</strong>

Disabled routes are removed from discovery **and** rejected again at call time.

</div>

<div class="rule-card">

<strong>Agents are default-deny for child MCP tools</strong>

Declaring a child MCP server does not automatically grant its tools to the model.

</div>

<div class="rule-card">

<strong>Unknown public parameters fail closed</strong>

MCP tool inputs reject unknown JSON fields; agent definitions reject
unknown fields.

</div>

<div class="rule-card">

<strong>Secrets stay outside agent files</strong>

Provider credentials are read from environment variables; literal provider
secret fields are rejected.

</div>

<div class="rule-card">

<strong>Network and file operations are bounded</strong>

Large bodies, media, tool results, process output, and provider responses all
have explicit limits.

</div>

<div class="rule-card">

<strong>MCP annotations are descriptive, not authorization</strong>

Child tool annotations never determine whether an agent may call a tool.

</div>

<div class="rule-card">

<strong>The shell server is not presented as a sandbox</strong>

OS-level containment is a separate deployment responsibility.

</div>

</div>

</div>
