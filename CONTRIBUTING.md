# Contributing

## Setup

Clone the repository, then build and test locally:

```bash
cargo build
cargo test --locked
```

Rust stable with the `rustfmt` and `clippy` components is expected. No network service or external API is needed for the default test suite.

## Verification

Before proposing a change, run the same checks CI runs:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo check --no-default-features --features mcp
cargo publish --dry-run
```

The MCP-only check matters: the `mcp` feature must build without `cli`/`reqwest`. `--stats` output belongs on stderr so JSON-RPC on stdout stays clean.

## Architecture invariants

These five rules from [SPEC.md](SPEC.md) are hard constraints:

1. **Strict `stdout` Purity in MCP Mode**: Never use `println!` or write logs to `stdout` in `core/` or `mcp.rs`. Route `tracing_subscriber` exclusively to `std::io::stderr` with `.with_ansi(false)`.
2. **Zero UI/Network Coupling in `core/`**: `src/core/` must never import `clap`, `rmcp`, or `reqwest`. Every core function takes typed Rust structs and returns `anyhow::Result<T>` where `T: Serialize`.
3. **Single Source of Truth for Tool Schemas**: All MCP tool parameter structs in `src/core/models.rs` must derive `#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]`.
4. **Secret & PII Redaction by Default**: Automatically mask Bearer tokens, API keys (`sk-...`, `ghp_...`), AWS keys, and database connection URLs (`postgres://user:pass@...`) before returning sliced logs to any LLM or MCP client.
5. **Streaming / Bounded Memory Reads**: Never load multi-gigabyte log files blindly into memory; tail/window large files safely from the end (`BufReader` / reverse seek) when `tail_lines` is set.

## Fixtures and tests

- Sample inputs live in `tests/fixtures/` (panics, tracebacks, compiler output, noisy logs). Add a small deterministic fixture for each new parser or language, and assert behavior in `tests/core_tests.rs`.
- Keep fixture-dependent assertions line-ending agnostic; CI runs on Windows as well as Linux and macOS.
- Keep tests offline and deterministic: no live Docker, journalctl, or network calls in the default suite.

## Releases and publishing

Version bumps, tags, GitHub releases, `cargo publish`, pushes, and merges require explicit approval from the maintainer. Do not bump the version or publish as part of a normal change; add user-facing changes to [CHANGELOG.md](CHANGELOG.md) instead.
