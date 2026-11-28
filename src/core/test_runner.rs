//! Test-output slicing for cargo, pytest, jest/vitest and go test.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::{Result, bail};
use regex::Regex;

use crate::core::models::{
    FrameKind, SliceStats, SliceTestArgs, SliceTestReport, StackFrame, StackLanguage, TestFailure,
};
use crate::core::source_ctx::{render_snippet, resolve_frame};
use crate::core::{redactor, sources};

static ANSI_ESCAPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(?:\x07|\x1b\\)").expect("ANSI escape regex")
});
static CARGO_PASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^test\s+.+?\s+\.\.\.\s+ok\s*$").expect("cargo pass regex"));
static CARGO_RESULT_PASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^test\s+result:\s+ok\.").expect("cargo summary regex"));
static GO_OK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^ok[ \t]{2,}\S+").expect("go ok regex"));
static DOT_PROGRESS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\.+(?:\s+\[\d+%])?$").expect("dot progress regex"));

const FAILURE_MARKERS: [&str; 8] = [
    "FAILED",
    "FAIL",
    "--- FAIL:",
    "test result: FAILED",
    "AssertionError",
    "assertion failed",
    "panicked at",
    "Error:",
];

static CARGO_FAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^test\s+(.+?)\s+\.\.\.\s+FAILED\s*$").expect("cargo failure regex")
});
static GO_FAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)---\s+FAIL:\s*(.+?)(?:\s+\([0-9.]+s\))?\s*$").expect("go failure regex")
});
static PYTEST_FAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bFAILED\s+(\S+)").expect("pytest failure regex"));
static GO_SUMMARY_FAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*FAIL\s+(\S+)").expect("go summary failure regex"));
static JEST_FAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)✗\s+(.+?)(?:\s+\([^)]*\))?\s*$").expect("jest failure regex")
});

static FRAME_LOCATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?P<path>[\w./\\-]+\.(?:rs|py|jsx|js|tsx|ts|go|java)):(?P<line>\d+)(?::(?P<col>\d+))?",
    )
    .expect("test frame location regex")
});

fn strip_ansi(text: &str) -> String {
    ANSI_ESCAPE.replace_all(text, "").into_owned()
}

/// Whether a test-runner line only reports success and can be dropped.
pub fn is_passing_line(line: &str) -> bool {
    let clean = strip_ansi(line);
    let trimmed = clean.trim();
    if trimmed.is_empty() {
        return false;
    }

    // cargo: `test <name> ... ok` and `test result: ok.`
    if CARGO_PASS.is_match(trimmed) || CARGO_RESULT_PASS.is_match(trimmed) {
        return true;
    }

    // go test: `--- PASS:`, `ok  <pkg>` (also with a tab), and bare `PASS`.
    if trimmed.starts_with("--- PASS:") || trimmed == "PASS" || GO_OK.is_match(trimmed) {
        return true;
    }

    // jest/vitest: checkmarks and `PASS <suite>`.
    if trimmed.starts_with('✓') || trimmed.starts_with('✔') {
        return true;
    }
    if trimmed.starts_with("PASS ") && !trimmed.contains("FAIL") {
        return true;
    }

    // pytest: `PASSED` tokens and dot-only progress lines.
    if DOT_PROGRESS.is_match(trimmed) {
        return true;
    }
    if !trimmed.contains("FAIL") && trimmed.split_whitespace().any(|word| word == "PASSED") {
        return true;
    }

    false
}

fn extract_name(block: &str) -> Option<String> {
    const PATTERNS: [&LazyLock<Regex>; 5] = [
        &CARGO_FAIL,
        &GO_FAIL,
        &PYTEST_FAIL,
        &GO_SUMMARY_FAIL,
        &JEST_FAIL,
    ];
    PATTERNS.iter().find_map(|regex| {
        regex
            .captures(block)
            .and_then(|captures| captures.get(1).map(|name| name.as_str().trim().to_owned()))
    })
}

