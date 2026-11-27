# logcut

[![crates.io](https://img.shields.io/crates/v/logcut.svg)](https://crates.io/crates/logcut)
[![docs.rs](https://docs.rs/logcut/badge.svg)](https://docs.rs/logcut)
[![CI](https://github.com/bigwalkdoe/logcut/actions/workflows/ci.yml/badge.svg)](https://github.com/bigwalkdoe/logcut/actions/workflows/ci.yml)
[![license](https://img.shields.io/crates/l/logcut.svg)](https://crates.io/crates/logcut)

Local-first, high-speed **stacktrace, runtime log & compiler error slicing** for AI coding agents and terminal pipelines.

`logcut` is one Rust binary with two interfaces over the same pure-Rust core:

- **`logcut mcp`** — an MCP stdio server exposing `slice_log`, `slice_build`, `slice_container`, and `slice_service`.
- **`logcut slice` / `build` / `docker` / `journal` / `ask`** — a CLI that compresses thousands of noisy lines into a surgical diagnostic report.

Secrets are redacted by default (Bearer tokens, `sk-`/`ghp_`/AWS keys, database URLs). Large files are tailed from the end in bounded chunks; stacktraces from Rust, Python, JS/TS, Go, and Java/JVM lose their vendor frames while surviving app frames are inlined with the exact crashing source lines. Compiler output from `rustc`/`cargo`, `tsc`, `go build`, and `gcc`/`clang` is collapsed into root causes with `path:line` snippets.

## Install

```bash
cargo install logcut
```

Or build from source:

```bash
cargo build --release
./target/release/logcut --help
```

Or grab a prebuilt archive from the [v0.2.0 GitHub release](https://github.com/bigwalkdoe/logcut/releases/tag/v0.2.0): Linux x86_64, macOS x86_64 and aarch64, and Windows x86_64. Each archive has a matching `.sha256` checksum file; unpack it and put the `logcut` binary on your `PATH`.

## CLI

```bash
# Pipe a failing command straight into the slicer
cargo test 2>&1 | logcut slice --stats

# Slice a file, keeping 10 lines of context around crashing frames
logcut slice /var/log/app.log --context-lines 10 --stats

# Collapse compiler error walls into root causes (rustc, tsc, go build, gcc/clang)
cargo build 2>&1 | logcut build --stats
logcut build compiler.log --workspace-root /path/to/repo --max-errors 5

# Container and systemd sources
logcut docker my-api --tail-lines 2000 --since 1h
logcut journal api.service --lines 1000

# Agentic loop (needs OPENAI_API_KEY; optional OPENAI_BASE_URL / OPENAI_MODEL)
logcut ask "why does the worker panic on startup?"
```

`--stats` writes JSON size/token statistics to **stderr**, so the report on stdout stays pipe-safe. `--workspace-root` controls which stack frames count as application code and where `path:line` snippets are read from. `build` defaults to 10 root causes (`--max-errors`) and 3 lines of source context (`--context-lines`); when reading a file it tails the last 5,000 lines unless `--tail-lines` says otherwise.

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

A compiler report looks like:

```text
== Compiler errors ==
[1] E0277 src/api.rs:42:18 — the trait bound `User: Serialize` is not satisfied
    [+3 trait/note lines collapsed]
    | serde_json::to_string(user)

== Source context ==
--- src/api.rs (39:45) ---
  39 | // line 39
> 42 | serde_json::to_string(user)
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

| Tool | Input | Result |
| --- | --- | --- |
| `slice_log` | file path or raw text | deduplicated errors, stack frames, source context |
| `slice_build` | build log path or raw text | root-cause compiler errors, collapsed cascades, source context |
| `slice_container` | container name | sliced Docker logs |
| `slice_service` | systemd unit | sliced journal logs |

All four tools accept an optional `workspace_root` for resolving `path:line` snippets.

## Working with treecut

[`treecut`](https://crates.io/crates/treecut) maps a codebase; `logcut` explains what broke at runtime. Use them together in OpenCode, Claude Code, or Cursor:

1. Ask the agent to run `treecut` on the repository to find the modules involved.
2. Reproduce the failure and pipe runtime output into `logcut slice` or compiler output into `logcut build` (or let the agent call the MCP tools).
3. `logcut` returns redacted, deduplicated errors, vendor-collapsed stack frames, collapsed compiler cascades, and inlined `path:line` snippets.
4. The agent uses the treecut map to navigate from those frames to related code, then proposes a fix.

The `/debug-log` command created by `logcut init` gives the agent that workflow in one step.

## Feature flags

| Feature | Default | Provides |
| --- | --- | --- |
| `cli` | yes | `logcut ask` agent loop (`reqwest`) |
| `mcp` | yes | `logcut mcp` stdio server (`rmcp`) |

`slice`, `build`, `docker`, `journal`, and `init` are available in every build.

```bash
cargo build --no-default-features --features mcp   # MCP-only server
```

## Documentation

Start with the [documentation index](docs/README.md):

- [Usage guide](docs/usage.md) — commands, flags, defaults, and examples
- [MCP and agent reference](docs/mcp.md) — stdio tools, host setup, and `logcut ask`
- [Architecture guide](docs/architecture.md) — modules, invariants, and extension points
- [Troubleshooting](docs/troubleshooting.md) — common symptoms and fixes

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo check --no-default-features --features mcp
cargo publish --dry-run
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup and verification, and [CHANGELOG.md](CHANGELOG.md) for release notes.

Licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
