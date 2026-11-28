//! NDJSON detection, filtering and breadcrumb correlation.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Value};

use crate::core::models::{JsonBreadcrumb, JsonLogEntry};
use crate::core::redactor::redact;

const MAX_BREADCRUMBS: usize = 5;

/// Keys promoted to first-class fields on [`JsonLogEntry`].
const EXTRACTED_KEYS: [&str; 8] = [
    "timestamp",
    "level",
    "message",
    "msg",
    "error",
    "stack",
    "trace_id",
    "request_id",
];

/// Process/host metadata that never belongs in a diagnostic slice.
const METADATA_KEYS: [&str; 3] = ["pid", "hostname", "v"];

/// Detect a predominantly structured (NDJSON) log without hijacking mixed
/// plain/JSON streams: at least half of the non-empty lines must be JSON
/// objects carrying `level` or `message`.
pub fn is_ndjson(text: &str) -> bool {
    let mut total = 0usize;
    let mut structured = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        total += 1;
        if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(line)
            && (object.contains_key("level") || object.contains_key("message"))
        {
            structured += 1;
        }
    }
    structured > 0 && structured >= total.div_ceil(2)
}

/// Numeric severity used by `--min-level`; unknown levels are treated as info.
pub fn severity(level: &str) -> u8 {
    match level.trim().to_ascii_lowercase().as_str() {
        "trace" => 0,
        "debug" => 1,
        "info" => 2,
        "warn" | "warning" => 3,
        "error" => 4,
        "fatal" => 5,
        _ => 2,
    }
}

/// Parse, filter and correlate NDJSON lines into normalized entries.
pub fn slice(text: &str, min_level: Option<&str>, where_filters: &[String]) -> Vec<JsonLogEntry> {
    let all: Vec<JsonLogEntry> = text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter_map(|value| parse_entry(&value))
        .collect();
    let min_severity = min_level.map(severity);

    all.iter()
        .enumerate()
        .filter(|(_, entry)| min_severity.is_none_or(|level| severity(&entry.level) >= level))
        .filter(|(_, entry)| {
            where_filters
                .iter()
                .all(|filter| matches_filter(entry, filter))
        })
        .map(|(index, entry)| {
            let mut entry = entry.clone();
            entry.breadcrumbs = breadcrumbs_for(&all, index);
            entry
        })
        .collect()
}

/// Render entries as a compact, delimited section.
pub fn render(entries: &[JsonLogEntry]) -> String {
    let mut out = String::from("== Structured logs ==");
    for entry in entries {
        out.push('\n');
        push_heading(
            &mut out,
            entry.timestamp.as_deref(),
            &entry.level,
            &entry.message,
        );
        if let Some(error) = &entry.error {
            out.push_str(&format!("\n  error: {}", redact(error)));
        }
        if let Some(stack) = &entry.stack {
            out.push_str(&format!("\n  stack: {}", redact(stack)));
        }
        if !entry.fields.is_empty() {
            let fields = entry
                .fields
                .iter()
                .map(|(key, value)| format!("{}={}", redact(key), redact(value)))
                .collect::<Vec<_>>()
                .join(" ");
            out.push_str(&format!("\n  fields: {fields}"));
        }
        if !entry.breadcrumbs.is_empty() {
            out.push_str("\n  breadcrumbs:");
            for crumb in &entry.breadcrumbs {
                out.push_str("\n  ");
                push_heading(
                    &mut out,
                    crumb.timestamp.as_deref(),
                    &crumb.level,
                    &crumb.message,
                );
            }
        }
    }
    out
}

fn push_heading(out: &mut String, timestamp: Option<&str>, level: &str, message: &str) {
    if let Some(timestamp) = timestamp {
        out.push_str(&redact(timestamp));
        out.push(' ');
    }
    out.push_str(&redact(level));
    out.push(' ');
    out.push_str(&redact(message));
}

fn parse_entry(value: &Value) -> Option<JsonLogEntry> {
    let object = value.as_object()?;
    let timestamp = string_field(object, "timestamp");
    let level = string_field(object, "level")
        .map(|level| level.to_ascii_uppercase())
        .unwrap_or_else(|| "INFO".to_owned());
    let message = string_field(object, "message")
        .or_else(|| string_field(object, "msg"))
        .unwrap_or_default();
    let reserved: HashSet<&str> = EXTRACTED_KEYS
        .iter()
        .copied()
        .chain(METADATA_KEYS)
        .collect();
    let fields = object
        .iter()
        .filter(|(key, _)| !reserved.contains(key.as_str()))
        .map(|(key, value)| (redact(key), redact(&stringify(value))))
        .collect::<BTreeMap<_, _>>();

    Some(JsonLogEntry {
        timestamp,
        level,
        message,
        error: string_field(object, "error"),
        stack: string_field(object, "stack"),
        trace_id: string_field(object, "trace_id"),
        request_id: string_field(object, "request_id"),
        fields,
        breadcrumbs: Vec::new(),
    })
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    match object.get(key) {
        Some(Value::String(value)) => Some(redact(value)),
        _ => None,
    }
}

