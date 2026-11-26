//! Repeated-error clustering and volatile-token normalization.

use std::sync::LazyLock;

use regex::Regex;

use crate::core::models::ErrorGroup;

static ISO_TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?")
        .expect("ISO timestamp regex")
});
static CLOCK_TIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d{2}:\d{2}:\d{2}\b").expect("clock time regex"));
static UUID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")
        .expect("UUID regex")
});
static HEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b0x[0-9a-fA-F]+\b").expect("hex regex"));
static REQUEST_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\breq_[0-9a-fA-F]+\b").expect("request id regex"));
static DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d+(?:\.\d+)?\s?(?:ms|µs|us|s)\b").expect("duration regex"));

/// Replace volatile tokens with stable placeholders so identical errors cluster.
pub fn normalize_line(line: &str) -> String {
    let normalized = ISO_TIMESTAMP.replace_all(line, "<TS>");
    let normalized = CLOCK_TIME.replace_all(&normalized, "<TS>");
    let normalized = UUID.replace_all(&normalized, "<UUID>");
    let normalized = HEX.replace_all(&normalized, "<HEX>");
    let normalized = REQUEST_ID.replace_all(&normalized, "<REQID>");
    let normalized = DURATION.replace_all(&normalized, "<DUR>");
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract the wall-clock time of a line, if it carries one.
fn time_of(line: &str) -> Option<String> {
    if let Some(found) = ISO_TIMESTAMP.find(line) {
        let stamp = &line[found.range()];
        let time = stamp.get(11..19).or_else(|| stamp.get(1..9))?;
        return Some(time.to_owned());
    }
    CLOCK_TIME.find(line).map(|found| found.as_str().to_owned())
}

/// Group repeated normalized lines in first-seen order, keeping burst boundaries.
pub fn group_lines(lines: &[String]) -> Vec<ErrorGroup> {
    let mut groups: Vec<ErrorGroup> = Vec::new();

    for line in lines {
        let template = normalize_line(line);
        if template.is_empty() {
            continue;
        }
        let time = time_of(line);

        if let Some(group) = groups.iter_mut().find(|group| group.template == template) {
            group.count += 1;
            if time.is_some() {
                group.last = time;
            }
        } else {
            groups.push(ErrorGroup {
                template,
                count: 1,
                first: time.clone(),
                last: time,
            });
        }
    }

    groups
}

fn with_commas(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Render one cluster as a single compact report line.
pub fn format_group(group: &ErrorGroup) -> String {
    let count = with_commas(group.count);
    // The burst header already carries the timestamps, so a leading <TS>
    // placeholder in the template would be redundant noise.
    let compact = match (&group.first, &group.last) {
        (Some(_), Some(_)) => group
            .template
            .strip_prefix("<TS> ")
            .unwrap_or(&group.template),
        _ => &group.template,
    };
    match (&group.first, &group.last) {
        (Some(first), Some(last)) => {
            format!("[x{count} | first: {first} | last: {last}] {compact}")
        }
        (Some(at), None) | (None, Some(at)) => format!("[x{count} | at: {at}] {compact}"),
        (None, None) => format!("[x{count}] {compact}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{format_group, group_lines, with_commas};

    #[test]
    fn comma_grouping_handles_thousands() {
        assert_eq!(with_commas(2), "2");
        assert_eq!(with_commas(2000), "2,000");
    }

    #[test]
    fn singleton_groups_have_no_timestamps() {
        let lines = vec!["ERROR disk full".to_owned()];
        assert_eq!(
            format_group(&group_lines(&lines)[0]),
            "[x1] ERROR disk full"
        );
    }
}
