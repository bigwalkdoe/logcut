//! Secret and PII scrubbing applied before any sliced log leaves the core.
//!
//! The patterns are deliberately conservative: they match credential *shapes*
//! (prefixed API keys, bearer tokens, AWS access keys, authenticated DSNs) and
//! leave surrounding diagnostic text intact.

use std::sync::LazyLock;

use regex::Regex;

static CREDENTIAL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(postgres(?:ql)?|mysql|mongodb(?:\+srv)?|redis|amqp)://[^/@\s:]+:[^@\s/]+@")
        .expect("credential URL regex")
});
static BEARER_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bBearer\s+[A-Za-z0-9._~+/=-]+").expect("bearer token regex"));
static OPENAI_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:sk|rk)-(?:live-|proj-|test-)?[A-Za-z0-9_-]{12,}").expect("OpenAI key regex")
});
static GITHUB_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bgh[pousr]_[A-Za-z0-9]{20,}").expect("GitHub key regex"));
static AWS_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}").expect("AWS key regex"));
static KEY_VALUE_SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(api[_-]?key|access[_-]?token|password|secret)\s*[:=]\s*['"]?[A-Za-z0-9._~+/=-]{8,}"#)
        .expect("key/value secret regex")
});

/// Replace every recognized credential in `input` with `<REDACTED>`.
///
/// Credential-bearing URLs keep their scheme and host so diagnostics remain
/// useful while the userinfo portion is dropped.
pub fn redact(input: &str) -> String {
    let redacted = CREDENTIAL_URL.replace_all(input, "$1://<REDACTED>@");
    let redacted = BEARER_TOKEN.replace_all(&redacted, "Bearer <REDACTED>");
    let redacted = OPENAI_KEY.replace_all(&redacted, "<REDACTED>");
    let redacted = GITHUB_KEY.replace_all(&redacted, "<REDACTED>");
    let redacted = AWS_KEY.replace_all(&redacted, "<REDACTED>");
    KEY_VALUE_SECRET
        .replace_all(&redacted, "$1=<REDACTED>")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn redaction_is_idempotent() {
        let once = redact("Bearer abcdefghijklmnop dsn=postgres://u:p@host/db");
        assert_eq!(redact(&once), once);
    }
}