fn language_for_path(path: &str) -> StackLanguage {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".rs") {
        StackLanguage::Rust
    } else if lower.ends_with(".py") {
        StackLanguage::Python
    } else if lower.ends_with(".js") || lower.ends_with(".jsx") {
        StackLanguage::JavaScript
    } else if lower.ends_with(".ts") || lower.ends_with(".tsx") {
        StackLanguage::TypeScript
    } else if lower.ends_with(".go") {
        StackLanguage::Go
    } else {
        StackLanguage::Java
    }
}

fn extract_frame(block: &str) -> Option<(String, StackFrame)> {
    let captures = FRAME_LOCATION.captures(block)?;
    let location = captures.get(0)?.as_str().to_owned();
    let path = captures.name("path")?.as_str().to_owned();
    let line = captures.name("line")?.as_str().parse().ok()?;
    let column = captures
        .name("col")
        .and_then(|value| value.as_str().parse().ok());
    let raw = block
        .lines()
        .find(|line| FRAME_LOCATION.is_match(line))
        .unwrap_or(&location)
        .to_owned();

    Some((
        location,
        StackFrame {
            language: language_for_path(&path),
            kind: FrameKind::App,
            path,
            line: Some(line),
            column,
            function: String::new(),
            raw,
        },
    ))
}

/// Split filtered test output into failure blocks and resolve their locations.
pub fn parse_failures(text: &str, root: &Path, context_lines: usize) -> Vec<TestFailure> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in text.split('\n') {
        // A named failure header starts a new block; blank lines and
        // continuation lines (including assertion diffs) stay with it.
        if starts_named_failure(line) && !current.is_empty() {
            blocks.push(current.join("\n"));
            current.clear();
        }
        if current.is_empty() && !starts_failure(line) {
            continue;
        }
        current.push(line);
    }
    if !current.is_empty() {
        blocks.push(current.join("\n"));
    }

    blocks
        .into_iter()
        .filter(|block| has_failure_marker(block))
        .map(|block| {
            let clean = strip_ansi(&block);
            let name = extract_name(&clean);
            let (location, snippet) = match extract_frame(&clean) {
                Some((location, frame)) => {
                    let snippet = resolve_frame(root, &frame, context_lines);
                    (Some(location), snippet)
                }
                None => (None, None),
            };

            TestFailure {
                name,
                location,
                message: block,
                snippet,
            }
        })
        .collect()
}

fn starts_named_failure(line: &str) -> bool {
    let clean = strip_ansi(line);
    clean.starts_with("--- FAIL:")
        || clean.starts_with("test result: FAILED")
        || CARGO_FAIL.is_match(&clean)
        || clean.starts_with("FAILED ")
        || clean.starts_with("FAIL ")
        || clean.starts_with("✗ ")
        || clean.starts_with("✕ ")
}

fn starts_failure(line: &str) -> bool {
    let clean = strip_ansi(line);
    starts_named_failure(&clean) || FAILURE_MARKERS.iter().any(|marker| clean.contains(marker))
}

fn has_failure_marker(block: &str) -> bool {
    let clean = strip_ansi(block);
    FAILURE_MARKERS.iter().any(|marker| clean.contains(marker))
}

