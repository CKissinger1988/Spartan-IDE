//! Real secrets detection/redaction (§9, §36, task #6) -- the "Secrets
//! scanning pass runs before any diff is shown to Leo as context" line
//! from §9. Deliberately scoped to *this* half of §9/§36.4 only, not
//! path-jailing (§36.4.6): jailing constrains a real agent tool-execution
//! layer's file resolution, and no such layer exists yet in this
//! workspace (no Leo, no `ModelProvider`, §75.4/§75.5 remain unbuilt) --
//! building a jail with no real caller to enforce it against would be
//! inert scaffolding, not a load-bearing security control, so it's named
//! here as a real, open gap rather than attempted. Secrets detection, by
//! contrast, has a real, honest, immediately useful call site *today*
//! independent of any agent: warning a user when a file they just opened
//! appears to contain a live credential, wired into `spartan-editor-
//! core`'s own `open_file()` in this same pass.

use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    AwsAccessKeyId,
    GitHubToken,
    SlackToken,
    StripeLiveKey,
    GoogleApiKey,
    PrivateKeyBlock,
    /// A lower-confidence catch-all: an assignment like `api_key = "..."`
    /// or `password: "..."` with a long-enough value -- real secrets take
    /// many shapes no fixed-prefix pattern above can enumerate, so this
    /// exists specifically to catch what the specific patterns miss, at
    /// the cost of a real, named false-positive rate (see the crate's own
    /// tests for what that tradeoff looks like in practice).
    GenericAssignment,
}

impl SecretKind {
    fn label(self) -> &'static str {
        match self {
            SecretKind::AwsAccessKeyId => "AWS_ACCESS_KEY_ID",
            SecretKind::GitHubToken => "GITHUB_TOKEN",
            SecretKind::SlackToken => "SLACK_TOKEN",
            SecretKind::StripeLiveKey => "STRIPE_LIVE_KEY",
            SecretKind::GoogleApiKey => "GOOGLE_API_KEY",
            SecretKind::PrivateKeyBlock => "PRIVATE_KEY",
            SecretKind::GenericAssignment => "POSSIBLE_SECRET",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretFinding {
    pub kind: SecretKind,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// One `(kind, compiled pattern)` per real, specific credential format,
/// each compiled exactly once via `OnceLock` (no external lazy-static
/// dependency needed for a fixed, small pattern set). Ordered
/// most-specific-first so a later, broader pass (`GenericAssignment`)
/// never re-flags a byte range an earlier, more specific pattern already
/// claimed (`scan`'s own overlap-skip logic below relies on this order).
fn patterns() -> &'static Vec<(SecretKind, Regex)> {
    static PATTERNS: OnceLock<Vec<(SecretKind, Regex)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            (
                SecretKind::AwsAccessKeyId,
                Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(),
            ),
            (
                SecretKind::GitHubToken,
                Regex::new(r"gh[pousr]_[A-Za-z0-9]{36}").unwrap(),
            ),
            (
                SecretKind::SlackToken,
                Regex::new(r"xox[baprs]-[A-Za-z0-9-]{10,48}").unwrap(),
            ),
            (
                SecretKind::StripeLiveKey,
                Regex::new(r"sk_live_[A-Za-z0-9]{24,}").unwrap(),
            ),
            (
                SecretKind::GoogleApiKey,
                Regex::new(r"AIza[0-9A-Za-z\-_]{35}").unwrap(),
            ),
            (
                SecretKind::PrivateKeyBlock,
                Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----").unwrap(),
            ),
            (
                SecretKind::GenericAssignment,
                Regex::new(
                    r#"(?i)(api[_-]?key|secret|token|password)\s*[:=]\s*['"]([A-Za-z0-9/+_\-]{16,})['"]"#,
                )
                .unwrap(),
            ),
        ]
    })
}

/// Real scan over `text`, returning every non-overlapping match, in
/// document order. `GenericAssignment` (last in `patterns()`) only
/// contributes a finding for a byte range no earlier, more specific
/// pattern already matched -- so a real GitHub token embedded in
/// `token = "ghp_..."` is reported once, as `GitHubToken`, not twice.
pub fn scan(text: &str) -> Vec<SecretFinding> {
    let mut findings: Vec<SecretFinding> = Vec::new();
    for (kind, re) in patterns() {
        for m in re.find_iter(text) {
            let overlaps = findings
                .iter()
                .any(|f| m.start() < f.end_byte && f.start_byte < m.end());
            if !overlaps {
                findings.push(SecretFinding {
                    kind: *kind,
                    start_byte: m.start(),
                    end_byte: m.end(),
                });
            }
        }
    }
    findings.sort_by_key(|f| f.start_byte);
    findings
}

