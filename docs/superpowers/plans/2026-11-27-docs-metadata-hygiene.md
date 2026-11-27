# Documentation, Metadata & Repo Hygiene Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Bring the repository's docs, package metadata, and hygiene config in line with the shipped v0.2.0 and prevent the known line-ending and stale-doc regressions.

**Architecture:** Repo-local edits only: Markdown docs, `Cargo.toml`, git/GitHub config. No Rust behavior changes, no version bump, no publish, no tag, no push. Each task is verified with grep, YAML parsing, `cargo metadata`, or the existing test suite.

**Tech Stack:** Markdown, Cargo manifest, Git attributes, GitHub Actions/YAML, existing Rust test suite.

**Spec:** `SPEC.md` (original project authority) plus the shipped `README.md`, `Cargo.toml`, `Cargo.lock`, and the observed crates.io/GitHub state.

## Global Constraints

- Version remains `0.2.0`; do not edit `Cargo.lock` or publish a new crate version.
- Leave `src/` and `tests/` unchanged; all permanent changes are docs, manifest metadata, and repo config.
- Every CLI flag written in docs must match the shipped commands: `slice`, `build`, `docker`, `journal`, `ask`, `init`, `mcp`, plus global `--stats`.
- crates.io metadata for the already-published 0.2.0 is immutable; manifest changes apply only to a future release.
- `tests/fixtures/*` must be checked out with LF endings on every OS.

## Review Focus

- Stale v0.1.0 spec claims (`logcut tail`, `version = "0.1.0"`, missing `slice_build`) mislead implementers — Task 1 grep guard.
- Published crate has no `repository`/`homepage` — Task 2 `cargo metadata` guard.
- README links/badges could point at guessed or placeholder URLs — Task 3 local-path check and use only URLs already seen in this repo.
- CRLF fixture checkout previously broke a stats test on Windows — Task 5 `.gitattributes` plus simulated CRLF test.
- New YAML config could be malformed — Task 5 parses every YAML file.

---

### Task 1: Refresh `SPEC.md` for v0.2.0

**Files:**
- Modify: `SPEC.md`

**Interfaces:**
- Consumes: shipped `README.md`, `Cargo.toml`, `src/main.rs` command enum.
- Produces: accurate spec text; no code interface.

- [x] **Step 1: Rewrite stale spec sections**

Update: mission line (remove `logcut tail`, add `build`); module tree (add `src/core/compiler.rs`, `.github/workflows/ci.yml`, compiler fixtures); Cargo.toml section (version `0.2.0`, add `repository`, `homepage`, `documentation`); tool contracts (add `slice_build` / `logcut build [FILE]` with `path`, `raw_text`, `workspace_root`, `tail_lines`, `max_errors`, `context_lines`, `max_chars`); phase list (mark v0.1.0 phases delivered, add v0.2.0 phase). Preserve the five hard invariants verbatim.

- [x] **Step 2: Verify no stale claims remain**

Run: `grep -nE '0\.1\.0|logcut tail|version = "0\.1' SPEC.md; grep -nE 'slice_build|logcut build|compiler\.rs|0\.2\.0' SPEC.md | head`
Expected: first grep finds only historical release references; second finds the new sections.

- [x] **Step 3: Commit**

`git add SPEC.md docs/superpowers/plans/2026-11-27-docs-metadata-hygiene.md && git commit -m "Refresh SPEC.md for v0.2.0"`

### Task 2: Complete Cargo package metadata

**Files:**
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: repository URL already used by README and release links (`https://github.com/bigwalkdoe/logcut`).
- Produces: `repository`, `homepage`, `readme` values readable via `cargo metadata`.

- [x] **Step 1: Add missing `[package]` fields**

Add `repository`, `homepage`, and `readme = "README.md"` alongside existing `documentation`. Do not change `version` or dependencies.

- [x] **Step 2: Verify metadata resolves**

Run: `cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; p=json.load(sys.stdin)["packages"][0]; print(p["repository"], p["homepage"], p["readme"])'`
Expected: the GitHub URL twice and `README.md`.

- [x] **Step 3: Commit**

`git add Cargo.toml && git commit -m "Add repository metadata to Cargo manifest"`

### Task 3: Polish README for launch

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: crates.io version `0.2.0`, GitHub release URL, docs.rs URL, license files, CI workflow file.
- Produces: accurate reader-facing overview; no code interface.

- [x] **Step 1: Add badges and missing links**