fn render(failures: &[TestFailure], passing_lines_stripped: usize) -> String {
    let mut sections = Vec::new();

    let blocks: Vec<String> = failures
        .iter()
        .enumerate()
        .map(|(index, failure)| {
            format!(
                "--- failure {} ---\nname: {}\nlocation: {}\nmessage:\n{}",
                index + 1,
                failure.name.as_deref().unwrap_or("(unknown)"),
                failure.location.as_deref().unwrap_or("(unknown)"),
                failure.message
            )
        })
        .collect();
    let failure_body = if blocks.is_empty() {
        "(none)".to_owned()
    } else {
        blocks.join("\n\n")
    };
    sections.push(format!("== Test failures ==\n{failure_body}"));

    let mut seen = HashSet::new();
    let snippets: Vec<String> = failures
        .iter()
        .filter_map(|failure| failure.snippet.as_ref())
        .filter(|snippet| {
            seen.insert((
                snippet.path.clone(),
                snippet.start_line,
                snippet.end_line,
                snippet.target_line,
            ))
        })
        .map(render_snippet)
        .collect();
    if !snippets.is_empty() {
        sections.push(format!(
            "== Source context ==\n{}",
            snippets.join("\n").trim_end()
        ));
    }

    if failures.is_empty() {
        sections.push("== Test summary ==\nno failures detected".to_owned());
    }
    sections.push(format!(
        "[passing lines stripped: {passing_lines_stripped} | failures: {}]",
        failures.len()
    ));
    sections.join("\n\n")
}

