---
title: Shell server
description: Execute programs with direct argv semantics.
---

# Shell server

Start the server:

```bash
tuls shell <DIR>... \
  --allow process.execute
```

## Tool

| Tool              | Capability        | Purpose                                         |
| ----------------- | ----------------- | ----------------------------------------------- |
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

## Important semantics

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

## Structured result

The tool returns a structured `CommandOutput`:

| Field             | Meaning                                                 |
| ----------------- | ------------------------------------------------------- |
| `exitCode`        | Numeric exit code, or `null` when terminated on timeout |
| `stdout`          | Captured standard output, lossy UTF-8, bounded to 8 KiB |
| `stderr`          | Captured standard error, lossy UTF-8, bounded to 8 KiB  |
| `timedOut`        | True when the process was terminated after `timeoutMs`  |
| `stdoutTruncated` | True when stdout exceeded the 8 KiB capture limit       |
| `stderrTruncated` | True when stderr exceeded the 8 KiB capture limit       |

## Shell is not a filesystem sandbox

::: danger Important

Allowed directories constrain the command's **working directory**. They do not
constrain the program's syscalls.

:::

A spawned process can still access any path, network destination, device, or
other OS resource available to the account running `tuls`. For untrusted
models, run the shell MCP process inside an OS/container sandbox with explicit
filesystem and network policy. See the [security model](../concepts/security-model).

::: tip Related

- [Agent profiles](../configuration/agent-profiles) — the test runner profile uses shell.
- [Limits & bounded behavior](../concepts/limits) — timeouts and capture limits.

:::
