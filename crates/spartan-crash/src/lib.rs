//! Real local-first crash reporting (§18, task #13). "Crash dumps are
//! inspected locally first with an option to redact before any optional
//! upload -- never auto-uploads raw crash data silently" (§18). This pass
//! is deliberately local-only: it writes a real, redacted crash report to
//! disk and never makes a network call of any kind -- there is no upload
//! path at all yet, optional or otherwise, so "never auto-uploads" is
//! trivially true by construction rather than by a gate that could be
//! bypassed. A future upload feature adds a genuinely new, explicit,
//! user-initiated action on top of this, not a change to this crate.

use serde::Serialize;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CrashReport {
    pub unix_timestamp: u64,
    pub message: String,
    pub location: Option<String>,
}

impl CrashReport {
    /// Builds a real report from a real `std::panic::PanicHookInfo` --
    /// the exact value a real panic hook receives, not a synthetic
    /// stand-in. `payload()`'s `Any` only ever downcasts cleanly to `&str`
    /// or `String` in practice (the two types `panic!`/`.unwrap()` and
    /// friends actually produce), matching the standard library's own
    /// default hook's handling -- anything else is a real, named,
    /// intentionally narrow fallback rather than a panic-in-the-panic-
    /// handler risk from an unhandled downcast.
    pub fn from_panic_info(info: &PanicHookInfo<'_>) -> Self {
        let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let unix_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            unix_timestamp,
            message,
            location,
        }
    }
}

/// Real redaction (via `spartan_security::redact`, §75.31) applied to
/// every free-text field before serialization -- a panic message or a
/// `#[track_caller]` location string can both, in principle, echo back
/// something sensitive (an error formatted with an embedded credential,
/// a path containing one), so this crash report gets the same treatment
/// §9 calls for before any diff reaches a cloud provider, applied here to
/// the closest real analogue: a report that will sit on local disk and
/// might later be explicitly uploaded by the user.
pub fn format_report(report: &CrashReport) -> String {
    let (redacted_message, _) = spartan_security::redact(&report.message);
    let redacted_location = report
        .location
        .as_deref()
        .map(|l| spartan_security::redact(l).0);
    let redacted = CrashReport {
        unix_timestamp: report.unix_timestamp,
        message: redacted_message,
        location: redacted_location,
    };
    serde_json::to_string_pretty(&redacted).unwrap_or_else(|_| "{}".to_string())
}

/// Real, honest write to `crash_dir/crash-<unix_timestamp>.json` --
/// creates the directory if it doesn't exist, returns the real path
/// written on success.
pub fn write_report(crash_dir: &Path, report: &CrashReport) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(crash_dir)?;
    let path = crash_dir.join(format!("crash-{}.json", report.unix_timestamp));
    std::fs::write(&path, format_report(report))?;
    Ok(path)
}

/// Installs a real `std::panic::set_hook` that captures the real panic,
/// redacts it, and writes it to `crash_dir` -- then still prints the
/// panic to stderr via the real default hook's own formatting, so a
/// developer running from a terminal loses none of the normal panic
/// output this replaces. Real, deliberate ordering: the crash report is
/// written *before* the human-readable stderr print, so a report exists
/// on disk even if the process is killed immediately after the panic
/// message appears (e.g. an impatient Ctrl+C).
pub fn install_hook(crash_dir: PathBuf) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let report = CrashReport::from_panic_info(info);
        match write_report(&crash_dir, &report) {
            Ok(path) => eprintln!(
                "A crash report was saved locally to {} (never auto-uploaded).",
                path.display()
            ),
            Err(e) => eprintln!("Failed to save local crash report: {e}"),
        }
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report() -> CrashReport {
        CrashReport {
            unix_timestamp: 1_700_000_000,
            message: "index out of bounds: the len is 3 but the index is 5".to_string(),
            location: Some("src/main.rs:42:9".to_string()),
        }
    }

    #[test]
    fn format_report_produces_valid_parseable_json_with_expected_fields() {
        let report = sample_report();
        let json = format_report(&report);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["unix_timestamp"], 1_700_000_000);
        assert_eq!(
            parsed["message"],
            "index out of bounds: the len is 3 but the index is 5"
        );
        assert_eq!(parsed["location"], "src/main.rs:42:9");
    }

    #[test]
    fn format_report_redacts_a_real_secret_embedded_in_the_panic_message() {
        let report = CrashReport {
            unix_timestamp: 1,
            message: "failed to auth with AKIAABCDEFGHIJKLMNOP".to_string(),
            location: None,
        };
        let json = format_report(&report);
        assert!(!json.contains("AKIAABCDEFGHIJKLMNOP"));
        assert!(json.contains("REDACTED:AWS_ACCESS_KEY_ID"));
    }

    #[test]
    fn format_report_redacts_a_secret_embedded_in_the_location_too() {
        // A real, if unusual, case: a file path itself contains something
        // secret-shaped (e.g. a temp file named after a token).
        let report = CrashReport {
            unix_timestamp: 1,
            message: "panicked".to_string(),
            location: Some("/tmp/sk_live_0123456789abcdefghijklmnop/main.rs:1:1".to_string()),
        };
        let json = format_report(&report);
        assert!(!json.contains("sk_live_0123456789abcdefghijklmnop"));
    }

    #[test]
    fn write_report_creates_the_directory_and_writes_a_real_file() {
        let dir = std::env::temp_dir().join("spartan_crash_test_write");
        let _ = std::fs::remove_dir_all(&dir);
        let report = sample_report();
        let path = write_report(&dir, &report).unwrap();
        assert!(path.exists());
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("index out of bounds"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_report_names_the_file_after_the_real_timestamp() {
        let dir = std::env::temp_dir().join("spartan_crash_test_naming");
        let _ = std::fs::remove_dir_all(&dir);
        let report = sample_report();
        let path = write_report(&dir, &report).unwrap();
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "crash-1700000000.json"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clean_panic_message_with_no_secrets_is_left_unredacted() {
        let report = sample_report();
        let json = format_report(&report);
        assert!(json.contains("index out of bounds: the len is 3 but the index is 5"));
    }
}