fn stringify(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

#[derive(Clone, Copy)]
enum Operator {
    GreaterOrEqual,
    LessOrEqual,
    NotEqual,
    Equal,
    Greater,
    Less,
}

fn parse_filter(filter: &str) -> Option<(&str, Operator, &str)> {
    let filter = filter.trim();
    let operators: [(&str, Operator); 7] = [
        (">=", Operator::GreaterOrEqual),
        ("<=", Operator::LessOrEqual),
        ("!=", Operator::NotEqual),
        ("==", Operator::Equal),
        ("=", Operator::Equal),
        (">", Operator::Greater),
        ("<", Operator::Less),
    ];
    for (token, operator) in operators {
        if let Some((field, value)) = filter.split_once(token) {
            let field = field.trim();
            if !field.is_empty() {
                return Some((field, operator, value.trim()));
            }
        }
    }
    None
}

fn field_value<'a>(entry: &'a JsonLogEntry, field: &str) -> Option<&'a str> {
    match field {
        "timestamp" => entry.timestamp.as_deref(),
        "level" => Some(entry.level.as_str()),
        "message" | "msg" => Some(entry.message.as_str()),
        "error" => entry.error.as_deref(),
        "stack" => entry.stack.as_deref(),
        "trace_id" => entry.trace_id.as_deref(),
        "request_id" => entry.request_id.as_deref(),
        other => entry.fields.get(other).map(String::as_str),
    }
}

fn matches_filter(entry: &JsonLogEntry, filter: &str) -> bool {
    let Some((field, operator, expected)) = parse_filter(filter) else {
        return true;
    };
    let Some(actual) = field_value(entry, field) else {
        return false;
    };
    if let (Ok(left), Ok(right)) = (actual.trim().parse::<f64>(), expected.trim().parse::<f64>()) {
        match operator {
            Operator::GreaterOrEqual => left >= right,
            Operator::LessOrEqual => left <= right,
            Operator::NotEqual => left != right,
            Operator::Equal => left == right,
            Operator::Greater => left > right,
            Operator::Less => left < right,
        }
    } else {
        match operator {
            Operator::GreaterOrEqual => actual >= expected,
            Operator::LessOrEqual => actual <= expected,
            Operator::NotEqual => actual != expected,
            Operator::Equal => actual == expected,
            Operator::Greater => actual > expected,
            Operator::Less => actual < expected,
        }
    }
}

/// Attach the last five preceding INFO/WARN lines that share either
/// correlation ID. The source is every parsed line, not just the filtered
/// subset, so context survives `--min-level` and `--where`.
fn breadcrumbs_for(all: &[JsonLogEntry], index: usize) -> Vec<JsonBreadcrumb> {
    let entry = &all[index];
    if !matches!(entry.level.as_str(), "ERROR" | "FATAL")
        || (entry.trace_id.is_none() && entry.request_id.is_none())
    {
        return Vec::new();
    }

    let mut breadcrumbs: Vec<JsonBreadcrumb> = all[..index]
        .iter()
        .filter(|candidate| matches!(candidate.level.as_str(), "INFO" | "WARN"))
        .filter(|candidate| {
            (entry.trace_id.is_some() && candidate.trace_id == entry.trace_id)
                || (entry.request_id.is_some() && candidate.request_id == entry.request_id)
        })
        .rev()
        .take(MAX_BREADCRUMBS)
        .map(|candidate| JsonBreadcrumb {
            timestamp: candidate.timestamp.clone(),
            level: candidate.level.clone(),
            message: candidate.message.clone(),
        })
        .collect();
    breadcrumbs.reverse();
    breadcrumbs
}

#[cfg(test)]
mod tests {
    use super::{is_ndjson, render, severity, slice};

    #[test]
    fn mixed_plain_and_json_lines_are_not_ndjson() {
        let text = [
            "plain INFO server listening",
            r#"{"level":"info","message":"structured one"}"#,
            "plain ERROR connection refused",
            r#"{"level":"error","message":"structured two"}"#,
            "plain INFO shutting down",
        ]
        .join("\n");
        assert!(!is_ndjson(&text));
    }