/// Real redaction: every real finding's byte span replaced with
/// `[REDACTED:<KIND>]` in one left-to-right pass. Returns the redacted
/// text alongside the same findings `scan` would have returned, so a
/// caller can both show a human-readable warning (via `findings`) and
/// safely forward `redacted_text` to a cloud provider (once one exists).
pub fn redact(text: &str) -> (String, Vec<SecretFinding>) {
    let findings = scan(text);
    if findings.is_empty() {
        return (text.to_string(), findings);
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for finding in &findings {
        out.push_str(&text[cursor..finding.start_byte]);
        out.push_str("[REDACTED:");
        out.push_str(finding.kind.label());
        out.push(']');
        cursor = finding.end_byte;
    }
    out.push_str(&text[cursor..]);
    (out, findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every "secret" below is a synthetic, non-functional, format-valid
    // fixture -- the same deliberate-fake-credential convention any real
    // secret-scanner's own test suite uses, never a real live credential.

    #[test]
    fn a_clean_file_with_no_secrets_produces_no_findings() {
        let text = "fn main() {\n    println!(\"hello\");\n}\n";
        assert!(scan(text).is_empty());
    }

    #[test]
    fn detects_an_aws_access_key_id() {
        let text = "aws_access_key_id = AKIAABCDEFGHIJKLMNOP";
        let findings = scan(text);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, SecretKind::AwsAccessKeyId);
        let matched = &text[findings[0].start_byte..findings[0].end_byte];
        assert_eq!(matched, "AKIAABCDEFGHIJKLMNOP");
    }

    #[test]
    fn detects_a_github_token() {
        let text = "GITHUB_TOKEN=ghp_0123456789abcdef0123456789abcdef0123";
        let findings = scan(text);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, SecretKind::GitHubToken);
    }

    #[test]
    fn detects_a_slack_token() {
        let text = "token: xoxb-111111111111-222222222222-abcdefghijklmnopqrstuvwx";
        let findings = scan(text);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, SecretKind::SlackToken);
    }

    #[test]
    fn detects_a_stripe_live_key() {
        let text = "STRIPE_KEY=sk_live_0123456789abcdefghijklmnop";
        let findings = scan(text);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, SecretKind::StripeLiveKey);
    }

    #[test]
    fn detects_a_google_api_key() {
        let text = "apiKey: \"AIzaSyD-1234567890abcdefghijklmnopqrstuv\"";
        let findings = scan(text);
        assert!(findings.iter().any(|f| f.kind == SecretKind::GoogleApiKey));
    }

    #[test]
    fn detects_a_private_key_block_header() {
        let text = "-----BEGIN RSA PRIVATE KEY-----\nMIIEow...\n-----END RSA PRIVATE KEY-----";
        let findings = scan(text);
        assert!(findings
            .iter()
            .any(|f| f.kind == SecretKind::PrivateKeyBlock));
    }

    #[test]
    fn detects_a_generic_password_assignment() {
        let text = "password = \"correct-horse-battery-staple-9\"";
        let findings = scan(text);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, SecretKind::GenericAssignment);
    }

    #[test]
    fn a_short_generic_assignment_is_not_flagged() {
        // Real, deliberate false-negative: the generic pattern requires a
        // 16+ char value specifically to avoid flagging every short,
        // obviously-not-a-secret placeholder like `password = "abc"`.
        let text = "password = \"abc\"";
        assert!(scan(text).is_empty());
    }

    #[test]
    fn a_specific_token_inside_a_generic_assignment_is_reported_once_as_the_specific_kind() {
        let text = "token = \"ghp_0123456789abcdef0123456789abcdef0123\"";
        let findings = scan(text);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, SecretKind::GitHubToken);
    }

    #[test]
    fn multiple_distinct_secrets_are_all_found_in_order() {
        let text = "AKIAABCDEFGHIJKLMNOP\nsome text\nsk_live_0123456789abcdefghijklmnop";
        let findings = scan(text);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].kind, SecretKind::AwsAccessKeyId);
        assert_eq!(findings[1].kind, SecretKind::StripeLiveKey);
        assert!(findings[0].start_byte < findings[1].start_byte);
    }

    #[test]
    fn redact_replaces_the_matched_span_and_preserves_surrounding_text() {
        let text = "before AKIAABCDEFGHIJKLMNOP after";
        let (redacted, findings) = redact(text);
        assert_eq!(findings.len(), 1);
        assert_eq!(redacted, "before [REDACTED:AWS_ACCESS_KEY_ID] after");
    }

    #[test]
    fn redact_on_clean_text_returns_it_unchanged() {
        let text = "nothing sensitive here";
        let (redacted, findings) = redact(text);
        assert_eq!(redacted, text);
        assert!(findings.is_empty());
    }

    #[test]
    fn redact_handles_multiple_secrets_in_one_pass() {
        let text = "AKIAABCDEFGHIJKLMNOP and sk_live_0123456789abcdefghijklmnop";
        let (redacted, findings) = redact(text);
        assert_eq!(findings.len(), 2);
        assert_eq!(
            redacted,
            "[REDACTED:AWS_ACCESS_KEY_ID] and [REDACTED:STRIPE_LIVE_KEY]"
        );
    }
}
