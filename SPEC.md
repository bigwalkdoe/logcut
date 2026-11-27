# Project Master Specification: `logcut`

## 1. Mission & Product Identity
Build **`logcut`**, a high-speed, local-first Rust developer tool for **Stacktrace, Runtime Log & Compiler Error Slicing**.
`logcut` compiles into a single unified binary that operates in two modes over one shared, pure-Rust core engine:
1. **MCP Server Mode (`logcut mcp`)**: Exposes structured stacktrace extraction, compiler error cascade slicing, error burst deduplication, and Docker/systemd log slicing over `stdio` JSON-RPC 2.0 for AI coding hosts (OpenCode, Claude Code, Cursor).
2. **Agentic & Pipeline CLI Mode (`logcut slice` / `logcut build` / `logcut docker` / `logcut journal` / `logcut ask`)**: Accepts piped command output (`cargo test 2>&1 | logcut slice`, `cargo build 2>&1 | logcut build`), log files, or container names, compressing thousands of noisy lines into a surgical diagnostic report.

---

## 2. Hard Architectural Invariants (Do Not Violate)
1. **Strict `stdout` Purity in MCP Mode**: Never use `println!` or write logs to `stdout` in `core/` or `mcp.rs`. Route `tracing_subscriber` exclusively to `std::io::stderr` with `.with_ansi(false)`.
2. **Zero UI/Network Coupling in `core/`**: `src/core/` must never import `clap`, `rmcp`, or `reqwest`. Every core function takes typed Rust structs and returns `anyhow::Result<T>` where `T: Serialize`.
3. **Single Source of Truth for Tool Schemas**: All MCP tool parameter structs in `src/core/models.rs` must derive `#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]`.
4. **Secret & PII Redaction by Default**: Automatically mask Bearer tokens, API keys (`sk-...`, `ghp_...`), AWS keys, and database connection URLs (`postgres://user:pass@...`) before returning sliced logs to any LLM or MCP client.
5. **Streaming / Bounded Memory Reads**: Never load multi-gigabyte log files blindly into memory; tail/window large files safely from the end (`BufReader` / reverse seek) when `tail_lines` is set.

---

## 3. Directory & Module Architecture

```text
logcut/
├── Cargo.toml
├── README.md
├── SPEC.md
├── CHANGELOG.md
├── CONTRIBUTING.md
├── .gitattributes
├── .github/ISSUE_TEMPLATE/      # Bug report and feature request forms
├── .github/dependabot.yml
├── .github/workflows/ci.yml     # fmt, clippy, tests, MCP-only build, cross-OS release artifacts
├── docs/superpowers/plans/      # Implementation plans
├── src/
│   ├── main.rs                  # CLI router (`slice`, `build`, `docker`, `journal`, `ask`, `mcp`, `init`)
│   ├── lib.rs                   # Re-exports `core` for integration tests
│   ├── core/
│   │   ├── mod.rs               # Public API facade
│   │   ├── models.rs            # Input/Output structs with JsonSchema + SliceStats
│   │   ├── compiler.rs          # Multi-compiler diagnostic parser & cascade deduplication
│   │   ├── redactor.rs          # Regex-based secret/credential scrubber
│   │   ├── dedup.rs             # Error clustering & template normalization (timestamps, UUIDs, pointers)
│   │   ├── stacktrace.rs        # Multi-language panic/traceback parser & vendor-frame filter
│   │   ├── source_ctx.rs        # Maps surviving app frames (`path:line`) to real source code snippets
│   │   └── sources.rs           # Safe readers for files, raw text, Docker CLI, & journalctl
│   ├── mcp.rs                   # rmcp stdio server exposing core tools
│   └── agent.rs                 # OpenAI-compatible ReAct CLI loop
└── tests/
    ├── fixtures/                # Panics, tracebacks, compiler errors, & noisy JSON logs
    └── core_tests.rs            # Deterministic unit, redaction, dedup, & MCP stdio handshake tests
```

---

## 4. `Cargo.toml` Specification

```toml
[package]
name = "logcut"
version = "0.2.0"
edition = "2024"
description = "Local-first stacktrace & runtime log slicer for AI coding agents and terminal pipelines"
license = "MIT OR Apache-2.0"
keywords = ["mcp", "logs", "stacktrace", "debugging", "cli"]
categories = ["command-line-utilities", "development-tools"]
documentation = "https://docs.rs/logcut"
repository = "https://github.com/bigwalkdoe/logcut"
readme = "README.md"

[features]
default = ["cli", "mcp"]
cli = ["dep:reqwest"]
mcp = ["dep:rmcp"]

[dependencies]
anyhow = "1.0"
clap = { version = "4.5", features = ["derive", "env"] }
regex = "1.11"
schemars = "1.0"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "io-std", "io-util", "process"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }

# Optional interface layers
rmcp = { version = "3.9", features = ["server", "macros", "transport-io"], optional = true }
reqwest = { version = "0.13", features = ["json"], optional = true }

[dev-dependencies]
tempfile = "3.15"
```

---

## 5. Core Capabilities & Exact Tool Contracts

### Multi-Language Stacktrace & Panic Engine (`src/core/stacktrace.rs`)
Parse stack frames, classify each frame as **`AppFrame`** (inside workspace root) vs. **`VendorFrame`** (stdlib, runtime, third-party), and collapse consecutive vendor frames into a single summary line (`... [14 framework/runtime frames hidden] ...`):
* **Rust**: `thread '...' panicked at path/file.rs:line:col` + backtrace frames (`at ./src/...` vs `tokio::`, `std::`, `/rustc/`).
* **Python**: `Traceback (most recent call last):` + `File "...", line N, in func` (filtering `site-packages/`, `/usr/lib/python`).
* **JavaScript / TypeScript**: `Error: ...` + `at fn (path:line:col)` (filtering `node_modules/`, `node:internal/`).
* **Go**: `panic: ...` + `goroutine N [running]:` file/line pairs (filtering `runtime/`, `net/http/`).
* **Java / JVM**: `Exception in thread ...` + `at pkg.Class.method(File.java:line)` (filtering `java.*`, `sun.*`, `org.springframework.*`).

