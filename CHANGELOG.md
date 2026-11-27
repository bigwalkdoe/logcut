# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-11-27

### Added

- `slice_build`: compiler error cascade slicing for `rustc`/`cargo`, `tsc`, `go build`, and `gcc`/`clang`, exposed through MCP, the agent loop, and `logcut build [FILE]` (or piped stdin). Repeated cascades collapse into root causes with occurrence counts and `path:line` source snippets.
- `.github/workflows/ci.yml`: fmt, clippy, and tests on Linux, macOS, and Windows; an MCP-only build check; and tagged release archives with SHA-256 checksums.
- Integration fixtures for Rust, TypeScript, Go, and clang compiler errors.
- README updates for compiler slicing and bounded build-log tailing.

### Changed

- Broadened the product description from stacktrace/runtime log slicing to include compiler errors. Existing tool contracts and the five architectural invariants are unchanged.

### Fixed

- Made log statistics assertions CRLF-safe so the test suite passes on Windows.

## [0.1.0] - 2026-11-27

### Added

- First release: a pure-Rust core with secret/PII redaction, error-burst deduplication, multi-language stacktrace parsing (Rust, Python, JS/TS, Go, Java/JVM), vendor-frame collapsing, and crash-line source context.
- MCP stdio server (`logcut mcp`) with `slice_log`, `slice_container`, and `slice_service` tools, plus the agentic `logcut ask` loop.
- CLI commands `slice`, `docker`, `journal`, `ask`, `mcp`, and `init`; `--stats` writes size/token statistics to stderr so stdout stays pipe-safe.
- `logcut init` server configs and `/debug-log` slash commands for OpenCode, Claude Code, and Cursor.
- Dual licensing under MIT or Apache-2.0, at your option.