    #[test]
    fn pure_ndjson_is_detected() {
        let text = [
            r#"{"level":"info","message":"started"}"#,
            r#"{"level":"warn","msg":"slow query"}"#,
            "",
            r#"{"message":"finished"}"#,
        ]
        .join("\n");
        assert!(is_ndjson(&text));
        assert!(!is_ndjson(""));
        assert!(!is_ndjson("plain text\nmore plain text"));
    }

    #[test]
    fn severity_maps_known_levels_and_defaults_unknown_to_info() {
        assert_eq!(severity("trace"), 0);
        assert_eq!(severity("DEBUG"), 1);
        assert_eq!(severity("info"), 2);
        assert_eq!(severity("warn"), 3);
        assert_eq!(severity("WARNING"), 3);
        assert_eq!(severity("error"), 4);
        assert_eq!(severity("Fatal"), 5);
        assert_eq!(severity("something else"), 2);
    }

    #[test]
    fn min_level_warn_excludes_info() {
        let text = [
            r#"{"level":"INFO","message":"noise"}"#,
            r#"{"level":"WARN","message":"careful"}"#,
            r#"{"level":"ERROR","message":"broken"}"#,
        ]
        .join("\n");
        let entries = slice(&text, Some("warn"), &[]);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>(),
            ["careful", "broken"]
        );
    }

    #[test]
    fn numeric_filter_includes_500_but_excludes_499() {
        let text = [
            r#"{"level":"info","message":"edge","status":500}"#,
            r#"{"level":"error","message":"client","status":499}"#,
        ]
        .join("\n");
        let entries = slice(&text, None, &["status>=500".to_owned()]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "edge");
    }

    #[test]
    fn string_filter_selects_service() {
        let text = [
            r#"{"level":"info","message":"a","service":"auth"}"#,
            r#"{"level":"info","message":"b","service":"billing"}"#,
        ]
        .join("\n");
        let entries = slice(&text, None, &["service=auth".to_owned()]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "a");
    }

    #[test]
    fn rendered_output_omits_static_metadata() {
        let text = r#"{"timestamp":"2026-11-28T00:00:00Z","level":"info","message":"ready","service":"api","pid":42,"hostname":"worker-1","v":3}"#;
        let output = render(&slice(text, None, &[]));
        assert!(output.starts_with("== Structured logs ==\n"));
        assert!(output.contains("2026-11-28T00:00:00Z INFO ready"));
        assert!(output.contains("service=api"));
        assert!(!output.contains("pid"));
        assert!(!output.contains("hostname"));
        assert!(!output.contains("worker-1"));
    }

    #[test]
    fn bearer_tokens_are_redacted() {
        let text =
            r#"{"level":"info","message":"Authorization: Bearer abcdefghijklmnopqrstuvwxyz"}"#;
        let output = render(&slice(text, None, &[]));
        assert!(output.contains("Bearer <REDACTED>"));
        assert!(!output.contains("abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn trace_id_correlation_attaches_last_five_preceding_entries() {
        let mut lines: Vec<String> = (1..=7)
            .map(|number| {
                format!(
                    r#"{{"timestamp":"t{number}","level":"INFO","message":"before-{number}","trace_id":"tr_1"}}"#
                )
            })
            .collect();
        lines.push(
            r#"{"timestamp":"t8","level":"ERROR","message":"boom","trace_id":"tr_1"}"#.to_owned(),
        );
        lines.push(r#"{"level":"INFO","message":"after","trace_id":"tr_1"}"#.to_owned());

        let entries = slice(&lines.join("\n"), Some("error"), &[]);
        assert_eq!(entries.len(), 1);
        let breadcrumbs = &entries[0].breadcrumbs;
        assert_eq!(breadcrumbs.len(), 5);
        assert_eq!(
            breadcrumbs
                .iter()
                .map(|crumb| crumb.message.as_str())
                .collect::<Vec<_>>(),
            ["before-3", "before-4", "before-5", "before-6", "before-7"]
        );
        assert_eq!(breadcrumbs[0].timestamp.as_deref(), Some("t3"));
    }

    #[test]
    fn request_id_correlation_survives_min_level_filtering() {
        let mut lines: Vec<String> = (1..=7)
            .map(|number| {
                format!(r#"{{"level":"WARN","message":"before-{number}","request_id":"req_1"}}"#)
            })
            .collect();
        lines.push(r#"{"level":"FATAL","message":"boom","request_id":"req_1"}"#.to_owned());

        let entries = slice(&lines.join("\n"), Some("error"), &[]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].breadcrumbs.len(), 5);
        assert_eq!(entries[0].breadcrumbs[4].message, "before-7");
        assert!(render(&entries).contains("breadcrumbs:"));
    }
}