### Log Deduplication & Normalization (`src/core/dedup.rs`)
* Normalize volatile tokens (ISO timestamps, UUIDs, hex memory addresses `0x...`, request IDs, numeric durations) into placeholders (`<TS>`, `<UUID>`, `<HEX>`) so 2,000 identical error lines collapse into:
  `[x2,000 | first: 02:14:01 | last: 02:14:58] ERROR Connection refused to <UUID>`

### Exposed MCP & CLI Tools
1. **`slice_log` (`logcut slice [FILE]` or piped `stdin`)**:
   * **Args**: `path: Option<String>`, `raw_text: Option<String>`, `workspace_root: Option<String>` (default `"."`), `tail_lines: Option<usize>` (default `Some(5000)`), `context_lines: Option<usize>` (default `Some(3)`), `include_vendor_frames: Option<bool>` (default `Some(false)`), `max_chars: Option<usize>` (default `Some(12000)`).
   * **Behavior**: Scrubs secrets, deduplicates repeated log bursts, extracts stacktraces, collapses vendor frames, and resolves surviving `AppFrame` file paths against `workspace_root` to inline the exact crashing lines (`context_lines` above/below). Includes `SliceStats` (`raw_bytes`, `sliced_bytes`, `estimated_tokens_saved`, `reduction_percent`).
2. **`slice_build` (`logcut build [FILE]` or piped `stdin`)**:
   * **Args**: `path: Option<String>`, `raw_text: Option<String>`, `workspace_root: Option<String>` (default `"."`), `tail_lines: Option<usize>` (default `Some(5000)`), `max_errors: Option<usize>` (default `Some(10)`), `context_lines: Option<usize>` (default `Some(3)`), `max_chars: Option<usize>` (default `Some(12000)`).
   * **Behavior**: Reads a build log from a bounded tail, scrubs secrets, parses `rustc`/`cargo`, `tsc`, `go build`, and `gcc`/`clang` diagnostics, merges repeated cascades into root causes with occurrence counts, and resolves `path:line` spans against `workspace_root` for source snippets. Returns `SliceBuildReport` with `errors: Vec<CompilerError>`, `snippets`, `cascading_suppressed`, and `SliceStats`.
3. **`slice_container` (`logcut docker <CONTAINER>`)**:
   * **Args**: `container: String`, `tail_lines: Option<usize>` (default `Some(2000)`), `since: Option<String>`, `workspace_root: Option<String>`, `max_chars: Option<usize>`.
   * **Behavior**: Executes `docker logs --tail <N> <container>` safely via `tokio::process::Command` (with argument array—no shell interpolation) and pipes stdout+stderr directly through `slice_log`.
4. **`slice_service` (`logcut journal <UNIT>`)**:
   * **Args**: `unit: String`, `lines: Option<usize>` (default `Some(1000)`), `workspace_root: Option<String>`, `max_chars: Option<usize>`.
   * **Behavior**: Executes `journalctl -u <unit> -n <lines> --no-pager` safely and pipes output through `slice_log`.
5. **`init` (`logcut init --opencode / --claude / --cursor / --all`)**:
   * Generates MCP config entries and `/debug-log` slash commands for OpenCode, Claude Code, and Cursor.

---

## 6. Delivery Phases

1. **Phase 1 — Scaffold, Secret Redactor & Fixtures** (delivered in v0.1.0):
   * Initialize `Cargo.toml`, module structure, `redactor.rs` (with tests verifying API keys/URLs are scrubbed), and multi-language crash fixtures in `tests/fixtures/`.
2. **Phase 2 — Stacktrace Parser, Vendor Frame Filter & Source Mapping** (delivered in v0.1.0):
   * Implement `stacktrace.rs` (Rust, Python, JS/TS, Go, Java), `dedup.rs`, and `source_ctx.rs`.
   * Write integration tests in `tests/core_tests.rs` asserting vendor frame collapsing, error burst grouping (`[xN]`), and `file:line` source snippet resolution.
3. **Phase 3 — Container/Journal Sources, MCP Server & CLI** (delivered in v0.1.0):
   * Implement `sources.rs`, `src/mcp.rs` (`rmcp` stdio server), `src/agent.rs`, and `src/main.rs` (`slice`, `docker`, `journal`, `ask`, `mcp`, `init`, plus `--stats`).
   * Verify `cargo build`, `cargo build --no-default-features --features mcp`, and a child-process JSON-RPC `initialize` + `tools/list` stdio handshake test.
4. **Phase 4 — Docs & Release Prep** (delivered in v0.1.0):
   * Write `README.md` showing how `logcut` + `treecut` work together in OpenCode/Claude Code/Cursor, add `LICENSE-MIT` and `LICENSE-APACHE`, and verify `cargo publish --dry-run`.
5. **Phase 5 — Compiler Error Slicing, CI & Release Automation** (delivered in v0.2.0):
   * Add `src/core/compiler.rs` with `rustc`/`tsc`/`go`/`clang` parsers and cascade deduplication, fixtures, and integration tests.
   * Expose `slice_build` through MCP, the agent loop, and `logcut build [FILE]`; add `.github/workflows/ci.yml` for cross-OS checks and tagged releases.
