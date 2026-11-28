//! NDJSON detection, filtering and breadcrumb correlation.

use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, bail};
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

/// Strict parser for caller-supplied severity values. Unknown log levels are
/// deliberately not rejected here: `severity()` still defaults them to info.
fn parse_level(level: &str) -> Option<u8> {
    match level.trim().to_ascii_lowercase().as_str() {
        "trace" => Some(0),
        "debug" => Some(1),
        "info" => Some(2),
        "warn" | "warning" => Some(3),
        "error" => Some(4),
        "fatal" => Some(5),
        _ => None,
    }
}

/// Numeric severity used by `--min-level`; unknown levels are treated as info.
pub fn severity(level: &str) -> u8 {
    parse_level(level).unwrap_or(2)
}

/// Validate caller-supplied options before any input is classified or sliced.
pub fn validate(min_level: Option<&str>, where_filters: &[String]) -> Result<()> {
    if let Some(level) = min_level
        && parse_level(level).is_none()
    {
        bail!("invalid min_level {level:?}; expected trace, debug, info, warn, error, or fatal");
    }
    for filter in where_filters {
        if parse_filter(filter).is_none() {
            bail!(
                "invalid where filter {filter:?}; expected field=value, field!=value, field>value, field>=value, field<value, or field<=value"
            );
        }
    }
    Ok(())
}

/// Parse, filter and correlate NDJSON lines into normalized entries.
pub fn slice(
    text: &str,
    min_level: Option<&str>,
    where_filters: &[String],
) -> Result<Vec<JsonLogEntry>> {
    validate(min_level, where_filters)?;
    let min_severity = min_level.and_then(parse_level);
    let mut filters = Vec::with_capacity(where_filters.len());
    for filter in where_filters {
        if let Some(parsed) = parse_filter(filter) {
            filters.push(parsed);
        }
    }

    // Once a stream is classified as NDJSON, still preserve non-JSON lines as
    // INFO context. Dropping half a mixed stream would lose diagnostics.
    let all: Vec<JsonLogEntry> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| match serde_json::from_str::<Value>(line) {
            Ok(value) => parse_entry(&value).or_else(|| Some(plain_entry(line))),
            Err(_) => Some(plain_entry(line)),
        })
        .collect();
    let entries: Vec<JsonLogEntry> = all
        .iter()
        .enumerate()
        .filter(|(_, entry)| min_severity.is_none_or(|level| severity(&entry.level) >= level))
        .filter(|(_, entry)| {
            filters.iter().all(|(field, operator, expected)| {
                matches_filter(entry, field, *operator, expected)
            })
        })
        .map(|(index, entry)| {
            let mut entry = entry.clone();
            entry.breadcrumbs = breadcrumbs_for(&all, index);
            entry
        })
        .collect();
    Ok(entries)
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

fn plain_entry(line: &str) -> JsonLogEntry {
    JsonLogEntry {
        timestamp: None,
        level: "INFO".to_owned(),
        message: redact(line),
        error: None,
        stack: None,
        trace_id: None,
        request_id: None,
        fields: BTreeMap::new(),
        breadcrumbs: Vec::new(),
    }
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
        .map(|(key, value)| (redact(key), redact(&stringify(&without_metadata(value)))))
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

/// Remove static host/process metadata from nested objects and arrays before
/// they are stringified into a report field.
fn without_metadata(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .filter(|(key, _)| !METADATA_KEYS.contains(&key.as_str()))
                .map(|(key, nested)| (key.clone(), without_metadata(nested)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(without_metadata).collect()),
        other => other.clone(),
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
            let value = value.trim();
            // Commit to the first (longest) operator token. Falling through to
            // a shorter one would parse `status>= ` as `status > "="`.
            return (!field.is_empty() && !value.is_empty()).then_some((field, operator, value));
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

fn matches_filter(entry: &JsonLogEntry, field: &str, operator: Operator, expected: &str) -> bool {
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
    fn unknown_min_level_is_rejected() {
        let err = slice("{\"level\":\"info\",\"message\":\"x\"}", Some("loud"), &[]).unwrap_err();
        assert!(err.to_string().contains("loud"), "{err:#}");
    }

    #[test]
    fn malformed_where_filters_are_rejected() {
        for filter in ["status", "=auth", "status=", "   ", "status>= "] {
            let err = slice(
                "{\"level\":\"info\",\"message\":\"x\"}",
                None,
                &[filter.to_owned()],
            )
            .unwrap_err();
            assert!(err.to_string().contains(filter), "{err:#}");
        }
    }

    #[test]
    fn min_level_warn_excludes_info() {
        let text = [
            r#"{"level":"INFO","message":"noise"}"#,
            r#"{"level":"WARN","message":"careful"}"#,
            r#"{"level":"ERROR","message":"broken"}"#,
        ]
        .join("\n");
        let entries = slice(&text, Some("warn"), &[]).unwrap();
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
        let entries = slice(&text, None, &["status>=500".to_owned()]).unwrap();
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
        let entries = slice(&text, None, &["service=auth".to_owned()]).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "a");
    }

    #[test]
    fn rendered_output_omits_static_metadata() {
        let text = r#"{"timestamp":"2026-11-28T00:00:00Z","level":"info","message":"ready","service":"api","pid":42,"hostname":"worker-1","v":3}"#;
        let output = render(&slice(text, None, &[]).unwrap());
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
        let output = render(&slice(text, None, &[]).unwrap());
        assert!(output.contains("Bearer <REDACTED>"));
        assert!(!output.contains("abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn structured_stream_keeps_plain_context_lines() {
        let text = [
            r#"{"level":"info","message":"structured one"}"#,
            "plain context line",
            r#"{"level":"error","message":"structured failure"}"#,
            "plain shutdown line",
        ]
        .join("\n");

        assert!(is_ndjson(&text));
        let entries = slice(&text, None, &[]).unwrap();
        assert_eq!(entries.len(), 4, "{entries:?}");
        assert_eq!(entries[1].message, "plain context line");
        assert_eq!(entries[1].level, "INFO");
        assert!(render(&entries).contains("plain context line"));
        assert!(render(&entries).contains("plain shutdown line"));
    }

    #[test]
    fn nested_metadata_is_stripped_recursively() {
        let text = r#"{"level":"error","message":"boom","meta":{"pid":42,"hostname":"worker-1","v":3,"service":"auth","nested":{"pid":7,"region":"eu"}}}"#;
        let entries = slice(text, None, &[]).unwrap();
        let output = render(&entries);

        assert!(output.contains("\"service\":\"auth\""), "{output}");
        assert!(output.contains("\"region\":\"eu\""), "{output}");
        for leaked in ["pid", "hostname", "worker-1", "v=3", "v\":3"] {
            assert!(!output.contains(leaked), "leaked {leaked:?}: {output}");
        }
        let fields = serde_json::to_string(&entries[0].fields).unwrap();
        for leaked in ["pid", "hostname", "worker-1", "\"v\":3"] {
            assert!(!fields.contains(leaked), "leaked {leaked:?}: {fields}");
        }
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

        let entries = slice(&lines.join("\n"), Some("error"), &[]).unwrap();
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

        let entries = slice(&lines.join("\n"), Some("error"), &[]).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].breadcrumbs.len(), 5);
        assert_eq!(entries[0].breadcrumbs[4].message, "before-7");
        assert!(render(&entries).contains("breadcrumbs:"));
    }
}
