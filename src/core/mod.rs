//! Pure-Rust slicing engine shared by the CLI and MCP transports.
//!
//! This module is intentionally free of CLI, MCP and HTTP dependencies: every
//! entry point takes typed structs and returns `anyhow::Result<SliceReport>`.

pub mod compiler;
pub mod dedup;
pub mod models;
pub mod redactor;
pub mod source_ctx;
pub mod sources;
pub mod stacktrace;

use std::path::Path;

use anyhow::{Result, bail};

use models::{
    FrameKind, SliceBuildArgs, SliceBuildReport, SliceContainerRequest, SliceLogRequest, SliceReport,
    SliceServiceRequest, SliceStats,
};
pub use models::{ErrorGroup, SliceStats as Stats, SourceSnippet, StackFrame, StackLanguage};

/// Slice a file, piped text, or raw string into a compact diagnostic report.
pub fn slice_log(request: SliceLogRequest) -> Result<SliceReport> {
    let tail_lines = request.tail_lines.unwrap_or(5000);
    let raw = match (&request.path, &request.raw_text) {
        (Some(path), _) => sources::read_log_tail(Path::new(path), tail_lines)?,
        (None, Some(text)) => sources::tail_text(text, tail_lines),
        _ => bail!("provide path or raw_text to slice"),
    };

    let redacted = redactor::redact(&raw);
    let lines: Vec<String> = redacted.lines().map(str::to_owned).collect();
    let groups = dedup::group_lines(&lines);

    let root = Path::new(request.workspace_root.as_deref().unwrap_or("."));
    let include_vendor = request.include_vendor_frames.unwrap_or(false);
    let frames = stacktrace::parse_stacktrace(&redacted, root, include_vendor);
    let context_lines = request.context_lines.unwrap_or(3);
    let snippets: Vec<_> = frames
        .iter()
        .filter(|frame| frame.kind == FrameKind::App)
        .filter_map(|frame| source_ctx::resolve_frame(root, frame, context_lines))
        .collect();

    let mut sections = Vec::new();
    let bursts: Vec<String> = groups
        .iter()
        .filter(|group| group.count > 1)
        .map(dedup::format_group)
        .collect();
    if !bursts.is_empty() {
        sections.push(format!("== Error bursts ==\n{}", bursts.join("\n")));
    }
    if !frames.is_empty() {
        sections.push(format!(
            "== Stacktrace ==\n{}",
            stacktrace::render_stacktrace(&redacted, root, include_vendor).trim_end()
        ));
    }
    if !snippets.is_empty() {
        let rendered: Vec<String> = snippets.iter().map(source_ctx::render_snippet).collect();
        sections.push(format!("== Source context ==\n{}", rendered.join("\n").trim_end()));
    }
    if sections.is_empty() && !lines.is_empty() {
        sections.push(format!("== Log (last {} lines) ==\n{}", lines.len(), lines.join("\n")));
    }

    // Source snippets are read from disk, so redact once more over the final
    // rendered text before anything can reach an LLM or MCP client.
    let mut text = redactor::redact(&sections.join("\n\n"));
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
    Ok(SliceReport {
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
        frames,
        groups,
        snippets,
    })
}

/// Slice Docker container logs through the same engine as `slice_log`.
pub async fn slice_container(request: SliceContainerRequest) -> Result<SliceReport> {
    let raw = sources::docker_logs(&request).await?;
    slice_log(SliceLogRequest {
        path: None,
        raw_text: Some(raw),
        workspace_root: request.workspace_root,
        tail_lines: request.tail_lines,
        ..Default::default()
    }
    .with_max_chars(request.max_chars))
}

/// Slice a systemd unit's journal through the same engine as `slice_log`.
pub async fn slice_service(request: SliceServiceRequest) -> Result<SliceReport> {
    let raw = sources::journal_logs(&request).await?;
    slice_log(SliceLogRequest {
        path: None,
        raw_text: Some(raw),
        workspace_root: request.workspace_root,
        tail_lines: Some(request.lines.unwrap_or(1000)),
        ..Default::default()
    }
    .with_max_chars(request.max_chars))
}

impl SliceLogRequest {
    fn with_max_chars(mut self, max_chars: Option<usize>) -> Self {
        if let Some(max_chars) = max_chars {
            self.max_chars = Some(max_chars);
        }
        self
    }
}

/// Slice compiler diagnostics into root causes with source context.
pub fn slice_build(request: SliceBuildArgs) -> Result<SliceBuildReport> {
    let raw = match (&request.path, &request.raw_text) {
        (Some(path), _) => std::fs::read_to_string(path)?,
        (None, Some(text)) => text.clone(),
        _ => bail!("provide path or raw_text to build"),
    };
    let redacted = redactor::redact(&raw);
    let (parsed, cascading_suppressed) = compiler::parse_with_cascade_count(&redacted);
    let errors: Vec<_> = parsed
        .into_iter()
        .take(request.max_errors.unwrap_or(10))
        .collect();

    let root = Path::new(request.workspace_root.as_deref().unwrap_or("."));
    let context_lines = request.context_lines.unwrap_or(3);
    let snippets: Vec<_> = errors
        .iter()
        .filter_map(|error| {
            let language = match error.kind {
                models::CompilerKind::Rust => StackLanguage::Rust,
                models::CompilerKind::TypeScript => StackLanguage::TypeScript,
                models::CompilerKind::Go => StackLanguage::Go,
                models::CompilerKind::C => StackLanguage::C,
            };
            let frame = StackFrame {
                language,
                kind: FrameKind::App,
                path: error.path.clone(),
                line: Some(error.line),
                column: error.column,
                function: error.message.clone(),
                raw: error.message.clone(),
            };
            source_ctx::resolve_frame(root, &frame, context_lines)
        })
        .collect();

    let mut rendered = Vec::new();
    for error in &errors {
        let code = error
            .code
            .as_deref()
            .map_or(String::new(), |code| format!(" {code}"));
        let location = error
            .column
            .map_or_else(
                || format!("{}:{}", error.path, error.line),
                |column| format!("{}:{}:{}", error.path, error.line, column),
            );
        let prefix = if error.count > 1 {
            format!("[x{} cascading occurrences]", error.count)
        } else {
            "[1]".to_owned()
        };
        rendered.push(format!("{prefix}{code} {location} — {}", error.message));
        if error.notes_collapsed > 0 {
            rendered.push(format!(
                "    [+{} trait/note lines collapsed]",
                error.notes_collapsed
            ));
        }
        if let Some(snippet) = &error.primary_snippet {
            rendered.push(format!("    | {snippet}"));
        }
    }

    let mut sections = vec![format!(
        "== Compiler errors ==\n{}",
        rendered.join("\n").trim_end()
    )];
    if cascading_suppressed > 0 {
        sections.push(format!(
            "== Cascade suppression ==\n[cascades merged: {cascading_suppressed}]"
        ));
    }
    if !snippets.is_empty() {
        let snippets: Vec<String> = snippets.iter().map(source_ctx::render_snippet).collect();
        sections.push(format!(
            "== Source context ==\n{}",
            snippets.join("\n").trim_end()
        ));
    }

    let mut text = redactor::redact(&sections.join("\n\n"));
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
    Ok(SliceBuildReport {
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
        errors,
        snippets,
        cascading_suppressed,
    })
}
