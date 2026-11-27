## Summary

<!-- What changed and why? Link any related issue. -->

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --locked`
- [ ] `cargo check --no-default-features --features mcp` (when core or MCP behavior changes)
- [ ] Tests and fixtures updated where relevant

## Invariants

- [ ] MCP stdout stays JSON-RPC-only; diagnostics go to stderr
- [ ] `src/core/` has no UI or network coupling
- [ ] Tool parameter structs still derive `schemars::JsonSchema`
- [ ] Secrets stay redacted by default
- [ ] Large inputs remain bounded/tailed

<!-- Version bumps, tags, releases, publishes, pushes, and merges need explicit maintainer approval. -->
