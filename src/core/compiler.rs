//! Multi-compiler diagnostic parsing for Rust, TypeScript, Go, and C/C++.
//!
//! Each parser emits root-cause blocks with a primary `path:line:column` span,
//! the primary code snippet, and a count of collapsed note/help lines. Duplicate
//! cascades are merged before the caller renders a compact report.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::core::models::CompilerError;

static RUST_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^error(?:\[([A-Z]\d+)\])?: (.*)$").expect("rust header regex")
});
static RUST_LOCATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-->\s+(.+):(\d+):(\d+)").expect("rust location regex")
});
static RUST_SNIPPET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\d+\s*\|\s*(.*)$").expect("rust snippet regex"));
static TSC_PAREN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(.+)\((\d+),(\d+)\): error (TS\d+): (.*)$").expect("tsc paren regex")
});
static TSC_DASH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(.+):(\d+):(\d+) - error (TS\d+): (.*)$").expect("tsc dash regex")
});
static GO_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\./)?(.+\.go):(\d+):(\d+): (.*)$").expect("go diagnostic regex")
});
static C_ERROR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(.+?):(\d+):(\d+): (?:fatal )?error: (.*)$").expect("c error regex")
});
static C_NOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^.+?:\d+:\d+: note: .*$").expect("c note regex"));

fn parse_number(value: &str) -> usize {
    value.parse().unwrap_or(0)
}

fn rust_errors(lines: &[&str]) -> Vec<(usize, CompilerError)> {
    let headers: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| RUST_HEADER.captures(line).map(|_| index))
        .collect();
    let mut errors = Vec::new();

    for (position, &start) in headers.iter().enumerate() {
        let Some(header) = RUST_HEADER.captures(lines[start]) else {
            continue;
        };
        let end = headers.get(position + 1).copied().unwrap_or(lines.len());
        let block = &lines[start..end];
        let Some(span) = block
            .iter()
            .find_map(|line| RUST_LOCATION.captures(line))
        else {
            continue;
        };
        let notes = block
            .iter()
            .filter(|line| {
                let trimmed = line.trim_start();
                trimmed.starts_with("note:")
                    || trimmed.starts_with("help:")
                    || trimmed.contains("required by a bound")
            })
            .count();
        let snippet = block
            .iter()
            .filter_map(|line| RUST_SNIPPET.captures(line))
            .map(|capture| capture[1].trim().to_owned())
            .find(|line| !line.is_empty());

        errors.push((
            start,
            CompilerError {
                kind: crate::core::models::CompilerKind::Rust,
                code: header.get(1).map(|code| code.as_str().to_owned()),
                message: header[2].to_owned(),
                path: span[1].to_owned(),
                line: parse_number(&span[2]),
                column: Some(parse_number(&span[3])),
                count: 1,
                notes_collapsed: notes,
                primary_snippet: snippet,
            },
        ));
    }
    errors
}

fn tsc_errors(lines: &[&str]) -> Vec<(usize, CompilerError)> {
    use crate::core::models::CompilerKind;

    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            if let Some(capture) = TSC_PAREN.captures(line) {
                Some((
                    index,
                    CompilerError {
                        kind: CompilerKind::TypeScript,
                        code: Some(capture[4].to_owned()),
                        message: capture[5].to_owned(),
                        path: capture[1].to_owned(),
                        line: parse_number(&capture[2]),
                        column: Some(parse_number(&capture[3])),
                        count: 1,
                        notes_collapsed: 0,
                        primary_snippet: None,
                    },
                ))
            } else {
                TSC_DASH.captures(line).map(|capture| {
                    (
                        index,
                        CompilerError {
                            kind: CompilerKind::TypeScript,
                            code: Some(capture[4].to_owned()),
                            message: capture[5].to_owned(),
                            path: capture[1].to_owned(),
                            line: parse_number(&capture[2]),
                            column: Some(parse_number(&capture[3])),
                            count: 1,
                            notes_collapsed: 0,
                            primary_snippet: None,
                        },
                    )
                })
            }
        })
        .collect()
}

fn go_errors(lines: &[&str]) -> Vec<(usize, CompilerError)> {
    use crate::core::models::CompilerKind;

    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            GO_LINE.captures(line).map(|capture| {
                (
                    index,
                    CompilerError {
                        kind: CompilerKind::Go,
                        code: None,
                        message: capture[4].to_owned(),
                        path: capture[1].to_owned(),
                        line: parse_number(&capture[2]),
                        column: Some(parse_number(&capture[3])),
                        count: 1,
                        notes_collapsed: 0,
                        primary_snippet: None,
                    },
                )
            })
        })
        .collect()
}

fn c_errors(lines: &[&str]) -> Vec<(usize, CompilerError)> {
    use crate::core::models::CompilerKind;

    let mut errors: Vec<(usize, CompilerError)> = Vec::new();
    let mut last_error: Option<usize> = None;
    for (index, line) in lines.iter().enumerate() {
        if let Some(capture) = C_ERROR.captures(line) {
            last_error = Some(errors.len());
            errors.push((
                index,
                CompilerError {
                    kind: CompilerKind::C,
                    code: None,
                    message: capture[4].to_owned(),
                    path: capture[1].to_owned(),
                    line: parse_number(&capture[2]),
                    column: Some(parse_number(&capture[3])),
                    count: 1,
                    notes_collapsed: 0,
                    primary_snippet: None,
                },
            ));
        } else if C_NOTE.is_match(line) {
            if let Some(position) = last_error {
                errors[position].1.notes_collapsed += 1;
            }
        }
    }
    errors
}

fn normalized_message(message: &str) -> String {
    message
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

fn deduplicate(parsed: Vec<(usize, CompilerError)>) -> (Vec<CompilerError>, usize) {
    let mut ordered = parsed;
    ordered.sort_by_key(|(index, _)| *index);

    let mut unique: Vec<CompilerError> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut suppressed = 0;

    for (_, error) in ordered {
        let key = format!(
            "{:?}|{}|{}|{}",
            error.kind,
            error.code.as_deref().unwrap_or(""),
            error.path,
            normalized_message(&error.message)
        );
        if let Some(&position) = seen.get(&key) {
            unique[position].count += 1;
            unique[position].notes_collapsed += error.notes_collapsed;
            suppressed += 1;
        } else {
            seen.insert(key, unique.len());
            unique.push(error);
        }
    }
    (unique, suppressed)
}

/// Parse all supported diagnostics and merge repeated cascades.
pub fn parse_all(text: &str) -> Vec<CompilerError> {
    let lines: Vec<&str> = text.lines().collect();
    let mut parsed = Vec::new();
    parsed.extend(rust_errors(&lines));
    parsed.extend(tsc_errors(&lines));
    parsed.extend(go_errors(&lines));
    parsed.extend(c_errors(&lines));
    deduplicate(parsed).0
}

/// Parse all diagnostics and also report how many cascades were merged.
pub fn parse_with_cascade_count(text: &str) -> (Vec<CompilerError>, usize) {
    let lines: Vec<&str> = text.lines().collect();
    let mut parsed = Vec::new();
    parsed.extend(rust_errors(&lines));
    parsed.extend(tsc_errors(&lines));
    parsed.extend(go_errors(&lines));
    parsed.extend(c_errors(&lines));
    deduplicate(parsed)
}
