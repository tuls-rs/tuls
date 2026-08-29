<p align="center">
  <img src="https://raw.githubusercontent.com/tuls-rs/tuls/refs/heads/main/docs/public/logo.svg" alt="tuls" width="150" />
</p>

# tuls

> A compact Rust MCP toolbox for filesystem access, HTTP fetches, persistent memory,
> local process execution, reusable skills, and provider-backed local subagents.

**Documentation:** see the full [documentation site](https://tuls-rs.github.io/tuls/) for a
guided walkthrough, per-server references, configuration guides, and
troubleshooting.

`tuls` packages six focused MCP servers into one binary:

| Server | Purpose | Primary capability |
| --- | --- | --- |
| `filesystem` | Read, inspect, search, edit, and move workspace files | `filesystem.read`, `filesystem.write` |
| `fetch` | Fetch bounded HTTP(S) content with explicit network policy | `network.fetch` |
| `memory` | Maintain a persistent JSONL knowledge graph | `memory.read`, `memory.write` |
| `shell` | Execute programs with direct argv semantics | `process.execute` |
| `skills` | Discover and activate workspace skills | `skills.read` |
| `agents` | Run local provider-backed subagents with child MCP tools | `agents.run` |

The project is designed around **explicit capabilities**, **least privilege**,
**strict input schemas**, and **bounded I/O**. It targets MCP `2026-07-28` and
uses that protocol lifecycle only.

---

## Table of contents

- [Documentation](#documentation)
- [Why tuls](#why-tuls)
- [Requirements](#requirements)
- [Build and install](#build-and-install)
- [Quick start](#quick-start)
- [Connecting from an MCP client](#connecting-from-an-mcp-client)
- [CLI reference](#cli-reference)
- [Capability policy](#capability-policy)
- [Filesystem server](#filesystem-server)
- [Fetch server](#fetch-server)
- [Memory server](#memory-server)
- [Shell server](#shell-server)
- [Skills server](#skills-server)
- [Agents server](#agents-server)
- [Subagent configuration](#subagent-configuration)
- [Provider configuration](#provider-configuration)
- [OpenRouter subagents](#openrouter-subagents)
- [Child MCP servers](#child-mcp-servers)
- [Recommended agent profiles](#recommended-agent-profiles)
- [Workspace layout](#workspace-layout)
- [Naming conventions](#naming-conventions)
- [Security model](#security-model)
- [Limits and bounded behavior](#limits-and-bounded-behavior)
- [Troubleshooting](#troubleshooting)
- [Development](#development)

---

## Documentation

The repository ships a full documentation site under
[`docs/`](docs/), published at <https://tuls-rs.github.io/tuls/>. It includes:

- an overview and feature tour of all six servers;
- guided setup for MCP clients;
- per-server references with tool tables and JSON examples;
- subagent, provider, OpenRouter, and child MCP configuration guides;
- the security model, limits, naming conventions, and a complete
  least-privilege example;
- troubleshooting and development guides.

Run it locally (from the repository root):

```bash
npm --prefix docs install
npm --prefix docs run docs:dev
```

---

## Why tuls

`tuls` is intentionally small and opinionated. It does not try to make every
operation available to every model by default.

Core design rules:

1. **Tools are grouped by stable capabilities.** A client can grant
   `filesystem.read` without granting writes, or `network.fetch` without shell
   execution.
2. **A denied tool is not merely hidden.** Disabled routes are removed from
   discovery and rejected again at call time.
3. **Subagents are default-deny for child MCP tools.** Declaring a child MCP
   server does not automatically grant its tools to the model.
4. **Unknown public parameters fail closed.** MCP tool inputs reject unknown
   JSON fields, and canonical agent definitions reject unknown fields.
5. **Secrets stay outside agent files.** Provider credentials are read from
   environment variables; literal provider secret fields are rejected.
6. **Network and file operations are bounded.** Large bodies, large media,
   tool results, process output, and provider responses have explicit limits.
7. **MCP annotations are descriptive, not authorization.** Child tool
   annotations do not determine whether a subagent may call a tool.
8. **The shell server is not presented as a sandbox.** OS-level containment is
   a separate deployment responsibility.

---

## Requirements

| Requirement | Value |
| --- | --- |
| Rust edition | 2024 |
| Minimum Rust version | 1.98 |
| MCP protocol | `2026-07-28` |
| Primary transport for built-in servers | stdio |
| Child MCP transports | stdio, Streamable HTTP |

`tuls` supports only the MCP lifecycle implemented for `2026-07-28` and rejects
`initialize` requests. The MCP client used with this binary therefore needs to
support the same protocol lifecycle.

---

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

### Prebuilt binaries

Every [GitHub release](https://github.com/tuls-rs/tuls/releases) also attaches
prebuilt binaries for Linux, macOS, Windows, iOS, and Android targets. Install
the latest one for your platform with:

```bash
sh -c "$(curl -fsSL https://raw.githubusercontent.com/tuls-rs/tuls/main/install.sh)"
```

The `install.sh` installer is also bundled inside every release archive. Set
`TULS_RUST_LABEL=rust-stable` to prefer binaries built with the latest stable
toolchain instead of the MSRV default (`rust-msrv`).

Until the crate is published, a source checkout builds and runs now; see
[Development](#development).

---

## Quick start

### Read-only filesystem MCP

```bash
tuls filesystem /absolute/path/to/project \
  --allow filesystem.read
```

### Read/write filesystem MCP

```bash
tuls filesystem /absolute/path/to/project \
  --allow filesystem.read \
  --allow filesystem.write
```

### Public-web fetch MCP

```bash
tuls fetch \
  --allow network.fetch
```

The fetch defaults are intentionally restrictive:

```text
--robots respect
--network public
```

### Persistent memory MCP

```bash
tuls memory \
  --memory-file /absolute/path/to/memory.jsonl \
  --allow memory.read \
  --allow memory.write
```

### Local process execution MCP

```bash
tuls shell /absolute/path/to/project \
  --allow process.execute
```

### Workspace skills MCP

```bash
tuls skills /absolute/path/to/project \
  --allow skills.read
```

### Workspace subagents MCP

```bash
tuls agents /absolute/path/to/project \
  --allow agents.run
```

---

## Connecting from an MCP client

`tuls` uses stdio for its built-in servers. MCP clients commonly represent a
stdio server as a command plus an argument array. The exact configuration file
and field names are client-specific, but the process configuration is
conceptually equivalent to the following:

```json
{
  "mcpServers": {
    "project-files": {
      "command": "tuls",
      "args": [
        "filesystem",
        "/absolute/path/to/project",
        "--allow",
        "filesystem.read"
      ]
    },
    "web": {
      "command": "tuls",
      "args": [
        "fetch",
        "--allow",
        "network.fetch"
      ]
    },
    "subagents": {
      "command": "tuls",
      "args": [
        "agents",
        "/absolute/path/to/project",
        "--allow",
        "agents.run"
      ]
    }
  }
}
```

The examples use `tuls` resolved through `PATH`. GUI applications may start
with a different working directory and a different `PATH` than your
interactive shell; if a client cannot resolve `tuls`, configure it with the
resolved absolute path to the installed binary.

### Parent agent vs. subagent permissions

There are two separate permission boundaries:

```text
AI client / parent model
        |
        | connects to
        v
  tuls agents <workspace>
        |
        | spawn_agent("reviewer", ...)
        v
 provider-backed subagent
        |
        | may connect only to configured child MCP servers
        v
 filesystem / fetch / other MCP servers
```

The parent model needs `agents.run` to use `spawn_agent` and `send_input`.

The spawned subagent gets **only** the child MCP tools granted by its own
`allow_tools`/`deny_tools` configuration. These are independent policies.

---

## CLI reference

### Top-level syntax

```text
tuls <COMMAND> [OPTIONS]
```

### Server commands

| Command | Positional input | Important options |
| --- | --- | --- |
| `filesystem [DIR]...` | Zero or more allowed directories; default `.` | `--allow`, `--deny` |
| `fetch` | None | `--robots`, `--network`, `--user-agent`, `--proxy-url`, `--allow`, `--deny` |
| `memory` | None | `--memory-file`, `--allow`, `--deny` |
| `shell [DIR]...` | Zero or more allowed directories; default `.` | `--allow`, `--deny` |
| `skills [DIR]` | Workspace root; default `.` | `--allow`, `--deny` |
| `agents [DIR]` | Workspace root; default `.` | `--allow`, `--deny` |

### Shared policy options

| Option | Repeatable | Meaning |
| --- | --- | --- |
| `--allow SELECTOR` | Yes | Enables a capability or exact built-in tool. Once present, policy becomes an allowlist. |
| `--deny SELECTOR` | Yes | Denies a capability or exact built-in tool. Deny always wins. |

### Fetch-specific options

| Option | Values | Default | Meaning |
| --- | --- | --- | --- |
| `--robots` | `respect`, `ignore` | `respect` | robots.txt policy for autonomous tool calls |
| `--network` | `public`, `unrestricted` | `public` | Outbound destination policy |
| `--user-agent` | string | `tuls/<version>` | HTTP User-Agent |
| `--proxy-url` | HTTP(S) URL | none | Outbound proxy; requires `--network unrestricted` |

### Memory-specific options

| Option | Default | Meaning |
| --- | --- | --- |
| `--memory-file PATH` | `MEMORY_FILE_PATH`, otherwise `memory.jsonl` | Persistent JSONL knowledge graph path |

For `filesystem` and `shell`, relative paths resolve against the **first
configured directory**. If multiple roots are supplied, the first root remains
the default base rather than being reordered internally.

---

## Capability policy

### Canonical capabilities

| Capability | Server | Scope |
| --- | --- | --- |
| `filesystem.read` | filesystem | Read/list/search/inspect files |
| `filesystem.write` | filesystem | Create/edit/write/move files and directories |
| `network.fetch` | fetch | Fetch tool and fetch prompt |
| `memory.read` | memory | Read/search/open graph data and memory resource |
| `memory.write` | memory | Mutate graph data |
| `process.execute` | shell | Execute local programs |
| `skills.read` | skills | Activate discovered skills |
| `agents.run` | agents | Spawn agent tasks and send follow-up input |

### Exact built-in selectors

Exact selectors use:

```text
server/tool_name
```

Examples:

```text
filesystem/read_text_file
filesystem/write_file
memory/search_nodes
agents/spawn_agent
```

### Policy behavior

With no `--allow`, a directly launched server exposes all of its own tools:

```bash
tuls filesystem .
```

As soon as at least one `--allow` is present, the policy becomes an allowlist:

```bash
tuls filesystem . \
  --allow filesystem.read
```

Grant two capabilities:

```bash
tuls filesystem . \
  --allow filesystem.read \
  --allow filesystem.write
```

Grant read/write but remove one exact operation:

```bash
tuls filesystem . \
  --allow filesystem.read \
  --allow filesystem.write \
  --deny filesystem/move_file
```

Grant a single exact tool:

```bash
tuls filesystem . \
  --allow filesystem/read_text_file
```

Invalid capabilities, misspelled tool IDs, and capabilities that do not belong
to the selected server are rejected at startup.

> **Resource/prompt note:** An auxiliary MCP surface (the `fetch` prompt, the
> `memory://knowledge-graph` resource) is enabled only when the controlling
> capability is itself granted **and** the controlling tool is not explicitly
> denied. Grant `memory.read` for the memory resource and `network.fetch` for
> the fetch prompt. An exact single-tool grant (for example
> `--allow fetch/fetch`) does not grant the capability, so the surface stays
> disabled; conversely `--deny fetch/fetch` or `--deny memory/read_graph`
> disables the surface even when the capability is granted.

---

## Filesystem server

Start the server:

```bash
tuls filesystem <DIR>... [--allow SELECTOR]... [--deny SELECTOR]...
```

### Tools

| Tool | Capability | Purpose |
| --- | --- | --- |
| `read_text_file` | `filesystem.read` | Read bounded UTF-8 text; supports `head` or `tail` |
| `read_media_file` | `filesystem.read` | Read a bounded media file into typed MCP content |
| `read_multiple_files` | `filesystem.read` | Read a bounded batch of files |
| `list_directory` | `filesystem.read` | List directory entries |
| `list_directory_with_sizes` | `filesystem.read` | List entries with sizes and sort by name/size |
| `directory_tree` | `filesystem.read` | Build a bounded tree representation |
| `search_files` | `filesystem.read` | Search by glob-style pattern |
| `get_file_info` | `filesystem.read` | Return filesystem metadata |
| `list_allowed_directories` | `filesystem.read` | Show configured roots |
| `write_file` | `filesystem.write` | Atomically replace file contents |
| `edit_file` | `filesystem.write` | Apply structured edits or preview a diff |
| `create_directory` | `filesystem.write` | Create directories |
| `move_file` | `filesystem.write` | Move a file or directory inside allowed scope |

### MCP JSON naming

Public tool JSON uses camelCase:

```json
{
  "path": "src/main.rs",
  "head": 120
}
```

```json
{
  "path": ".",
  "sortBy": "size"
}
```

```json
{
  "path": ".",
  "pattern": "**/*.rs",
  "excludePatterns": ["target/**"]
}
```

Unknown fields are rejected rather than ignored.

### Path behavior

- All paths must remain under at least one allowed root.
- Relative paths resolve against the first configured root.
- Existing paths are canonicalized before authorization.
- Symlink entries are listed as symlinks; size listing does not follow a
  symlink just to obtain target metadata.
- Writes use a same-directory temporary file and rename for atomic replacement.
- `edit_file` matches `oldText` exactly, with no whitespace or line-ending
  normalization (`\n` never matches `\r\n`), and each `oldText` must occur
  exactly once in the file; a missing or ambiguous match fails the call
  without writing.

Example review-only scope:

```bash
tuls filesystem /work/project \
  --allow filesystem.read
```

Example implementation scope without moves:

```bash
tuls filesystem /work/project \
  --allow filesystem.read \
  --allow filesystem.write \
  --deny filesystem/move_file
```

---

## Fetch server

Start the server:

```bash
tuls fetch [OPTIONS]
```

### Tool

| Tool | Capability | Input |
| --- | --- | --- |
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

`raw: false` converts HTML to Markdown where applicable. `raw: true` returns
page content without that simplification.

`maxLength` accepts 1–50,000 characters (default 5,000) and is a character
window for the rendered result, not a network safety limit; the raw response
body is bounded independently (8 MiB).

### Default network posture

```text
robots: respect
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

For hostnames, addresses are resolved and validated before the request and the
validated addresses are pinned to the request client. Redirects are disabled,
so a request cannot authorize one destination and then automatically follow a
redirect to another.

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

This restriction exists because proxy-side DNS/routing cannot be constrained
by the local public-destination check.

### robots.txt

Autonomous `fetch` tool calls obey the configured `--robots` policy. The MCP
fetch prompt represents an explicit user-initiated fetch and does not apply
robots.txt, while still using the configured network policy and response
limits.

---

## Memory server

Start the server:

```bash
tuls memory \
  --memory-file /work/state/memory.jsonl \
  --allow memory.read \
  --allow memory.write
```

If `--memory-file` is omitted:

1. `MEMORY_FILE_PATH` is used when present;
2. otherwise `memory.jsonl` is used in the process working directory.

### Tools

| Tool | Capability | Purpose |
| --- | --- | --- |
| `read_graph` | `memory.read` | Read the graph |
| `search_nodes` | `memory.read` | Search names, types, and observations |
| `open_nodes` | `memory.read` | Open named entities |
| `create_entities` | `memory.write` | Add entities |
| `create_relations` | `memory.write` | Add relations |
| `add_observations` | `memory.write` | Append observations to entities |
| `delete_entities` | `memory.write` | Delete entities |
| `delete_observations` | `memory.write` | Delete selected observations |
| `delete_relations` | `memory.write` | Delete relations |

The server also exposes the knowledge graph as:

```text
memory://knowledge-graph
```

Grant `memory.read` when resource access is required.

### Persistence

The graph is stored as JSONL and mutations rewrite a complete validated graph
atomically. Existing Unix file permissions are preserved when replacing an
existing regular file.

No-op mutations — duplicate entity/relation/observation creates, or deletes
that match nothing — change nothing: they do not rewrite the file and do not
send resource-update notifications.

Read-only memory:

```bash
tuls memory \
  --memory-file /work/state/memory.jsonl \
  --allow memory.read
```

---

## Shell server

Start the server:

```bash
tuls shell <DIR>... \
  --allow process.execute
```

### Tool

| Tool | Capability | Purpose |
| --- | --- | --- |
| `execute_command` | `process.execute` | Execute one program directly with an argv array |

Input example:

```json
{
  "program": "cargo",
  "args": ["test", "--workspace"],
  "cwd": ".",
  "timeoutMs": 120000
}
```

Important semantics:

- `program` is an executable, **not** a shell command string;
- `args` are passed exactly as individual argv values;
- there is no shell parsing, quoting, glob expansion, pipelines, `&&`, or
  variable expansion performed by `tuls`;
- default timeout is 120 seconds;
- maximum timeout is 600 seconds;
- stdout and stderr are captured independently and bounded to 8 KiB each;
- child processes receive a minimal inherited environment;
- on Unix, timeout termination kills the whole process group, so descendants
  are terminated too; on Windows only the direct child is terminated, and a
  descendant process tree is not guaranteed to be killed.

If shell syntax is intentionally required, invoke the shell explicitly and
accept the corresponding risk, for example:

```json
{
  "program": "bash",
  "args": ["-lc", "cargo test && cargo clippy"],
  "cwd": "."
}
```

`execute_command` requires the **MCP Tasks extension**: the client must declare
the `io.modelcontextprotocol/tasks` client capability. The call returns a
standard task handle immediately; poll `tasks/get` for status and `tasks/cancel`
to terminate the process early. The completed task result carries a structured
`CommandOutput` (`exitCode`, `stdout`, `stderr`, `timedOut`,
`stdoutTruncated`, `stderrTruncated`).

### Shell is not a filesystem sandbox

Allowed directories constrain the command's **working directory**. They do not
constrain the program's syscalls.

A spawned process can still access any path, network destination, device, or
other OS resource available to the account running `tuls`. For untrusted
models, run the shell MCP process inside an OS/container sandbox with explicit
filesystem and network policy. See [`SECURITY.md`](SECURITY.md).

---

## Skills server

Skills are discovered from the workspace:

```text
.agents/skills/<skill-name>/SKILL.md
.claude/skills/<skill-name>/SKILL.md
```

The canonical project location is `.agents/skills`. The `.claude/skills`
location is supported as a vendor adapter; canonical definitions take
precedence on collisions.

Start the server:

```bash
tuls skills /work/project \
  --allow skills.read
```

### Tool

| Tool | Capability | Purpose |
| --- | --- | --- |
| `activate_skill` | `skills.read` | Load one discovered skill's full instructions and resource manifest |

A skill's supporting files are not automatically injected into the model
context. The activation result provides resource paths so the model can read
only what is needed.

Example layout:

```text
.agents/
└── skills/
    └── rust-review/
        ├── SKILL.md
        ├── checklist.md
        └── examples/
            └── review.md
```

---

## Agents server

The `agents` server lets a parent MCP client spawn named workspace agents.
Each agent has:

- model/provider configuration;
- instructions;
- optional skills;
- an explicit child MCP tool allowlist;
- optional child MCP stdio/HTTP servers;
- a bounded maximum number of provider turns.

Start it with:

```bash
tuls agents /work/project \
  --allow agents.run
```

### Parent-facing tools

Both tools require the **MCP Tasks extension**: the client must declare the
`io.modelcontextprotocol/tasks` client capability, otherwise the server rejects
the call. Each call returns immediately with a standard task handle
(`resultType: "task"`).

| Tool | Capability | Purpose |
| --- | --- | --- |
| `spawn_agent` | `agents.run` | Start a named agent on a task as an MCP Task |
| `send_input` | `agents.run` | Continue an existing agent conversation as a new MCP Task |

### Typical parent flow

```text
1. spawn_agent(name="reviewer", task="Review src/fetch for SSRF issues")
2. poll tasks/get(taskId) until the task settles
3. read the terminal task result: agentId, agent name, final response
4. optionally send_input(target=<agentId>, ...) for follow-up turns
5. cancel an in-flight turn with tasks/cancel
```

The `taskId` returned by `spawn_agent` is distinct from the agent session
`agentId`; the session `agentId` appears in the structured content of the
terminal task result and is the target for `send_input`. While a task runs,
`tasks/get` reports `statusMessage` updates (starting child MCP servers, model
turn, tool execution). A completed turn settles with a `CallToolResult`:
successful runs carry `{agentId, name, result}` in `structuredContent`, failed
runs carry `{agentId, name, kind, message, resumable}` with `isError: true`.

The runtime supports multiple concurrent subagents up to its configured runtime
capacity.

Terminal sessions stay resumable: `send_input` on a completed agent starts a
new run that continues the retained conversation. Failed sessions are resumable
only for resumable error kinds (interrupts, transient provider errors, and
child MCP startup failures). Context limits, invalid provider requests, missing
provider credentials, and ambiguous tool execution mark the session
**non-resumable**. A session is resumable only after its last turn settles;
starting a replacement turn while one is still running is rejected.

### Agent discovery

Canonical definitions are discovered recursively under:

```text
.agents/agents/
```

Supported canonical formats:

```text
*.toml
*.md
```

A Claude-compatible Markdown adapter is also discovered under:

```text
.claude/agents/
```

Canonical `.agents/agents` definitions have higher precedence if the same agent
name appears in multiple discovery roots.

### Workspace trust

Agent definitions under `.agents/agents/` and `.claude/agents/` are
**executable trusted configuration**: they name the provider endpoint and
credential variable, and stdio child MCP entries declare commands that `tuls`
executes locally under its own OS identity. Running `tuls agents` against a
workspace therefore executes configuration shipped in that repository.

A custom provider endpoint receives the credential named by that definition's
`env_key`. `${NAME}` interpolation in child MCP environment values and headers
exposes the specifically named parent-process environment value.

Point the agents server only at workspaces you trust. An untrusted repository
can define agents that make credentialed provider calls and run arbitrary
local commands, so treat an untrusted repository the same as untrusted code.

---

## Subagent configuration

Canonical TOML is the recommended format for repository-owned agents because it
is explicit, strict, and easy to review.

### Minimal OpenAI agent

Create:

```text
.agents/agents/reviewer.toml
```

```toml
name = "reviewer"
description = "Reviews workspace code without modifying files"
instructions = "Review the requested code and report concrete correctness, security, and maintainability issues."

model_provider = "openai"
model = "YOUR_OPENAI_MODEL"

allow_tools = ["filesystem/*"]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = ["filesystem", ".", "--allow", "filesystem.read"]
```

Before starting `tuls agents`, make sure the provider credential exists in its
environment:

```bash
export OPENAI_API_KEY='...'
tuls agents . --allow agents.run
```

### Minimal Anthropic agent

```toml
name = "reviewer-anthropic"
description = "Reviews workspace code using Anthropic"
instructions = "Review the requested code. Do not modify files."

model_provider = "anthropic"
model = "YOUR_ANTHROPIC_MODEL"

allow_tools = ["filesystem/*"]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = ["filesystem", ".", "--allow", "filesystem.read"]
```

Credential:

```bash
export ANTHROPIC_API_KEY='...'
```

### Canonical agent fields

| Field | Required | Default | Description |
| --- | --- | --- | --- |
| `name` | Yes | — | Stable local agent identifier, 1–64 chars |
| `description` | Yes | — | Catalog description shown to the parent model, at most 4 KiB |
| `instructions` | TOML: yes | — | System/task instructions; Markdown uses body text |
| `model_provider` | Yes | — | `openai`, `anthropic`, `openrouter`, or `custom` |
| `model` | Yes | — | Provider model identifier |
| `base_url` | Custom: yes | provider default | Provider API prefix/root; rejected for first-class providers |
| `env_key` | Custom: yes | provider default | Environment variable holding the API credential; rejected for first-class providers |
| `wire_api` | Custom: yes | provider default | `responses` or `anthropic-messages`; rejected for first-class providers |
| `temperature` | No | provider default | `0..=2` for Responses, `0..=1` for Anthropic Messages |
| `reasoning_effort` | No | provider default | Wire-specific reasoning effort |
| `max_turns` | No | `32` | Provider/tool loop limit, `1..=128` |
| `allow_tools` | No | empty | Explicit child MCP grants; empty means no child tools |
| `deny_tools` | No | empty | Explicit child MCP denials; deny wins |
| `skills` | No | empty | Skills injected into the agent's system context |
| `mcp_servers` | No | empty | Named stdio or HTTP child MCP servers |

Unknown canonical fields are rejected.

### Reasoning effort

For `wire_api = "responses"`:

```text
none
minimal
low
medium
high
xhigh
```

For `wire_api = "anthropic-messages"`:

```text
low
medium
high
xhigh
max
```

Only set a value that the selected upstream model/provider actually supports.
`tuls` validates the wire-level vocabulary, while the upstream provider remains
the authority on model-specific support.

---

## Provider configuration

### Provider matrix

| `model_provider` | Default base URL | Default `env_key` | Default wire | Authentication sent by tuls |
| --- | --- | --- | --- | --- |
| `openai` | `https://api.openai.com/v1` | `OPENAI_API_KEY` | `responses` | `Authorization: Bearer ...` |
| `anthropic` | `https://api.anthropic.com` | `ANTHROPIC_API_KEY` | `anthropic-messages` | `x-api-key: ...` + `anthropic-version` |
| `openrouter` | `https://openrouter.ai/api/v1` | `OPENROUTER_API_KEY` | `responses` | `Authorization: Bearer ...` |
| `custom` | none | none | none | Determined by `wire_api` |

First-class providers (`openai`, `anthropic`, `openrouter`) reject `base_url`,
`env_key`, and `wire_api` overrides: each has a fixed endpoint, credential
variable, and wire contract. `custom` is the only way to reach a differently
shaped endpoint, and it requires `base_url`, `env_key`, and `wire_api` all
explicitly.

### `base_url` semantics

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

Do not include `/v1` in a custom Anthropic-style `base_url` unless the target
API specifically expects a duplicated path segment.

### Custom Responses-compatible provider

Use this only for an endpoint that implements the OpenAI **Responses API**
shape used by `tuls`. Chat Completions compatibility alone is not sufficient.

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

### Custom Anthropic-Messages-compatible provider

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

### Provider secrets

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

---

## OpenRouter subagents

`openrouter` is a first-class provider: the endpoint, credential variable, and
wire are fixed (`https://openrouter.ai/api/v1` + `/responses`,
`OPENROUTER_API_KEY`, Responses), so agent files never repeat them.

### 1. Export the credential

```bash
export OPENROUTER_API_KEY='...'
```

The API key is read by the `tuls agents` process when a subagent is spawned.
It does not need to be copied into the child filesystem/fetch MCP processes.

### 2. Create an OpenRouter agent

Create:

```text
.agents/agents/openrouter-researcher.toml
```

```toml
name = "openrouter-researcher"
description = "Researches public web sources through OpenRouter"
instructions = "Research the requested topic. Use fetch when evidence is needed and return concise, source-oriented findings."

model_provider = "openrouter"
model = "openai/gpt-5.6-luna"
reasoning_effort = "high"
max_turns = 32

allow_tools = ["fetch/*"]

[mcp_servers.fetch]
type = "stdio"
command = "tuls"
args = ["fetch", "--allow", "network.fetch"]
```

The resulting provider request goes to:

```text
POST https://openrouter.ai/api/v1/responses
Authorization: Bearer $OPENROUTER_API_KEY
```

OpenRouter model identifiers use provider-qualified names. Replace
`openai/gpt-5.6-luna` with the OpenRouter model you actually want to run.

### 3. Start the agents MCP server

From the workspace root:

```bash
tuls agents . --allow agents.run
```

The parent MCP client will discover `openrouter-researcher` in the
`spawn_agent` catalog.

### 4. Spawn it from the parent model

Conceptually:

```json
{
  "name": "openrouter-researcher",
  "task": "Compare the current Rust MCP ecosystem and identify the most relevant libraries."
}
```

The call returns a task handle. Poll `tasks/get` with the returned `taskId`
until the task settles, then read the terminal task result for the agent's
`agentId`, name, and final response.

### OpenRouter implementer with filesystem access

```toml
name = "openrouter-implementer"
description = "Implements scoped code changes through OpenRouter"
instructions = "Implement the requested changes. Keep edits scoped to the workspace and preserve project conventions."

model_provider = "openrouter"
model = "openai/gpt-5.6-luna"
reasoning_effort = "high"
max_turns = 48

allow_tools = ["filesystem/*"]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = [
  "filesystem",
  ".",
  "--allow",
  "filesystem.read",
  "--allow",
  "filesystem.write",
]
```

### OpenRouter researcher with fetch + read-only workspace

```toml
name = "openrouter-investigator"
description = "Combines public-web research with read-only workspace inspection"
instructions = "Investigate the task using repository evidence and public sources. Do not modify workspace files."

model_provider = "openrouter"
model = "openai/gpt-5.6-luna"
reasoning_effort = "high"

allow_tools = [
  "filesystem/*",
  "fetch/*",
]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = ["filesystem", ".", "--allow", "filesystem.read"]

[mcp_servers.fetch]
type = "stdio"
command = "tuls"
args = ["fetch", "--allow", "network.fetch"]
```

### Why Responses for OpenRouter?

`tuls` sends Responses credentials using Bearer authentication, matching
OpenRouter's Responses endpoint at `/api/v1/responses`. The wire is fixed:
`openrouter` rejects `wire_api` overrides. Do **not** expect
`anthropic-messages` for OpenRouter — that wire also changes the HTTP
authentication contract to Anthropic-style `x-api-key`, so it is intended for
endpoints that explicitly implement that contract.

---

## Child MCP servers

Subagents can use named child MCP servers.

Two transport types are supported:

```text
stdio
http
```

### stdio child MCP

```toml
[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = ["filesystem", ".", "--allow", "filesystem.read"]
```

The child process:

- runs locally as a direct exec of `command`, with the OS identity of the
  `tuls` process;
- starts in the agent workspace;
- has `kill_on_drop` enabled;
- receives a minimal inherited environment;
- does not implicitly inherit unrelated credentials;
- uses MCP `2026-07-28` discovery.

### Explicit child environment

If a child MCP server itself needs a credential, pass only that credential:

```toml
[mcp_servers.external]
type = "stdio"
command = "external-mcp"
args = ["serve"]
env = { EXTERNAL_API_KEY = "${EXTERNAL_API_KEY}" }
```

`${NAME}` placeholders selectively expose individual variables from the
`tuls agents` process environment to the child; nothing else is inherited. A
missing variable fails the child startup.

### HTTP child MCP

```toml
[mcp_servers.issues]
type = "http"
url = "https://mcp.example.com/mcp"
headers = { Authorization = "Bearer ${ISSUE_MCP_TOKEN}" }
```

HTTP child MCP clients use bounded timeouts and do not follow redirects.
Header values support the same `${NAME}` environment interpolation.

### Child tool selectors

Canonical selector format:

```text
server/tool
server/*
```

Example:

```toml
allow_tools = [
  "filesystem/read_text_file",
  "filesystem/search_files",
  "fetch/*",
]

deny_tools = [
  "fetch/some_tool_name"
]
```

Rules:

1. Empty `allow_tools` means **no child MCP tools**.
2. `server/*` grants all tools advertised by that named child server.
3. `server/tool` grants one exact child tool.
4. Deny always overrides allow.
5. A selector referencing an unknown configured server is rejected.
6. After connection, an exact selector referencing a tool not actually
   advertised by that child server is rejected.
7. Authorization is based on this explicit policy, not child-provided
   read-only/destructive annotations.
8. A child tool's reported `isError` is preserved and committed to the agent
   conversation as an error output; the run continues.
9. A call that times out or fails after dispatch has an ambiguous outcome
   (the tool may have executed) and the session is marked **non-resumable**.
10. Completed sessions are resumable: `send_input` on a completed agent starts
    a new run that continues the retained conversation.

### Defense in depth

For built-in child servers, restrict both layers.

Good:

```toml
allow_tools = ["filesystem/*"]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = ["filesystem", ".", "--allow", "filesystem.read"]
```

This means:

- the child filesystem process itself exposes only read operations;
- the subagent policy grants only tools from that child server.

Do not rely on only one of those layers for high-risk tools.

---

## Recommended agent profiles

### Code reviewer

Goal: inspect repository content without modifying it.

```toml
allow_tools = ["filesystem/*"]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = ["filesystem", ".", "--allow", "filesystem.read"]
```

### Web researcher

Goal: public-web access without filesystem or shell access.

```toml
allow_tools = ["fetch/*"]

[mcp_servers.fetch]
type = "stdio"
command = "tuls"
args = ["fetch", "--allow", "network.fetch"]
```

### Implementer

Goal: read and edit repository files without arbitrary process execution.

```toml
allow_tools = ["filesystem/*"]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = [
  "filesystem",
  ".",
  "--allow",
  "filesystem.read",
  "--allow",
  "filesystem.write",
]
```

### Test runner

Goal: run commands in addition to reading repository content.

```toml
allow_tools = [
  "filesystem/*",
  "shell/*",
]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = ["filesystem", ".", "--allow", "filesystem.read"]

[mcp_servers.shell]
type = "stdio"
command = "tuls"
args = ["shell", ".", "--allow", "process.execute"]
```

This profile is substantially more privileged because `shell` is arbitrary
local process execution under the OS account. Prefer a real OS/container
sandbox when exposing it to an autonomous model.

### Research + implementation split

For larger workflows, prefer multiple narrow agents instead of one all-powerful
agent:

```text
parent
├── researcher      -> fetch only
├── reviewer        -> filesystem.read only
└── implementer     -> filesystem.read + filesystem.write
```

This reduces tool confusion and limits the blast radius of a bad tool choice.

---

## Workspace layout

Recommended repository structure:

```text
project/
├── .agents/
│   ├── agents/
│   │   ├── reviewer.toml
│   │   ├── researcher.toml
│   │   └── implementer.toml
│   └── skills/
│       ├── rust-review/
│       │   ├── SKILL.md
│       │   └── checklist.md
│       └── release-check/
│           └── SKILL.md
├── src/
├── tests/
└── Cargo.toml
```

## Naming conventions

`tuls` intentionally uses different naming conventions at different interface
boundaries instead of mixing styles within one interface.

| Interface | Convention | Example |
| --- | --- | --- |
| Rust identifiers | `snake_case` | `max_length` |
| Canonical TOML | `snake_case` | `allow_tools` |
| CLI flags | `--kebab-case` | `--user-agent` |
| MCP JSON fields | `camelCase` | `maxLength` |
| MCP tool names | `snake_case` | `read_text_file` |
| Capabilities | `domain.action` | `filesystem.read` |
| Built-in exact selectors | `server/tool` | `filesystem/read_text_file` |
| Child MCP selectors | `server/tool`, `server/*` | `fetch/*` |

Provider-facing tool names are internally qualified so tools from different
child MCP servers remain distinguishable. Policy configuration should always
use the canonical `server/tool` form documented above.

---

## Security model

`tuls` separates **tool authorization** from **runtime containment**.

### What tuls enforces

- strict built-in capability/tool policy;
- default-deny child MCP tool policy for subagents;
- tool removal from discovery plus call-time enforcement;
- strict public MCP JSON inputs;
- canonical agent field validation;
- environment-based provider credentials;
- minimal environment inheritance for spawned commands/stdio child MCPs;
- bounded tool/provider/network outputs;
- filesystem root checks for filesystem operations;
- conservative public-network fetch policy;
- no automatic HTTP redirects in fetch/provider/child HTTP clients;
- explicit timeout handling.

### What tuls does not claim to enforce

`tuls shell` is **not** an OS sandbox. Directory roots do not restrict the
syscalls made by a spawned executable.

Filesystem path validation is designed to prevent ordinary path/symlink escape,
but path validation is not a replacement for a kernel-enforced capability
filesystem when hostile concurrent processes can mutate paths during an
operation.

For hostile or highly autonomous workloads, deploy `tuls` inside a real sandbox
and restrict:

- writable filesystem paths;
- readable secret paths;
- network destinations;
- process execution;
- environment variables;
- operating-system identity and privileges.

See [`SECURITY.md`](SECURITY.md) for the security boundary and deployment
recommendations.

---

## Limits and bounded behavior

Selected implementation limits:

| Area | Limit / behavior |
| --- | --- |
| Generic tool text result | 64 KiB |
| Fetch raw response body | 8 MiB |
| Fetch URL | 8,192 characters |
| Fetch `maxLength` | 1–50,000 characters (default 5,000) |
| Fetch request timeout | 30 seconds |
| Fetch DNS resolution timeout | 10 seconds |
| Fetch redirects | disabled |
| Media file input | 1 MiB |
| Text file input | 8 MiB |
| Multi-file read batch | 32 files |
| `edit_file` batch | 1,024 operations and 8 MiB total edit text |
| Shell stdout capture | 8 KiB |
| Shell stderr capture | 8 KiB |
| Shell default timeout | 120 seconds |
| Shell maximum timeout | 600 seconds |
| Provider response body | 8 MiB |
| Agent spawn task / `send_input` message | 256 KiB each |
| Agent turn execution limit | 30 minutes |
| Agent task TTL | 35 minutes |
| Agent result size | 24 KiB |
| Retained idle agent sessions | 64 |
| Discovered agents | 256 |
| Agent catalog description | 4 KiB |
| Agent file / generated catalog | 1 MiB / 64 KiB |
| Discovered skills | 256 |
| Skill description / generated catalog | 4 KiB / 64 KiB |
| Activated skill / resource manifest | 1 MiB / 64 KiB |
| Skills per agent | 32 |
| Built agent context (instructions + skills) | 1 MiB |
| Canonical agent file | 1 MiB |
| Default provider turns | 32 |
| Maximum provider turns | 128 |
| Agent runtime concurrent capacity | 8 |
| Memory file | 8 MiB |
| Individual memory text field | 16 KiB |

These limits are safety boundaries, not tuning recommendations. Applications
that need materially larger payloads should change them deliberately and review
the impact on model context size, memory use, latency, and denial-of-service
exposure.

---

## Troubleshooting

### `unknown capability` or `unknown tool policy selector`

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

Use a capability from the capability table or an exact `server/tool` ID.

### Subagent appears in the catalog but has no tools

This is expected when `allow_tools` is empty.

Declaring:

```toml
[mcp_servers.filesystem]
...
```

does **not** grant access. Add an explicit policy:

```toml
allow_tools = ["filesystem/*"]
```

### `child tool selector references unknown MCP server`

The part before `/` must exactly match a key under `[mcp_servers.<name>]`.

This must match:

```toml
allow_tools = ["repo/*"]

[mcp_servers.repo]
...
```

### `child tool selector references unavailable tool`

An exact selector points to a tool that the child did not advertise.

Check both:

1. the exact child tool name;
2. whether the child process's own `--allow`/`--deny` policy disabled it.

### Agent reports missing environment variable

`OPENROUTER_API_KEY` is the default credential variable for
`model_provider = "openrouter"`, so it must exist in the environment of the
**`tuls agents` process**.

Check before launching the MCP client/process:

```bash
printenv OPENROUTER_API_KEY
```

For GUI MCP clients, configure secrets using that client's environment/secret
mechanism rather than assuming the GUI inherited your terminal session.

### OpenRouter returns an HTTP error

`openrouter` is first-class: the request goes to
`https://openrouter.ai/api/v1/responses` with Bearer auth and the credential
from `OPENROUTER_API_KEY`. Overrides are rejected, so a custom-style
`base_url`/`env_key`/`wire_api` in the agent file is a configuration error.

Verify that `OPENROUTER_API_KEY` is set in the `tuls agents` process and that
the selected OpenRouter model supports the behavior needed by the agent,
especially tool calling and any requested reasoning parameters.

### Custom provider returns an error at `/responses`

`wire_api = "responses"` requires a Responses-compatible endpoint, not merely
an OpenAI Chat Completions-compatible endpoint.

### Child MCP cannot see an environment variable

stdio child MCP processes deliberately start with a minimal environment. Pass
required variables explicitly:

```toml
env = { TOKEN = "${TOKEN}" }
```

### `shell` command works in a terminal but not through tuls

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

### Relative paths resolve somewhere unexpected

For `filesystem` and `shell`, relative paths resolve against the first root:

```bash
tuls filesystem /work/project /work/shared
```

Here `src/main.rs` resolves relative to `/work/project`.

### Fetch cannot access localhost/private services

That is the default `--network public` policy.

For an explicitly trusted deployment that needs private network access:

```bash
tuls fetch \
  --network unrestricted \
  --allow network.fetch
```

Treat unrestricted network access as a meaningful privilege increase.

---

## Development

Required commands:

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

### Live provider tests

Live end-to-end tests run against the real compiled binary and a real model:

```bash
export OPENROUTER_API_KEY='...'
export TULS_LIVE_MODEL='openai/gpt-5.6-luna'
TULS_LIVE=1 cargo test --test live_provider -- --nocapture
```

`TULS_LIVE` is the required gate; without it the live tests report themselves
as skipped and `cargo test` stays green offline.

### Source layout

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

---

## Example: complete least-privilege OpenRouter setup

This example gives a parent model access only to the agents orchestration
surface. The spawned agent can read repository files and fetch public web
content, but cannot write files or execute arbitrary processes.

### Workspace

```text
project/
└── .agents/
    └── agents/
        └── investigator.toml
```

### `.agents/agents/investigator.toml`

```toml
name = "investigator"
description = "Investigates repository issues using read-only files and public web research"
instructions = "Inspect repository evidence first. Use public web research only when needed. Do not modify files and do not execute local programs."

model_provider = "openrouter"
model = "openai/gpt-5.6-luna"
reasoning_effort = "high"
max_turns = 32

allow_tools = [
  "filesystem/*",
  "fetch/*",
]

[mcp_servers.filesystem]
type = "stdio"
command = "tuls"
args = ["filesystem", ".", "--allow", "filesystem.read"]

[mcp_servers.fetch]
type = "stdio"
command = "tuls"
args = ["fetch", "--allow", "network.fetch"]
```

### Start environment

```bash
cd /absolute/path/to/project
export OPENROUTER_API_KEY='...'
tuls agents . --allow agents.run
```

### Effective permissions

| Layer | Granted | Not granted |
| --- | --- | --- |
| Parent MCP surface | `agents.run` | filesystem, fetch, memory, shell directly |
| Subagent child policy | `filesystem/*`, `fetch/*` | shell, memory, undeclared child servers |
| Child filesystem server | `filesystem.read` | `filesystem.write` |
| Child fetch server | `network.fetch`, public-network default | private network, redirects |
| OS process boundary | normal account permissions | **not sandboxed by tuls** |

This layered model is the recommended pattern: grant the model only the tool
families it needs, and independently restrict each child MCP process to the
minimum operation set required for its role.
