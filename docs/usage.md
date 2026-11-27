# CLI usage guide

`logcut` is one binary with a CLI for pipelines and an MCP server for AI hosts. This guide covers the CLI; see the [MCP and agent reference](mcp.md) for host setup and tool contracts.

## Install and feature flags

```bash
cargo install logcut
# or, from a checkout:
cargo build --release && ./target/release/logcut --help
```

Prebuilt archives for Linux x86_64, macOS x86_64/aarch64, and Windows x86_64 are attached to the [v0.2.0 release](https://github.com/bigwalkdoe/logcut/releases/tag/v0.2.0), each with a `.sha256` checksum.

| Feature | Default | Provides |
| --- | --- | --- |
| `cli` | yes | `logcut ask` (pulls in `reqwest`) |
| `mcp` | yes | `logcut mcp` (pulls in `rmcp`) |

`slice`, `build`, `docker`, `journal`, and `init` are available in every build. To build an MCP-only server:

```bash
cargo build --no-default-features --features mcp
```

## Choosing an input source

`logcut slice` accepts one of:

- **A file path** — `logcut slice app.log`. File input is read from the end in bounded chunks (see [Bounded reads](#bounded-reads-and-output-limits)).
- **`--raw-text`** — inline text, e.g. `logcut slice --raw-text $'panic at src/main.rs:12'`. If both a file and `--raw-text` are supplied to `slice`, the file takes precedence.
- **Piped stdin** — when no file and no `--raw-text` are given and stdin is not a terminal, stdin is read to a string: `cargo test 2>&1 | logcut slice`.

`logcut build` accepts a file or piped stdin; it has no `--raw-text` flag:

```bash
cargo build 2>&1 | logcut build
logcut build compiler.log
```

The MCP `slice_build` tool does accept a `raw_text` field; see the [MCP reference](mcp.md#slice_build).

`logcut docker <CONTAINER>` calls `docker logs`; `logcut journal <UNIT>` calls `journalctl`. Both pass arguments as an explicit array — container and unit names are never interpreted by a shell — and the resulting output goes through the same slicer as piped text.

## Commands and flags

`--stats` is global and can appear before or after the subcommand.

### `logcut slice [FILE]`

| Flag | Default | Meaning |
| --- | --- | --- |
| `--raw-text <TEXT>` | none | Slice literal text instead of a file or stdin. |
| `--workspace-root <DIR>` | `.` | Root used to classify app vs. vendor frames and to resolve `path:line` snippets. |
| `--tail-lines <N>` | `5000` | Keep at most the last N lines. `0` yields an empty slice. |
| `--context-lines <N>` | `3` | Lines of source context on each side of a crashing frame. |
| `--include-vendor-frames` | off | Keep framework/runtime frames instead of collapsing consecutive runs. |
| `--max-chars <N>` | `12000` | Truncate the rendered report text to roughly N characters. |
| `--stats` | off | Write JSON size/token statistics to stderr. |

### `logcut build [FILE]`

| Flag | Default | Meaning |
| --- | --- | --- |
| `--workspace-root <DIR>` | `.` (core default) | Root for resolving compiler `path:line` spans to source snippets. |
| `--tail-lines <N>` | `5000` | Keep at most the last N lines **when reading a file**; stdin is not tailed (nor is the MCP tool's `raw_text`). |
| `--max-errors <N>` | `10` | Maximum number of root-cause errors reported. |
| `--context-lines <N>` | `3` | Lines of source context on each side of an error span. |
| `--max-chars <N>` | `12000` | Truncate the rendered report text. |
| `--stats` | off | Write JSON statistics to stderr. |

### `logcut docker <CONTAINER>`

| Flag | Default | Meaning |
| --- | --- | --- |
| `--tail-lines <N>` | `2000` | Passed to the slicer after `docker logs` returns. |
| `--since <TIME>` | none | Passed through to `docker logs --since`. |
| `--workspace-root <DIR>` | `.` | Root for frame classification and snippets. |
| `--max-chars <N>` | `12000` | Truncate the rendered report text. |
| `--stats` | off | Write JSON statistics to stderr. |

### `logcut journal <UNIT>`

| Flag | Default | Meaning |
| --- | --- | --- |
| `--lines <N>` | `1000` | Passed to `journalctl -n`. |
| `--workspace-root <DIR>` | `.` | Root for frame classification and snippets. |
| `--max-chars <N>` | `12000` | Truncate the rendered report text. |
| `--stats` | off | Write JSON statistics to stderr. |

### `logcut ask [QUESTION...]`

Runs a bounded ReAct loop that gives the model the same four core tools used by MCP. It requires `OPENAI_API_KEY`; `OPENAI_MODEL` defaults to `gpt-4o-mini` and `OPENAI_BASE_URL` to `https://api.openai.com/v1`. See the [agent section](mcp.md#logcut-ask).

| Flag | Default | Meaning |
| --- | --- | --- |
| `--workspace-root <DIR>` | `.` | Root available to the agent's tool calls. |

### `logcut init`

Generates MCP config entries and `/debug-log` commands for AI hosts.

| Flag | Meaning |
| --- | --- |
| `--opencode` | Write OpenCode config and command. |
| `--claude` | Write Claude Code config and command. |
| `--cursor` | Write Cursor config and rule. |
| `--all` | Write all three sets (same as passing all flags). |

With no flags, `init` writes all three sets. Existing files are left untouched, so rerunning `init` will not overwrite your setup.

## Four worked examples

```bash
# 1. Compress a failing test run; statistics go to stderr, report to stdout.
cargo test 2>&1 | logcut slice --stats

# 2. Slice a large file with wider source context from an explicit workspace root.
logcut slice /var/log/app.log --workspace-root /srv/app --tail-lines 2000 --context-lines 10

# 3. Reduce a compiler error wall to ten root causes.
cargo build 2>&1 | logcut build --max-errors 10 --stats

# 4. Inspect a container and a systemd unit.
logcut docker my-api --since 1h --tail-lines 2000
logcut journal api.service --lines 1000
```

## Reports and statistics

A `slice` report has sections for deduplicated error bursts, surviving stack frames, and inlined source context; a `build` report has compiler root causes, cascade counts, and source context. Both send the human-readable report to **stdout**.

With `--stats`, a single JSON line goes to **stderr**:

```json
{"raw_bytes":12345,"sliced_bytes":2345,"estimated_tokens_saved":2500,"reduction_percent":81.0}
```

`raw_bytes` counts the input after tailing, `sliced_bytes` counts the rendered text, and estimated token savings are computed as bytes saved divided by four. Keeping statistics on stderr means the report on stdout remains pipe-safe.

## Redaction

Redaction happens before rendering and again over the final rendered text. By default, `logcut`:

- masks Bearer tokens as `Bearer <REDACTED>`;
- masks `sk-`/`rk-` OpenAI-style keys, `gh[pousr]_` GitHub tokens, and `AKIA`/`ASIA` AWS access keys;
- keeps the scheme and host of credential-bearing database URLs (`postgres`, `mysql`, `mongodb`, `redis`, `amqp`) but replaces userinfo with `<REDACTED>@`;
- masks `api_key`, `access_token`, `password`, and `secret` key/value pairs.

This is deliberate: sliced logs are routinely pasted to an LLM. Note that redaction applies to `text`; the structured `snippets` field contains raw source lines and is not redacted, so treat that field as sensitive if your sources hold credentials.

## Bounded reads and output limits

File input is tailed by seeking backwards in 8 KiB chunks; the reader never seeks from the beginning or allocates the whole file. `tail_lines = 0` produces an empty slice. Container and journal sources apply their own line limits (2,000 and 1,000 by default) before slicing.

`--max-chars` truncates **rendered text only**. Structured result fields (`frames`, `groups`, `errors`, `snippets`) are not truncated, so MCP clients and programmatic callers still receive the full structures; a CLI user sees a `… [truncated]` marker at the cut point. For `slice_build`, the 5,000-line tail applies **only to file input** — piped output and the MCP tool's `raw_text` are used as given. The `build` CLI has no `--raw-text` flag.

## Working with treecut

[`treecut`](https://crates.io/crates/treecut) maps a codebase; `logcut` explains what broke. In OpenCode, Claude Code, or Cursor:

1. Run `treecut` to find the modules likely involved.
2. Reproduce the failure and pipe runtime output into `logcut slice`, or compiler output into `logcut build` (or let the agent call the MCP tools).
3. Use the redacted, deduplicated report with vendor-collapsed frames and inlined `path:line` snippets.
4. Use the treecut map to navigate from those frames to related code and propose a fix.

`logcut init` installs a `/debug-log` command that gives the agent this workflow in one step.
