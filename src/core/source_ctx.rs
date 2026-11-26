//! Source-snippet resolution for surviving application frames.
//!
//! Files are read through a `BufReader` and scanning stops past the requested
//! window, so a multi-gigabyte source file does not become a multi-gigabyte
//! allocation.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::core::models::{SourceSnippet, StackFrame};

fn relative_path(path: &str, workspace_root: &Path) -> String {
    let mut normalized = path.replace('\\', "/");
    if let Some(offset) = normalized.find(":///") {
        normalized = normalized[offset + 4..].to_owned();
    }
    normalized = normalized.trim_start_matches("./").to_owned();

    if Path::new(&normalized).is_absolute() {
        if let Ok(stripped) = Path::new(&normalized).strip_prefix(workspace_root) {
            normalized = stripped.to_string_lossy().replace('\\', "/");
        }
    }
    normalized
}

/// Read `context_lines` on either side of a frame's crashing line.
pub fn resolve_frame(
    workspace_root: &Path,
    frame: &StackFrame,
    context_lines: usize,
) -> Option<SourceSnippet> {
    let target = frame.line?;
    let path = relative_path(&frame.path, workspace_root);
    if path.is_empty() {
        return None;
    }

    let file_path: PathBuf = if Path::new(&path).is_absolute() {
        PathBuf::from(&path)
    } else {
        workspace_root.join(&path)
    };
    let file = File::open(file_path).ok()?;

    let start = target.saturating_sub(context_lines).max(1);
    let end = target.saturating_add(context_lines);
    let mut lines = Vec::with_capacity(context_lines.saturating_mul(2) + 1);

    for (index, line) in BufReader::new(file).lines().enumerate() {
        let number = index + 1;
        if number < start {
            continue;
        }
        if number > end {
            break;
        }
        lines.push(line.ok()?);
    }

    if lines.is_empty() {
        return None;
    }

    let first_line = start;
    let last_line = first_line + lines.len() - 1;
    Some(SourceSnippet {
        path,
        start_line: first_line,
        end_line: last_line,
        target_line: target,
        text: lines.join("\n"),
    })
}

/// Render a snippet with line numbers and a `>` marker on the crashing line.
pub fn render_snippet(snippet: &SourceSnippet) -> String {
    let mut out = format!("--- {} ({}:{}) ---\n", snippet.path, snippet.start_line, snippet.end_line);
    for (index, line) in snippet.text.lines().enumerate() {
        let number = snippet.start_line + index;
        let marker = if number == snippet.target_line { '>' } else { ' ' };
        out.push_str(&format!("{marker} {number} | {line}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::core::models::{FrameKind, StackFrame, StackLanguage};

    use super::{relative_path, resolve_frame};

    #[test]
    fn webpack_urls_resolve_to_workspace_paths() {
        assert_eq!(
            relative_path("webpack-internal:///./src/users.ts", Path::new("/app")),
            "src/users.ts"
        );
    }

    #[test]
    fn missing_files_do_not_produce_snippets() {
        let frame = StackFrame {
            language: StackLanguage::Rust,
            kind: FrameKind::App,
            path: "src/missing.rs".into(),
            line: Some(1),
            column: None,
            function: "missing".into(),
            raw: "at src/missing.rs:1".into(),
        };
        assert!(resolve_frame(Path::new("/nonexistent"), &frame, 2).is_none());
    }
}
