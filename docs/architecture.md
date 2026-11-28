# Architecture guide

This guide is for contributors changing `logcut`. It explains how the transports share one core, where each responsibility lives, and what must stay true when extending the code. Product-level vision is in [SPEC.md](../SPEC.md); verification and release rules are in [CONTRIBUTING.md](../CONTRIBUTING.md).

## One core, three transports

```text
CLI (src/main.rs) ──┐
MCP (src/mcp.rs) ───┼──> src/core/ ──> redacted, bounded diagnostic reports
Agent (src/agent.rs) ┘
```

- **`src/main.rs`** parses CLI arguments and renders reports. It owns all stdout writes for CLI commands and routes diagnostics to stderr.
- **`src/mcp.rs`** registers six tools (`slice_log`, `slice_test`, `slice_build`, `slice_container`, `slice_service`, `get_last_crash`) and serves JSON-RPC 2.0 over stdio. Only protocol messages reach stdout.
- **`src/agent.rs`** runs an OpenAI-compatible ReAct loop. Tool definitions are generated from the same parameter structs used by MCP, so the agent cannot call a different schema than an MCP client.

None of these transports contains slicing logic. They construct typed request structs, call a core function, and present the result.

## Core modules

| Module | Responsibility |
| --- | --- |
| `models.rs` | Input/output contracts. Parameter structs derive `Debug`, `Clone`, `Deserialize`, `Serialize`, and `schemars::JsonSchema`; defaults (5,000/2,000/1,000/3/10/12,000) live here. |
| `redactor.rs` | Regex-based scrubbing of Bearer tokens, API keys, AWS keys, credential URLs, and key/value secrets. Idempotent; applied before rendering and again over final text. |
| `dedup.rs` | Groups repeated lines after normalizing timestamps, UUIDs, pointers, request IDs, and durations into placeholders; produces `[xN | first: … | last: …]` bursts. |
| `stacktrace.rs` | Parses Rust, Python, JS/TS, Go, and Java panics/tracebacks, classifies frames as app or vendor, and collapses consecutive vendor frames. |
| `compiler.rs` | Parses `rustc`/`cargo`, `tsc`, `go build`, and `gcc`/`clang` diagnostics, merges cascades into root causes with occurrence counts, and reports `cascading_suppressed`. |
| `json_log.rs` | Detects NDJSON, strips static metadata recursively, keeps non-JSON lines as context, validates and applies `min_level`/`where_filters`, redacts fields, and correlates up to five breadcrumbs per error. |
| `test_runner.rs` | Strips passing test lines, preserves failure diffs, and resolves `file:line` failures to source snippets. |
| `watch.rs` | Streams a child command into a bounded ring buffer, detects crashes, and atomically writes `.logcut/last-crash.md`; printing is injected by the CLI sink. |
| `source_ctx.rs` | Maps surviving `path:line` frames to real source lines using a bounded `BufReader` and normalizes absolute paths against `workspace_root`. |
| `sources.rs` | Bounded reverse-seek tail reader for files, tail for in-memory text, and safe Docker/journal readers that spawn commands with argument arrays (no shell). |
| `mod.rs` | Public facade: `slice_log`, `slice_test`, `slice_build`, `slice_container`, and `slice_service`. |

## Data flow

### `slice_log`

1. Select input: file path → reverse-seek tail; raw text → bounded in-memory tail; otherwise error.
2. Redact the input. If it is NDJSON, normalize fields in `json_log.rs`, apply `min_level`/`where_filters`, attach correlated breadcrumbs, render the structured section, and return early with `json_entries` populated.
3. For plain text, split into lines and group near-duplicate bursts.
4. Parse stack frames, classify against `workspace_root`, and collapse vendor runs.
5. Resolve surviving app frames to source snippets with `context_lines` on each side.
6. Render sections (bursts, stacktrace, source context), redact again, truncate `text` at `max_chars`, and compute `SliceStats`.
7. Return `SliceReport { text, stats, frames, groups, snippets, json_entries }` (`json_entries` is empty for plain text).

