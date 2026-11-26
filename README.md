# logcut

Local-first, high-speed **stacktrace & runtime log slicing** for AI coding agents and terminal pipelines.

`logcut` is one Rust binary with two interfaces over the same pure-Rust core:

- **`logcut mcp`** — an MCP stdio server exposing `slice_log`, `slice_container`, and `slice_service`.
- **`logcut slice` / `docker` / `journal` / `ask`** — a CLI that compresses thousands of noisy lines into a surgical diagnostic report.

Secrets are redacted by default (Bearer tokens, `sk-`/`ghp_`/AWS keys, database URLs). Large files are tailed from the end in bounded chunks; stacktraces from Rust, Python, JS/TS, Go, and Java/JVM lose their vendor frames while surviving app frames are inlined with the exact crashing source lines.

## Install

```bash
cargo install logcut
```

Or build from source:

```bash
cargo build --release
./target/release/logcut --help
```

## CLI

```bash
# Pipe a failing command straight into the slicer
cargo test 2>&1 | logcut slice --stats

# Slice a file, keeping 10 lines of context around crashing frames
logcut slice /var/log/app.log --context-lines 10 --stats

# Container and systemd sources
logcut docker my-api --tail-lines 2000 --since 1h
logcut journal api.service --lines 1000

# Agentic loop (needs OPENAI_API_KEY; optional OPENAI_BASE_URL / OPENAI_MODEL)
logcut ask "why does the worker panic on startup?"
```

`--stats` writes JSON size/token statistics to **stderr**, so the report on stdout stays pipe-safe. `--workspace-root` controls which stack frames count as application code and where `path:line` snippets are read from.

A report looks like:

```text
== Error bursts ==
[x3 | first: 02:14:02 | last: 02:14:58] ERROR Connection refused to <UUID>

== Stacktrace ==
thread 'main' panicked at src/handler.rs:42:9:
... [3 framework/runtime frames hidden] ...
at logcut::handler::process (src/handler.rs:42:9)

== Source context ==
--- src/handler.rs (39:45) ---
  39 | // line 39
> 42 | let conn = connect();
```

## MCP setup

Install config and slash commands for every supported host at once:

```bash
logcut init --all          # or --opencode / --claude / --cursor
```

This generates:

| Host | Server config | Slash command |
| --- | --- | --- |
| OpenCode | `opencode.json` | `.opencode/commands/debug-log.md` |
| Claude Code | `.mcp.json` | `.claude/commands/debug-log.md` |
| Cursor | `.cursor/mcp.json` | `.cursor/rules/debug-log.mdc` |

Existing files are left untouched. Each config launches `logcut mcp` over stdio; JSON-RPC lives on stdout and all diagnostics go to stderr.

## Working with treecut

[`treecut`](https://crates.io/crates/treecut) maps a codebase; `logcut` explains what broke at runtime. Use them together in OpenCode, Claude Code, or Cursor:

1. Ask the agent to run `treecut` on the repository to find the modules involved.
2. Reproduce the failure and pipe it into `logcut slice` (or let the agent call the MCP tools).
3. `logcut` returns redacted, deduplicated errors, vendor-collapsed stack frames, and inlined `path:line` snippets.
4. The agent uses the treecut map to navigate from those frames to related code, then proposes a fix.

The `/debug-log` command created by `logcut init` gives the agent that workflow in one step.

## Feature flags

| Feature | Default | Provides |
| --- | --- | --- |
| `cli` | yes | `slice`, `docker`, `journal`, `ask` (`reqwest`) |
| `mcp` | yes | `logcut mcp` stdio server (`rmcp`) |

```bash
cargo build --no-default-features --features mcp   # MCP-only server
```

## Development

```bash
cargo test
cargo publish --dry-run
```

Licensed under MIT or Apache-2.0, at your option.
