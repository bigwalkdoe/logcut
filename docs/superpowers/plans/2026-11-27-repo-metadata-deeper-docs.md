# Repo Metadata & Deeper Documentation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fill in the GitHub About section, refresh the crate description, and add user-facing guides that explain how to use, configure, and extend `logcut`.

**Architecture:** GitHub-side settings are applied with `gh repo edit`; repository documentation lives as Markdown under `docs/` and is linked from `README.md`. The Rust source and shipped behavior stay untouched; every document is checked against the actual flags, defaults, and code paths it describes.

**Tech Stack:** GitHub CLI, Markdown, Cargo manifest metadata, existing Rust test/gate commands.

**Spec:** `SPEC.md` (vision and five hard invariants) plus the request "Add repo metadata, about section and deeper documentation".

## Global Constraints

- No changes to `src/` or `tests/`; no shipped behavior changes.
- Do not bump the version, tag, publish, push, or merge without explicit approval.
- Preserve the five hard invariants in `SPEC.md` section 2 verbatim in any copied text.
- Published crates.io 0.2.0 metadata is immutable; manifest description changes apply to the next release.
- Every documented flag, default, tool name, environment variable, and path must be verified against the current source or a local file.

## Review Focus

- `slice_log`/`slice_build` `max_chars` truncates rendered text only; structured `errors`/`frames`/`groups` are not truncated. Guides must say so.
- `slice_build` tails a file path but does not tail `raw_text`; usage guide must scope the 5,000-line tail claim to file input.
- `tail_lines = 0` yields an empty slice; troubleshooting must mention it.
- `workspace_root` controls app-frame classification and snippet lookup; missing files silently produce no snippets.
- MCP stdout carries JSON-RPC only, CLI `--stats` goes to stderr; a reader redirecting stdout must not get diagnostics mixed in.

---

### Task 1: Fill in the GitHub About section

**Files:** none (GitHub repository settings)

**Interfaces:**
- Consumes: repository URL `https://github.com/bigwalkdoe/logcut`, docs.rs URL, crate purpose from `Cargo.toml`.
- Produces: public description, website URL, and topic list visible in the GitHub sidebar.

- [ ] **Step 1: Set description, website, and topics**

Run:

```bash
gh repo edit bigwalkdoe/logcut \
  --description "Local-first stacktrace, runtime log & compiler error slicing for AI coding agents and terminal pipelines" \
  --homepage "https://docs.rs/logcut" \
  --add-topic mcp,rust,cli,debugging,logging,stacktrace,error-handling,developer-tools,ai-agents,local-first,compiler-errors,observability
```

- [ ] **Step 2: Verify GitHub settings**

Run: `gh repo view bigwalkdoe/logcut --json description,homepageUrl,repositoryTopics --jq '{description,homepageUrl,topics:[.repositoryTopics[].name]}'`
Expected: description matches exactly, `homepageUrl` is `https://docs.rs/logcut`, and all twelve topics appear.

### Task 2: Refresh crate description and add a docs index

**Files:**
- Modify: `Cargo.toml` `[package].description`
- Create: `docs/README.md`
- Modify: `README.md`

**Interfaces:**
- Consumes: tool list from `src/mcp.rs`, guide filenames from Tasks 3–5.
- Produces: `docs/README.md` with links to `usage.md`, `mcp.md`, `architecture.md`, `troubleshooting.md`; README "Documentation" section pointing at that index.

- [ ] **Step 1: Update the manifest description**

Change `description` in `Cargo.toml` to `Local-first stacktrace, runtime log & compiler error slicing for AI coding agents and terminal pipelines`. Do not change version, dependencies, or other fields.

- [ ] **Step 2: Write `docs/README.md`**

Include a one-paragraph orientation, the four guide links with one-line descriptions, and a note that `SPEC.md` is the vision/invariants document and `CHANGELOG.md` is the release record.

- [ ] **Step 3: Link the index from `README.md`**

Add a short `## Documentation` section before `## Development` with links to `docs/README.md` and the four guides.

- [ ] **Step 4: Verify metadata and links**

