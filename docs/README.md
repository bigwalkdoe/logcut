# logcut documentation

`logcut` turns noisy runtime logs, stacktraces, and compiler error cascades into compact, redacted diagnostic reports. It runs both as a CLI for shell pipelines and as an MCP stdio server for AI coding hosts, with one shared Rust core behind both interfaces. These guides cover the day-to-day work: feeding the slicer, configuring hosts and agents, understanding the code, and recovering from bad input.

## Guides

- [Usage guide](usage.md) — every CLI command and flag, input source, default, worked example, and what the report contains.
- [MCP and agent reference](mcp.md) — starting the stdio server, the six MCP tools and four agent tools, host configuration via `logcut init`, and `logcut ask`.
- [Architecture guide](architecture.md) — module responsibilities, data flow, hard invariants, feature flags, and where to extend the code.
- [Troubleshooting](troubleshooting.md) — symptom-first fixes for empty reports, missing snippets, redaction, source readers, and feature gates.

For the vision and the five hard architectural invariants, see [SPEC.md](../SPEC.md). For release history, see [CHANGELOG.md](../CHANGELOG.md). Contributor setup and verification live in [CONTRIBUTING.md](../CONTRIBUTING.md).
