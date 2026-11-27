# MCP and agent reference

`logcut` exposes its core tools over MCP stdio for AI coding hosts and also uses the same tools in its `ask` agent loop. For CLI flags and examples, see the [usage guide](usage.md).

## Running the server

```bash
logcut mcp
```

The server speaks JSON-RPC 2.0 over stdio and exits when the client closes stdin. **stdout is reserved for JSON-RPC messages**; diagnostics, including `logcut init` progress messages and tracing output, go to stderr with ANSI colouring disabled. If you capture stdout yourself, do not mix human-readable text into it — an MCP client will try to parse it as protocol data.

The MCP-only build is available as `cargo build --no-default-features --features mcp`; a build without the `mcp` feature reports that the feature is missing instead of starting a server.

## Tools

All four tools accept typed arguments whose JSON schemas are generated from the same Rust structs used by the core library (`#[derive(..., schemars::JsonSchema)]`), so the published schema cannot drift from the implementation. Omitted optional fields use the defaults below.

### `slice_log`

Redacts, deduplicates, and slices a log file, piped text, or raw log string.

| Input | Default | Meaning |
| --- | --- | --- |
| `path` | none | File to read from the end. Mutually exclusive with `raw_text`. |
| `raw_text` | none | Log text supplied directly. |
| `workspace_root` | `"."` | Root for app-frame classification and `path:line` snippet lookup. |
| `tail_lines` | `5000` | Maximum lines retained from file or raw text. |
| `context_lines` | `3` | Source lines on each side of a crashing frame. |
| `include_vendor_frames` | `false` | Keep vendor frames instead of collapsing consecutive runs. |
| `max_chars` | `12000` | Truncate the rendered text. |

Result: `SliceReport` with `text` (rendered report), `stats`, `frames`, `groups`, and `snippets`.

### `slice_build`

Collapses compiler error cascades into root causes with source context.

| Input | Default | Meaning |
| --- | --- | --- |
| `path` | none | Build-log file, tailed from the end. Mutually exclusive with `raw_text`. |
| `raw_text` | none | Build-log text; not tailed. |
| `workspace_root` | `"."` (core default) | Root for resolving compiler `path:line` spans. |
| `tail_lines` | `5000` | File input only. |
| `max_errors` | `10` | Maximum root-cause errors. |
| `context_lines` | `3` | Source lines on each side of an error span. |
| `max_chars` | `12000` | Truncate the rendered text. |

Result: `SliceBuildReport` with `text`, `stats`, `errors`, `snippets`, and `cascading_suppressed` (the number of cascades merged). Each `CompilerError` contains `kind`, `code`, `message`, `path`, `line`, `column`, `count`, `notes_collapsed`, and `primary_snippet`.

### `slice_container`

Slices Docker container logs through the same engine as `slice_log`.

| Input | Default | Meaning |
| --- | --- | --- |
| `container` | required | Container name passed to `docker logs --tail … --since …`. |
| `tail_lines` | `2000` | Line limit applied to the retrieved output. |
| `since` | none | Passed through to `docker logs --since`. |
| `workspace_root` | `"."` | Root for frames and snippets. |
| `max_chars` | `12000` | Truncate the rendered text. |

Result: `SliceReport`. The external command is spawned with an argument array, never a shell, so container names cannot become shell code.

### `slice_service`

Slices a systemd unit's journal through the same engine as `slice_log`.

| Input | Default | Meaning |
| --- | --- | --- |
| `unit` | required | systemd unit passed to `journalctl -u … -n … --no-pager`. |
| `lines` | `1000` | Journal line limit. |
| `workspace_root` | `"."` | Root for frames and snippets. |
| `max_chars` | `12000` | Truncate the rendered text. |

Result: `SliceReport`.

### Shared behavior

- **Redaction** runs before rendering and again over the final text, including source snippets: Bearer tokens, `sk-`/`rk-`, GitHub and AWS keys, credential-bearing database URLs, and common `api_key`/`password`/`secret` assignments become `<REDACTED>`.
- **Source context** is resolved only for application frames, against `workspace_root`; missing or unreadable files simply yield no snippet rather than an error.
- **`max_chars` truncates `text` only.** Structured fields (`frames`, `groups`, `errors`, `snippets`) are returned in full, and `stats` reports the rendered size.

### `tools/list` example

```json
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
```

The response contains a `result.tools` array; abbreviated:

```json
{"jsonrpc":"2.0","id":2,"result":{"tools":[
  {"name":"slice_log","description":"Redact, deduplicate and slice a log file, piped text, or raw log string","inputSchema":{}},
  {"name":"slice_build","description":"Collapse compiler error cascades into root causes with source context","inputSchema":{}},
  {"name":"slice_container","description":"Slice Docker container logs with the logcut engine","inputSchema":{}},
  {"name":"slice_service","description":"Slice a systemd unit's journal with the logcut engine","inputSchema":{}}
]}}
```

The real `inputSchema` objects are generated from the parameter structs; `{}` above stands in for them.

## Host setup with `logcut init`

From the root of the repository you want to debug:

```bash
logcut init --all          # or --opencode / --claude / --cursor
```

| Host | Server config | Command / rule |
| --- | --- | --- |
| OpenCode | `opencode.json` | `.opencode/commands/debug-log.md` |
| Claude Code | `.mcp.json` | `.claude/commands/debug-log.md` |
| Cursor | `.cursor/mcp.json` | `.cursor/rules/debug-log.mdc` |

Each config launches `logcut mcp` over stdio. **Existing files are kept untouched** — `init` reports `keeping existing <path>` on stderr and never overwrites your configuration, so it is safe to rerun after adding your own settings.

## `logcut ask`

```bash
export OPENAI_API_KEY=...        # required
export OPENAI_MODEL=gpt-4o-mini  # optional default
export OPENAI_BASE_URL=https://api.openai.com/v1  # optional default
logcut ask --workspace-root /path/to/repo "why does the worker panic on startup?"
```

`ask` is an OpenAI-compatible ReAct loop: the model receives tool definitions generated from the same core parameter structs, chooses tools, and the results are fed back into the conversation. It allows at most **8 tool rounds**, then fails with `agent exceeded 8 tool rounds`. It requires the `cli` feature because it uses `reqwest`; the MCP server does not.