Add a compact badge row for crates.io version, docs.rs, CI status, and dual license using URLs already present in the repo or standard badge endpoints for those exact targets. Add release-artifact install instructions and links to `LICENSE-MIT` / `LICENSE-APACHE`. Keep all existing v0.2.0 tool claims unchanged.

- [x] **Step 2: Verify links point to real local files or known URLs**

Run: `python3 -c 'import re,pathlib; t=pathlib.Path("README.md").read_text(); print([p for p in re.findall(r"\]\(([^)#]+)", t) if not p.startswith("http") and not pathlib.Path(p).exists()])'`
Expected: `[]`.

- [x] **Step 3: Commit**

`git add README.md && git commit -m "Polish README launch links and badges"`

### Task 4: Add changelog and contribution guide

**Files:**
- Create: `CHANGELOG.md`, `CONTRIBUTING.md`

**Interfaces:**
- Consumes: commit history `c102b3a` (v0.1.0) and `4ecfa24` (v0.2.0), `SPEC.md` invariants, feature flags from `Cargo.toml`.
- Produces: human-facing release history and contributor workflow.

- [x] **Step 1: Write `CHANGELOG.md`**

Use Keep-a-Changelog shape with `## [0.2.0]` and `## [0.1.0]` sections. 0.2.0 lists compiler cascade slicing, new MCP/CLI/agent tool, CI/release workflow, docs; 0.1.0 lists the core/MCP/CLI first release. Do not invent unreleased items.

- [x] **Step 2: Write `CONTRIBUTING.md`**

Cover: setup (`cargo build`, `cargo test --locked`), verification commands (`cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, MCP-only check, `cargo publish --dry-run`), the five invariants from `SPEC.md`, fixture conventions, and the rule that releases/publishes need explicit approval.

- [x] **Step 3: Verify content**

Run: `grep -nE '^## \[0\.[12]\.0\]' CHANGELOG.md; grep -nE 'cargo test|SPEC.md|explicit approval' CONTRIBUTING.md`
Expected: both version headings and all three contributor lines present.

- [x] **Step 4: Commit**

`git add CHANGELOG.md CONTRIBUTING.md && git commit -m "Add changelog and contributor guide"`

### Task 5: Git and GitHub hygiene config

**Files:**
- Create: `.gitattributes`, `.github/PULL_REQUEST_TEMPLATE.md`, `.github/ISSUE_TEMPLATE/bug_report.yml`, `.github/ISSUE_TEMPLATE/feature_request.yml`, `.github/dependabot.yml`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: known CRLF failure from `tests/fixtures/noisy_json.log`, existing CI workflow.
- Produces: deterministic line endings and contribution automation config.

- [x] **Step 1: Force LF for fixtures and text files**

Create `.gitattributes` with `* text=auto eol=lf` and `tests/fixtures/* text eol=lf`. Add `.DS_Store`, `*.swp`, and `*.tmp` to `.gitignore` without removing existing entries.

- [x] **Step 2: Add GitHub templates and Dependabot config**

PR template: summary, verification checklist, invariants reminder. Issue templates: bug report and feature request in GitHub issue-form YAML. Dependabot: weekly `cargo` and `github-actions` ecosystems.

- [x] **Step 3: Verify YAML and line-ending guard**

Run: `python3 -c 'import yaml,glob; [yaml.safe_load(open(p)) for p in glob.glob(".github/**/*.yml", recursive=True)]; print("yaml ok")'`
Run: `python3 -c "p='tests/fixtures/noisy_json.log'; d=open(p,'rb').read(); open(p,'wb').write(d.replace(b'\\n', b'\\r\\n'))" && cargo test --test core_tests slice_log_redacts_secrets_and_groups_error_bursts; git checkout -- tests/fixtures/noisy_json.log`
Expected: `yaml ok` and the CRLF-simulated test passes.

- [x] **Step 4: Commit**

`git add .gitattributes .gitignore .github && git commit -m "Add git attributes, issue templates, and Dependabot config"`

### Task 6: Final verification

**Files:** none

- [x] **Step 1: Run the full local gate**

Run: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --locked`, `cargo check --locked --no-default-features --features mcp`, `cargo publish --dry-run --allow-dirty`.
Expected: all succeed; no changes to `src/` or `tests/`.

- [x] **Step 2: Confirm repo state**

Run: `git status --short && git log --oneline -6`
Expected: clean tree with one commit per task (plus any documented fixups); no tag, push, or publish performed.
