//! Multi-language stacktrace parsing and vendor-frame collapsing.
//!
//! Frames are always classified accurately; the `include_vendor_frames` flag
//! controls whether rendering keeps them or substitutes a single summary line.

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::core::models::{FrameKind, StackFrame, StackLanguage};

static RUST_FRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:\d+:\s*)?at\s+(?P<path>\S+\.rs):(?P<line>\d+):(?P<col>\d+)\s*$")
        .expect("Rust frame regex")
});
static RUST_SYMBOL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\d+:\s+(?P<function>.+)$").expect("Rust symbol regex"));
static PYTHON_FRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"File\s+"(?P<path>[^"]+)",\s+line\s+(?P<line>\d+),\s+in\s+(?P<function>.+?)\s*$"#)
        .expect("Python frame regex")
});
static JS_FRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*at\s+(?:(?P<function>[^()]+?)\s+\()?(?P<path>[^\s()]+):(?P<line>\d+):(?P<col>\d+)\)?\s*$")
        .expect("JS frame regex")
});
static GO_FRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s+(?P<path>\S+\.go):(?P<line>\d+)(?:\s+\+0x[0-9a-f]+)?\s*$")
        .expect("Go frame regex")
});
static JAVA_FRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s+at\s+(?P<function>[^\s()]+)\((?P<path>[^:()]+\.java):(?P<line>\d+)\)\s*$")
        .expect("Java frame regex")
});

fn capture(regex: &Regex, line: &str, name: &str) -> Option<String> {
    regex
        .captures(line)?
        .name(name)
        .map(|value| value.as_str().to_owned())
}

fn classify(path: &str, function: &str, workspace_root: &Path) -> FrameKind {
    let normalized = path.replace('\\', "/");
    const VENDOR_PATHS: [&str; 9] = [
        "/rustc/",
        ".cargo/registry",
        "site-packages/",
        "/usr/lib/python",
        "node_modules/",
        "node:internal/",
        "/usr/local/go/",
        "/runtime/",
        "/net/http/",
    ];
    const VENDOR_SYMBOLS: [&str; 11] = [
        "std::",
        "core::",
        "alloc::",
        "tokio::",
        "runtime/",
        "runtime.",
        "net/http.",
        "java.",
        "javax.",
        "sun.",
        "org.springframework.",
    ];

    if VENDOR_PATHS
        .iter()
        .any(|marker| normalized.contains(marker))
        || VENDOR_SYMBOLS
            .iter()
            .any(|prefix| function.starts_with(prefix))
    {
        return FrameKind::Vendor;
    }

    if Path::new(path).is_absolute() {
        let root = workspace_root.to_string_lossy();
        let root = root.trim_end_matches('/');
        if path != root && !path.starts_with(&format!("{root}/")) {
            return FrameKind::Vendor;
        }
    }

    FrameKind::App
}

/// Parse every recognizable frame from a crash log, in source order.
///
/// `include_vendor_frames` is retained in the API so callers can pass their
/// preference through one contract; classification itself is unconditional.
pub fn parse_stacktrace(
    text: &str,
    workspace_root: &Path,
    _include_vendor_frames: bool,
) -> Vec<StackFrame> {
    let mut frames = Vec::new();
    let mut rust_symbol = String::new();
    let mut go_symbol = String::new();

    for line in text.lines() {
        if let (Some(path), Some(number), Some(column)) = (
            capture(&RUST_FRAME, line, "path"),
            capture(&RUST_FRAME, line, "line"),
            capture(&RUST_FRAME, line, "col"),
        ) {
            frames.push(StackFrame {
                language: StackLanguage::Rust,
                kind: classify(&path, &rust_symbol, workspace_root),
                path,
                line: number.parse().ok(),
                column: column.parse().ok(),
                function: rust_symbol.clone(),
                raw: line.trim().to_owned(),
            });
            rust_symbol.clear();
            continue;
        }

        if let Some(symbol) = capture(&RUST_SYMBOL, line, "function") {
            rust_symbol = symbol;
        }

        if let (Some(path), Some(number), Some(function)) = (
            capture(&PYTHON_FRAME, line, "path"),
            capture(&PYTHON_FRAME, line, "line"),
            capture(&PYTHON_FRAME, line, "function"),
        ) {
            frames.push(StackFrame {
                language: StackLanguage::Python,
                kind: classify(&path, &function, workspace_root),
                path,
                line: number.parse().ok(),
                column: None,
                function,
                raw: line.trim().to_owned(),
            });
            continue;
        }

        if let (Some(path), Some(number), Some(column)) = (
            capture(&JS_FRAME, line, "path"),
            capture(&JS_FRAME, line, "line"),
            capture(&JS_FRAME, line, "col"),
        ) {
            let function = capture(&JS_FRAME, line, "function").unwrap_or_default();
            frames.push(StackFrame {
                language: StackLanguage::JavaScript,
                kind: classify(&path, &function, workspace_root),
                path,
                line: number.parse().ok(),
                column: column.parse().ok(),
                function,
                raw: line.trim().to_owned(),
            });
            continue;
        }

        if let (Some(path), Some(number)) = (
            capture(&GO_FRAME, line, "path"),
            capture(&GO_FRAME, line, "line"),
        ) {
            frames.push(StackFrame {
                language: StackLanguage::Go,
                kind: classify(&path, &go_symbol, workspace_root),
                path,
                line: number.parse().ok(),
                column: None,
                function: go_symbol.clone(),
                raw: line.trim().to_owned(),
            });
            go_symbol.clear();
            continue;
        }

        if let (Some(path), Some(number), Some(function)) = (
            capture(&JAVA_FRAME, line, "path"),
            capture(&JAVA_FRAME, line, "line"),
            capture(&JAVA_FRAME, line, "function"),
        ) {
            frames.push(StackFrame {
                language: StackLanguage::Java,
                kind: classify(&path, &function, workspace_root),
                path,
                line: number.parse().ok(),
                column: None,
                function,
                raw: line.trim().to_owned(),
            });
            continue;
        }

        let trimmed = line.trim();
        if trimmed.ends_with("()")
            && trimmed.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        {
            go_symbol = trimmed.to_owned();
        }
    }

    frames
}