/// Slice test output down to its failures with optional source context.
pub fn slice(request: SliceTestArgs) -> Result<SliceTestReport> {
    let tail_lines = request.tail_lines.unwrap_or(5000);
    let raw = match (&request.path, &request.raw_text) {
        (Some(path), _) => sources::read_log_tail(Path::new(path), tail_lines)?,
        (None, Some(text)) => sources::tail_text(text, tail_lines),
        _ => bail!("provide path or raw_text to slice tests"),
    };

    let redacted = redactor::redact(&raw);
    let mut passing_lines_stripped = 0;
    let kept: Vec<&str> = redacted
        .lines()
        .filter(|line| {
            if is_passing_line(line) {
                passing_lines_stripped += 1;
                false
            } else {
                true
            }
        })
        .collect();

    let root = Path::new(request.workspace_root.as_deref().unwrap_or("."));
    let context_lines = request.context_lines.unwrap_or(3);
    let failures = parse_failures(&kept.join("\n"), root, context_lines);

    // Snippets are read from disk, so redact the finished report once more.
    let mut text = redactor::redact(&render(&failures, passing_lines_stripped));
    let max_chars = request.max_chars.unwrap_or(12_000);
    if text.len() > max_chars {
        let marker = "\n… [truncated]";
        let budget = max_chars.saturating_sub(marker.len()).min(text.len());
        let mut cut = budget;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text = format!("{}{marker}", &text[..cut]);
    }

    let raw_bytes = raw.len();
    let sliced_bytes = text.len();
    let saved = raw_bytes.saturating_sub(sliced_bytes);
    Ok(SliceTestReport {
        text,
        stats: SliceStats {
            raw_bytes,
            sliced_bytes,
            estimated_tokens_saved: saved / 4,
            reduction_percent: if raw_bytes == 0 {
                0.0
            } else {
                saved as f64 / raw_bytes as f64 * 100.0
            },
        },
        failures,
        passing_lines_stripped,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{is_passing_line, slice};
    use crate::core::models::SliceTestArgs;

    fn args(raw: &str) -> SliceTestArgs {
        SliceTestArgs {
            path: None,
            raw_text: Some(raw.to_owned()),
            workspace_root: Some(".".to_owned()),
            tail_lines: Some(5000),
            context_lines: Some(3),
            max_chars: Some(12_000),
        }
    }

    #[test]
    fn cargo_passing_lines_are_stripped() {
        let report = slice(args(
            "test ok_case ... ok\ntest bad_case ... FAILED\nthread 'bad_case' panicked at src/lib.rs:3:5:\nassertion `left == right` failed\n  left: 1\n right: 2\n",
        ))
        .unwrap();

        assert_eq!(report.passing_lines_stripped, 1);
        assert!(!report.text.contains("test ok_case ... ok"));
        assert!(report.text.contains("test bad_case ... FAILED"));
        assert!(
            report
                .text
                .contains("[passing lines stripped: 1 | failures: 1]")
        );
    }

    #[test]
    fn pytest_passed_lines_are_stripped() {
        let report = slice(args(
            "test_ok PASSED\nFAILED tests/test_x.py::test_bad - AssertionError: bad\n",
        ))
        .unwrap();

        assert_eq!(report.passing_lines_stripped, 1);
        assert!(!report.text.lines().any(|line| line == "test_ok PASSED"));
        assert!(report.text.contains("tests/test_x.py::test_bad"));
        assert_eq!(
            report.failures[0].name.as_deref(),
            Some("tests/test_x.py::test_bad")
        );
    }

    #[test]
    fn jest_checkmarks_are_stripped() {
        let report = slice(args(
            "✓ adds\n✗ subtracts\nAssertionError: 2 !== 3\n  Expected: 2\n  Received: 3\n",
        ))
        .unwrap();

        assert_eq!(report.passing_lines_stripped, 1);
        assert!(!report.text.contains("✓ adds"));
        assert!(report.text.contains("✗ subtracts"));
        assert!(report.text.contains("  Expected: 2"));
        assert!(report.text.contains("  Received: 3"));
    }

    #[test]
    fn go_ok_lines_are_stripped() {
        let report = slice(args(
            "ok  \tpkg.test\t0.01s\n--- FAIL: TestThing (0.00s)\n    thing_test.go:12: got 1, want 2\n",
        ))
        .unwrap();

        assert_eq!(report.passing_lines_stripped, 1);
        assert!(!report.text.contains("ok  \tpkg.test"));
        assert!(report.text.contains("--- FAIL: TestThing"));
        assert_eq!(report.failures[0].name.as_deref(), Some("TestThing"));
    }

    #[test]
    fn assertion_diff_lines_are_preserved() {
        let report = slice(args(
            "thread 'x' panicked at src/lib.rs:10:5:\nassertion `left == right` failed\n  left: 1\n right: 2\n",
        ))
        .unwrap();

        assert!(report.text.contains("== Test failures =="));
        assert!(report.text.contains("  left: 1"));
        assert!(report.text.contains(" right: 2"));
        assert_eq!(report.failures.len(), 1);
    }

    #[test]
    fn assertion_diffs_survive_blank_lines() {
        let report = slice(args(
            "test bad_case ... FAILED\n\nthread 'bad_case' panicked at src/lib.rs:3:5:\nassertion `left == right` failed\n\n  left: 1\n right: 2\n",
        ))
        .unwrap();

        assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
        assert!(report.text.contains("  left: 1"), "{}", report.text);
        assert!(report.text.contains(" right: 2"), "{}", report.text);
    }

    #[test]
    fn file_line_resolves_to_tempdir_snippet() {
        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join("src");
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(
            source_dir.join("lib.rs"),
            "fn one() {}\nfn two() { panic!() }\nfn three() {}\n",
        )
        .unwrap();

        let mut request =
            args("thread 'bad' panicked at src/lib.rs:2:5:\nassertion failed: boom\n");
        request.workspace_root = Some(dir.path().to_string_lossy().into_owned());
        request.context_lines = Some(1);
        let report = slice(request).unwrap();

        assert_eq!(report.failures.len(), 1);
        let snippet = report.failures[0].snippet.as_ref().expect("source snippet");
        assert_eq!(snippet.target_line, 2);
        assert_eq!(snippet.path, "src/lib.rs");
        assert!(report.text.contains("== Source context =="));
        assert!(report.text.contains("> 2 | fn two()"));
    }

    #[test]
    fn ansi_passing_lines_are_recognized() {
        assert!(is_passing_line("\x1b[32mtest quick ... ok\x1b[0m"));
        assert!(is_passing_line("test result: ok. 3 passed; 0 failed"));
        assert!(is_passing_line(".......... [100%]"));
        assert!(!is_passing_line("test slow ... FAILED"));
        assert!(!is_passing_line("--- FAIL: TestBroken (0.00s)"));
    }
}
