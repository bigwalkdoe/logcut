# Troubleshooting

Symptom-first fixes for common `logcut` problems. Command details are in the [usage guide](usage.md) and tool contracts in the [MCP and agent reference](mcp.md).

## No source snippets appear in the report

**Cause.** For `slice_log`, source context is resolved only for frames classified as application code, relative to `workspace_root`. If the root is wrong, a path is absolute and cannot be stripped, or the source file is missing, `source_ctx.rs` silently yields no snippet.

**Fix.** Pass the repository root explicitly with `--workspace-root /path/to/repo` (or the `workspace_root` tool field). Check that the path in the log actually exists under that root:

```bash
logcut slice crash.log --workspace-root /path/to/repo --context-lines 10
```

Paths with Windows separators or `file:///` prefixes are normalized before lookup; if a traceback reports a path from a different checkout, copy the source into the workspace or point `--workspace-root` at that checkout. Snippet lookup never fails the whole report — an absent file simply produces no snippet.

## The report is empty

**Cause.** `--tail-lines 0` (or `tail_lines: 0` in MCP) keeps zero lines, and an empty log naturally produces no sections.

**Fix.** Remove the zero value or raise it. File readers also return an empty string for an empty file. `slice_build` applies `--tail-lines` only to file input; raw text and stdin are used as given, so an empty report there means the input had no recognized diagnostics.

## Invalid `--min-level` or `min_level`

**Cause.** The caller supplied a severity name outside the accepted set. Validation happens in the core before input is read, so the CLI, MCP tool, and direct library callers all fail the same way. Log lines with unknown levels are still treated as `INFO`; only caller-supplied `min_level` values are strict.

**Fix.** Use one of `trace`, `debug`, `info`, `warn`, `warning`, `error`, or `fatal` (case-insensitive):

```bash
logcut slice app.jsonl --min-level warn
```

For MCP, send `"min_level": "warn"` (or omit the field). The error lists the accepted values.

## Malformed `--where` or `where_filters`

**Cause.** A filter must contain a recognized operator (`=`, `!=`, `>`, `>=`, `<`, `<=`) with a field name and value. The parser commits to the first operator it finds, so strings such as `status>= ` are rejected rather than interpreted as a comparison against `"="`.

**Fix.** Quote values that contain shell metacharacters and use a complete comparison:

```bash
logcut slice app.jsonl --where 'status>=500' --where 'service=auth'
```

For MCP, pass complete strings in `where_filters`. Values are compared case-sensitively, so `level=error` does not match `ERROR`.

## `--ring-lines 0` is rejected

**Cause.** `logcut watch` needs a non-empty ring buffer to slice a crash. A zero value is an explicit configuration error; it is not silently replaced with the default 500.

**Fix.** Omit `--ring-lines` for the default, or choose a positive value:

```bash
logcut watch --ring-lines 1000 -- cargo test
```

## The report ends with `… [truncated]`

**Cause.** `--max-chars` / `max_chars` (default 12,000) limits the rendered text. Truncation happens on a UTF-8 character boundary and adds a marker.

**Fix.** Raise the limit if you need more text. Note that **only the rendered text is truncated**: MCP and programmatic callers still receive complete `frames`, `groups`, `errors`, and `snippets` arrays. If a structured field looks short, the slicer did not find more entries — it is not a `max_chars` cut.

## Credentials show as `<REDACTED>`

**Cause.** Redaction is deliberate and happens before rendering and again over the final rendered text. Bearer tokens, `sk-`/`rk-` and GitHub/AWS keys, database URLs with userinfo, and `api_key`/`password`/`secret` assignments are masked so report text is safe to hand to an LLM or MCP client.

**Fix.** This is expected behavior, not a bug. Redaction applies to `text`; structured `snippets` contain raw source lines and are not redacted, so treat that field as sensitive if your sources hold credentials. If you need the original value in the report text, inspect the source log locally rather than asking `logcut` to reveal it. The surrounding scheme, host, and diagnostic text are preserved so the error remains understandable.

## Windows/CRLF logs look odd

**Cause.** Windows logs often arrive with `\r\n` line endings. Rust's line iteration treats CRLF as a line boundary, and the parsers work on the resulting lines.

**Fix.** `logcut` accepts CRLF input; no conversion is required. If you are adding a parser or fixture, keep assertions line-ending agnostic — CI runs on Windows as well as Linux and macOS, and the same fixture must pass on all three.

## `logcut docker` fails to find or read the container

**Cause.** The `docker` binary is missing, the daemon is unreachable, or the user lacks permission to read that container's logs.

**Fix.** Verify `docker logs <container>` works with the same user and environment. `logcut` spawns `docker logs` with an explicit argument array — there is no shell — so names containing shell metacharacters are passed literally, and `--since` is forwarded unchanged. Check that `--tail-lines` is not set to `0`, which would make the sliced output empty even when the command succeeds.

## `logcut journal` fails with a `journalctl` error

**Cause.** `journalctl` is unavailable (not a systemd host), the unit name is wrong, or the user is not in a group allowed to read the journal.

**Fix.** Run `journalctl -u <unit> -n 100 --no-pager` directly to see the underlying error, then fix the unit name or permissions. `logcut journal <UNIT> --lines <N>` passes those arguments as an array, never through a shell. On non-systemd systems, pipe those logs into `logcut slice` instead.

## `logcut ask` reports `OPENAI_API_KEY is required`

**Cause.** The agent loop reads `OPENAI_API_KEY` from the environment and requires it. `OPENAI_MODEL` defaults to `gpt-4o-mini` and `OPENAI_BASE_URL` to `https://api.openai.com/v1`.

**Fix.** Export the key, or set all three for a compatible endpoint:

```bash
export OPENAI_API_KEY=...
export OPENAI_MODEL=gpt-4o-mini
export OPENAI_BASE_URL=https://api.openai.com/v1
logcut ask "why does the worker panic on startup?"
```

`ask` needs the `cli` feature (it uses `reqwest`) and stops after 8 tool rounds.

## `logcut mcp` produces no JSON-RPC

**Cause.** The binary was built without the `mcp` feature, so the command reports that the build does not include it.

**Fix.** Install or rebuild with the feature enabled (it is on by default): `cargo build --features mcp`. Point your host at the resulting binary via `logcut init` or your own config. Remember that stdout carries JSON-RPC only; add no wrapper that prints banners or logs to stdout, or the client will fail to parse the stream.

## `--stats` is not in the piped report

**Cause.** Statistics are written to **stderr** on purpose so stdout stays a clean report for `head`, `grep`, or an MCP client.

**Fix.** Read stderr separately, for example `cargo test 2>&1 | logcut slice --stats 2>stats.json` (adjust the redirection to capture only the stats line), or simply let the terminal show it. The JSON contains `raw_bytes`, `sliced_bytes`, `estimated_tokens_saved`, and `reduction_percent`.

## A check fails locally

**Cause.** Missing Rust components, stale formatting, or a change that violates an architectural invariant.

**Fix.** Run the same gates as CI, from the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo check --no-default-features --features mcp
cargo publish --dry-run
```

The MCP-only check verifies that the server builds without `cli`/`reqwest`. See [CONTRIBUTING.md](../CONTRIBUTING.md) for fixture and release rules, and [architecture.md](architecture.md) for the invariants to preserve.

If none of the above helps, open an issue with the command, the log shape (redacted), your OS, and the exact output at <https://github.com/bigwalkdoe/logcut/issues/new/choose>.