Run: `cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; assert "compiler error" in json.load(sys.stdin)["packages"][0]["description"]; print("DESCRIPTION_OK")'`
Run: `python3 -c 'import re,pathlib; bad=[p for f in ["README.md","docs/README.md"] for p in re.findall(r"\]\(([^)#]+)", pathlib.Path(f).read_text()) if not p.startswith("http") and not (pathlib.Path(f).parent / p).exists()]; print(bad); assert not bad'`
Expected: `DESCRIPTION_OK` and `[]`.

- [ ] **Step 5: Commit**

`git add Cargo.toml README.md docs/README.md && git commit -m "Refresh crate description and add documentation index"`

### Task 3: Write the usage guide

**Files:**
- Create: `docs/usage.md`

**Interfaces:**
- Consumes: CLI flags in `src/main.rs:50-123`, defaults in `src/core/models.rs:80-100`, `stats` behavior in `src/main.rs:125-139`, redaction patterns in `src/core/redactor.rs`.
- Produces: reader-facing CLI guide; no code interface.

- [ ] **Step 1: Write `docs/usage.md`**

Cover, in order: install and feature flags; source selection (`FILE`, `--raw-text`, piped stdin, `docker`, `journal`); every flag for `slice`, `build`, `docker`, `journal`, `ask`, and `init` with the exact defaults (5000 tail lines, 2000 container lines, 1000 journal lines, 3 context lines, 10 max errors, 12000 max chars); four worked examples; what `--stats` emits and where; what redaction masks; bounded-tail behavior; `max_chars` truncates rendered text only; `slice_build` tails files but not `raw_text`; and the treecut workflow.

- [ ] **Step 2: Verify flags and defaults against source**

Run: `for flag in raw-text workspace-root tail-lines context-lines include-vendor-frames max-chars max-errors lines since stats opencode claude cursor all; do grep -q -- "--$flag" src/main.rs || echo "MISSING $flag"; done`
Run: `grep -nE 'Some\(5000\)|Some\(2000\)|Some\(1000\)|Some\(3\)|Some\(10\)|Some\(12_000\)' src/core/models.rs`
Expected: no `MISSING` lines and all six default values present.

- [ ] **Step 3: Commit**

`git add docs/usage.md && git commit -m "Add CLI usage guide"`

### Task 4: Write the MCP and agent reference

**Files:**
- Create: `docs/mcp.md`

**Interfaces:**
- Consumes: tool names and descriptions in `src/mcp.rs:38-93`, parameter structs in `src/core/models.rs`, output structs in `src/core/models.rs`, `logcut init` output in `src/main.rs:150-189`, agent environment variables in `src/agent.rs:122-184`.
- Produces: MCP/agent reference; no code interface.

- [ ] **Step 1: Write `docs/mcp.md`**

Cover: starting `logcut mcp`; stdio JSON-RPC purity (stdout reserved for protocol, diagnostics to stderr); the four tools with input fields, defaults, and result fields; source-context and redaction behavior; `logcut init` generated files for OpenCode/Claude/Cursor and the "keep existing files" rule; a `tools/list` example; and `logcut ask` environment variables (`OPENAI_API_KEY` required, `OPENAI_MODEL` defaults to `gpt-4o-mini`, `OPENAI_BASE_URL` defaults to `https://api.openai.com/v1`, 8 tool-round limit).

- [ ] **Step 2: Verify tool names, schemas, and host config paths**

Run: `for tool in slice_log slice_build slice_container slice_service; do grep -q -- "name = \"$tool\"" src/mcp.rs || echo "MISSING $tool"; done`
Run: `for path in opencode.json .opencode/commands/debug-log.md .mcp.json .claude/commands/debug-log.md .cursor/mcp.json .cursor/rules/debug-log.mdc; do grep -Fq "$path" src/main.rs || echo "MISSING $path"; done`
Run: `PATH=/tmp/opencode/rust-tools/toolchain/bin:$PATH cargo test --locked --test core_tests tool_requests_share_json_schema_with_mcp`
Expected: no `MISSING` lines and the schema test passes.

- [ ] **Step 3: Commit**

`git add docs/mcp.md && git commit -m "Add MCP and agent reference"`

### Task 5: Write the architecture guide

**Files:**
- Create: `docs/architecture.md`