`slice_container` and `slice_service` fetch text through `sources.rs`, then reuse this same pipeline with their own line defaults.

### `slice_build`

1. Select input: file path → reverse-seek tail with `tail_lines` (default 5,000); raw text → used as given, **not** tailed.
2. Redact, parse compiler diagnostics, and merge cascades.
3. Keep at most `max_errors` root causes (default 10) and resolve each `path:line` to a snippet via `source_ctx`.
4. Render root causes, cascade counts, and source context; redact again; truncate `text` at `max_chars`.
5. Return `SliceBuildReport { text, stats, errors, snippets, cascading_suppressed }`.

### `slice_test` and `watch`

`slice_test` tails the input, redacts it, drops passing lines from cargo/pytest/jest/go output, groups the surviving failure blocks, resolves `path:line` references through `source_ctx`, and returns `SliceTestReport { text, stats, failures, passing_lines_stripped }`.

`watch` spawns a command with an argument array, reads stdout and stderr concurrently into a bounded ring buffer, and lets the CLI inject a sink for live pass-through. On a crash marker or non-zero exit it slices the buffered tail with `slice_log` and publishes both `.logcut/last-crash.md` and `.logcut/.gitignore` (containing `*`) through the same atomic temp-file+rename helper. `get_last_crash` reads the report for MCP clients.

Truncation always applies to rendered text only; structured fields are returned in full.

### Hard invariants

1. **Strict `stdout` Purity in MCP Mode**: Never use `println!` or write logs to `stdout` in `core/` or `mcp.rs`. Route `tracing_subscriber` exclusively to `std::io::stderr` with `.with_ansi(false)`.
2. **Zero UI/Network Coupling in `core/`**: `src/core/` must never import `clap`, `rmcp`, or `reqwest`. Every core function takes typed Rust structs and returns `anyhow::Result<T>` where `T: Serialize`.
3. **Single Source of Truth for Tool Schemas**: All MCP tool parameter structs in `src/core/models.rs` must derive `#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]`.
4. **Secret & PII Redaction by Default**: Automatically mask Bearer tokens, API keys (`sk-...`, `ghp_...`), AWS keys, and database connection URLs (`postgres://user:pass@...`) before returning sliced logs to any LLM or MCP client.
5. **Streaming / Bounded Memory Reads**: Never load multi-gigabyte log files blindly into memory; tail/window large files safely from the end (`BufReader` / reverse seek) when `tail_lines` is set.

## Feature flags

| Feature | Default | Gates |
| --- | --- | --- |
| `cli` | yes | `logcut ask` and its `reqwest` dependency. |
| `mcp` | yes | `logcut mcp` and its `rmcp` dependency. |

`slice`, `test`, `watch`, `build`, `docker`, `journal`, and `init` compile in every configuration. The MCP-only build must compile without `cli`/`reqwest`: `cargo check --no-default-features --features mcp`.

## Where to extend

- **New parser or language**: add detection and frame/error extraction in `stacktrace.rs` or `compiler.rs`, then add a small deterministic fixture in `tests/fixtures/` and assertions in `tests/core_tests.rs`. Keep assertions line-ending agnostic for Windows CI.
- **New source**: add a bounded reader in `sources.rs`, a request struct with `JsonSchema` in `models.rs`, a facade function in `mod.rs`, and a CLI subcommand in `main.rs`. Spawn external commands with argument arrays, never a shell.
- **New MCP tool**: define the parameter and result structs in `models.rs`, register the tool in `mcp.rs`, expose it in the agent's generated tool definitions when the tool is safe for autonomous use (the v0.3.0 `slice_test`/`get_last_crash` tools are MCP-only), and reuse core functions rather than duplicating logic.
- **New redaction shape**: add a conservative regex in `redactor.rs` and a test proving idempotence and that surrounding diagnostics survive.

## Verification

Run the same checks as CI before proposing a change:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo check --no-default-features --features mcp
cargo publish --dry-run
```

See [troubleshooting.md](troubleshooting.md) when a check or a report looks wrong.
