# Security model

`tuls` uses layered least privilege. A capability policy decides which MCP operations are exposed, while operating-system permissions remain the final boundary for local filesystem and process access.

## Authorization

Built-in tools have static capability metadata. MCP tool annotations are descriptive hints for clients and are never used as authorization input.

For direct servers, disabled routes are removed from tool discovery and rejected at call time. Local agents use a separate child-MCP policy that defaults to no granted tools. Child selectors are `server/tool` or `server/*` identifiers and are checked against the configured and discovered child catalogs.

Provider credentials are resolved from the `tuls agents` process environment and sent only to the configured provider endpoint, using the wire's authentication form: Bearer for Responses, `x-api-key` plus `anthropic-version` for Anthropic Messages.

## Filesystem

Configured roots are canonicalized for validation while preserving the first configured root as the base for relative paths. Requested existing paths are canonicalized before use, and targets outside the allowed roots are rejected.

Directory size listings use symlink metadata rather than following symlink targets. Recursive search and tree traversal validate entries against the filesystem scope before exposing or traversing them. Writes use same-directory temporary files and atomic rename with bounded content.

Filesystem policy is not a substitute for OS account permissions against adversarial local processes. If the MCP process itself is untrusted, place it inside an OS sandbox or container.

## Network fetch

`--network public` is the default. It permits only globally routable public destinations, rejects special-purpose literal addresses (including IPv4-mapped and translation/transition IPv6 ranges), and validates every DNS result before opening a connection, with a bounded 10-second DNS resolution timeout. Validated DNS addresses are pinned for the request. Redirects are disabled, which keeps each network operation to one validated target.

`--network unrestricted` intentionally permits destinations reachable by the process. Proxy use requires unrestricted mode.

`--robots respect` is the default for autonomous fetch tool calls. User-initiated fetch prompts skip robots policy but do not bypass network policy or response-size bounds.

## Process execution

The shell server executes a local program directly with explicit argv. It does not interpret a shell command string unless a shell is explicitly selected as the program.

Child processes inherit only a minimal platform environment by default. This reduces accidental credential propagation but is not secret isolation: processes retain the OS permissions of the `tuls` process and can read files or access networks permitted by the operating system.

On timeout, termination covers the whole process group on Unix, so descendants are also killed; on Windows only the direct child is terminated and a descendant process tree is not guaranteed to be killed. `execute_command` is a direct exec, never a sandbox.

Use an external OS/container sandbox for untrusted execution. Grant `process.execute` only to agents that require it.

## Child MCP processes

Child stdio MCP commands run locally as direct process execution under the `tuls` OS identity. They also start with a minimal inherited environment. Environment entries configured for a child MCP are added explicitly and may interpolate process environment variables; each `${NAME}` placeholder exposes exactly that one variable, and nothing else is inherited implicitly. Treat every explicitly configured variable as a credential grant.

Agent definitions under `.agents/agents/` are executable trusted configuration: they name provider endpoints, credential variables, and child commands that `tuls` runs. Run the agents server only in workspaces you trust; treat an untrusted repository like untrusted code.

A custom provider endpoint receives the credential selected by its `credential_env`. First-class OpenAI, Anthropic, and OpenRouter providers use fixed endpoint, credential-variable, wire, and authentication contracts and reject those overrides.

Remote child MCP endpoints require HTTPS except for loopback HTTP, reject embedded URL credentials, do not follow redirects, and have bounded startup/call timeouts and schema/catalog sizes.

## Persistent memory

Memory files and tool results are bounded. Mutations are serialized within a server instance and persistence uses atomic replacement; no-op mutations (duplicates or unmatched deletes) do not rewrite the file. Request batch limits are independent of total graph limits.

## Recommended deployment

Use separate `tuls` processes for distinct privilege domains. Give research agents network fetch without filesystem writes, reviewers filesystem read without process execution, and implementation agents only the write/execute capabilities they actually need. Keep credentials out of generic child process environments and prefer narrowly scoped provider or MCP-specific grants.