fn push_summary(out: &mut String, hidden: usize) {
    if hidden > 0 {
        out.push_str(&format!(
            "... [{hidden} framework/runtime frames hidden] ...\n"
        ));
    }
}

/// Render parsed frames, collapsing each consecutive vendor run to one line.
pub fn collapse_vendor_frames(frames: &[StackFrame]) -> String {
    let mut out = String::new();
    let mut index = 0;

    while index < frames.len() {
        if frames[index].kind == FrameKind::Vendor {
            let start = index;
            while index < frames.len() && frames[index].kind == FrameKind::Vendor {
                index += 1;
            }
            push_summary(&mut out, index - start);
        } else {
            let frame = &frames[index];
            match (frame.line, frame.column) {
                (Some(line), Some(column)) if !frame.function.is_empty() => {
                    out.push_str(&format!(
                        "at {} ({}:{}:{})\n",
                        frame.function, frame.path, line, column
                    ));
                }
                (Some(line), _) if !frame.function.is_empty() => {
                    out.push_str(&format!(
                        "at {} ({}:{})\n",
                        frame.function, frame.path, line
                    ));
                }
                _ => out.push_str(&format!("at {}:{}\n", frame.path, frame.line.unwrap_or(0))),
            }
            index += 1;
        }
    }

    out
}

fn is_symbol_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with(|c: char| c.is_ascii_digit()) && trimmed.contains(':')
        || (trimmed.ends_with("()") && !trimmed.starts_with("at "))
}

/// Replace the frame lines of `text` with their collapsed rendering.
pub fn render_stacktrace(text: &str, workspace_root: &Path, include_vendor_frames: bool) -> String {
    let frames = parse_stacktrace(text, workspace_root, include_vendor_frames);
    let mut by_line: HashMap<&str, &StackFrame> = HashMap::new();
    for frame in &frames {
        by_line.entry(frame.raw.as_str()).or_insert(frame);
    }

    let lines: Vec<&str> = text.lines().collect();
    let mut skip_symbol = vec![false; lines.len()];
    for (index, line) in lines.iter().enumerate() {
        if index + 1 < lines.len()
            && is_symbol_line(line)
            && let Some(frame) = by_line.get(lines[index + 1].trim())
        {
            skip_symbol[index] = frame.kind == FrameKind::Vendor && !include_vendor_frames;
        }
    }

    let mut out = String::new();
    let mut hidden = 0;
    for (index, line) in lines.iter().enumerate() {
        if skip_symbol[index] {
            continue;
        }
        match by_line.get(line.trim()) {
            Some(frame) if frame.kind == FrameKind::Vendor && !include_vendor_frames => hidden += 1,
            _ => {
                push_summary(&mut out, hidden);
                hidden = 0;
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    push_summary(&mut out, hidden);
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::core::models::FrameKind;

    use super::parse_stacktrace;

    #[test]
    fn empty_input_has_no_frames() {
        assert!(parse_stacktrace("", Path::new("."), false).is_empty());
    }

    #[test]
    fn frames_keep_language_and_classification() {
        let frames = parse_stacktrace(
            "TypeError: bad\n    at main (/app/src/main.js:12:3)\n    at node:internal/timers:1:2",
            Path::new("/app"),
            false,
        );
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].kind, FrameKind::App);
        assert_eq!(frames[1].kind, FrameKind::Vendor);
    }
}