**Interfaces:**
- Consumes: module map from `SPEC.md` section 3 and `src/core/mod.rs`, invariants from `SPEC.md` section 2, feature flags from `Cargo.toml`.
- Produces: contributor-facing architecture guide; no code interface.

- [ ] **Step 1: Write `docs/architecture.md`**

Cover: the CLI/MCP/agent transports over one core; module responsibilities for `models`, `redactor`, `dedup`, `stacktrace`, `compiler`, `source_ctx`, `sources`; the `slice_log` and `slice_build` data flow; all five invariants copied verbatim from `SPEC.md`; feature flags (`cli` gates `ask`/`reqwest`, `mcp` gates the stdio server, other commands always available); where to add a new parser, source, or tool; and the verification commands from `CONTRIBUTING.md`.

- [ ] **Step 2: Verify invariants and module claims**

Run: `python3 -c "import pathlib; s=pathlib.Path('SPEC.md').read_text(); a=pathlib.Path('docs/architecture.md').read_text(); g=lambda t:t.split('## 2. Hard Architectural Invariants (Do Not Violate)\n')[1].split('\n---')[0] if '## 2. Hard Architectural Invariants (Do Not Violate)' in t else t.split('### Hard invariants\n')[1].split('\n## ')[0]; print('INVARIANTS_OK' if g(s).strip()==g(a).strip() else 'INVARIANTS_DIFFER')"`
Expected: `INVARIANTS_OK`.

- [ ] **Step 3: Commit**

`git add docs/architecture.md && git commit -m "Add architecture guide"`

### Task 6: Write the troubleshooting guide

**Files:**
- Create: `docs/troubleshooting.md`

**Interfaces:**
- Consumes: source resolution in `src/core/source_ctx.rs`, tail behavior in `src/core/sources.rs`, redaction patterns, stdio rules in `src/main.rs`, feature gates in `Cargo.toml`.
- Produces: symptom-first help page; no code interface.

- [ ] **Step 1: Write `docs/troubleshooting.md`**

Use symptom headings with a cause and fix for: no source snippets (wrong `workspace_root`, missing file, path normalization); empty output from `tail_lines 0`; truncated reports (`max_chars` and rendered-text-only truncation); redacted credentials and why that is intentional; CRLF/Windows log input; `docker` and `journalctl` binary or permission failures (argument-array execution, no shell); `logcut ask` missing `OPENAI_API_KEY`; `logcut mcp` producing no JSON-RPC because the build lacks `mcp`; and `--stats` appearing on stderr rather than stdout. End with how to run the local gates and where to report a bug.

- [ ] **Step 2: Verify references**

Run: `for token in workspace-root tail-lines max-chars OPENAI_API_KEY journalctl docker stderr mcp; do grep -q -- "$token" docs/troubleshooting.md || echo "MISSING $token"; done`
Expected: no `MISSING` lines.

- [ ] **Step 3: Commit**

`git add docs/troubleshooting.md && git commit -m "Add troubleshooting guide"`

### Task 7: Final verification and review

**Files:** none

- [ ] **Step 1: Run the full local gate**

Run: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --locked`, `cargo check --locked --no-default-features --features mcp`, `cargo publish --dry-run --allow-dirty`.
Expected: all pass; `git diff --stat 1af54e7..HEAD` shows no `src/` or `tests/` changes.

- [ ] **Step 2: Verify docs links and About settings**

Run: `python3 -c 'import re,pathlib; bad=[(str(f),p) for f in pathlib.Path(".").rglob("*.md") if not {".git","target"}.intersection(f.parts) for p in re.findall(r"\]\(([^)#]+)", f.read_text()) if not p.startswith("http") and not (f.parent / p).exists()]; print(bad); assert not bad'`
Run: `gh repo view bigwalkdoe/logcut --json description,homepageUrl,repositoryTopics --jq '.description + " | " + .homepageUrl + " | " + ((.repositoryTopics // []) | length | tostring)'`
Expected: `[]` and a non-empty description, docs.rs homepage, topic count 12.

- [ ] **Step 3: Review and hand off**

Review the whole branch with a fresh reviewer subagent (or self-review if the reviewer is unavailable), fix Critical/Important findings, and report the branch state. Do not push, merge, tag, or publish without explicit approval.
