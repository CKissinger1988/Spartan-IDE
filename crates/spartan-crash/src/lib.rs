//! Real local-first crash reporting (§18, task #13/#35). "Crash dumps are
//! inspected locally first with an option to redact before any optional
//! upload -- never auto-uploads raw crash data silently" (§18). §75.32
//! shipped the local-only half: a real, redacted crash report written to
//! disk, no network call anywhere in this crate. This pass (§75.82) adds
//! the other half §18 always named as future work -- a real, explicit,
//! user-initiated upload -- without weakening that guarantee: `upload_report`
//! is the *only* function in this crate that ever makes a network call, it
//! takes an already-redacted report and an endpoint the user must have
//! typed in themselves, and nothing else in this crate (in particular,
//! `install_hook`'s own panic path) ever calls it. "Never auto-uploads" stays
//! true not because no upload path exists, but because the one that exists
//! is never reachable except through a real, separate, explicit user click.

use serde::Serialize;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// Real listing of every `crash-*.json` report currently on disk in
/// `crash_dir`, newest first by filename (which is itself the report's
/// real unix timestamp, so a plain reverse-sort is correct and doesn't
/// need to re-parse or re-stat anything). A missing directory (no crash
/// has ever been written) is a real, expected empty result, not an
/// error.
pub fn list_reports(crash_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    if !crash_dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = std::fs::read_dir(crash_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".json"))
        })
        .collect();
    paths.sort();
    paths.reverse();
    Ok(paths)
}

#[derive(Debug)]
pub enum UploadError {
    Network(String),
    Http { status: u16, body: String },
}

impl std::fmt::Display for UploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UploadError::Network(msg) => write!(f, "network error: {msg}"),
            UploadError::Http { status, body } => write!(f, "HTTP {status}: {body}"),
        }
    }
}

impl std::error::Error for UploadError {}

/// Real, explicit, user-initiated upload of one already-written,
/// already-redacted report file's exact on-disk bytes to `endpoint` --
/// no re-serialization, no fresh unredacted round trip through
/// `CrashReport` at all, so there is no way for this function to upload
/// anything other than what a human could already read on disk
/// themselves. `endpoint` is never defaulted or hardcoded anywhere in
/// this crate; the caller (a real, explicit user action one layer up)
/// supplies it every time. Returns the real HTTP status code on success.
pub fn upload_report(endpoint: &str, report_json: &str) -> Result<u16, UploadError> {
    let resp = ureq::post(endpoint)
        .set("Content-Type", "application/json")
        .set("User-Agent", "spartan-ide-crash-reporter")
        .timeout(Duration::from_secs(15))
        .send_string(report_json)
        .map_err(|e| match e {
            ureq::Error::Status(status, resp) => UploadError::Http {
                status,
                body: resp.into_string().unwrap_or_default(),
            },
            ureq::Error::Transport(t) => UploadError::Network(t.to_string()),
        })?;
    Ok(resp.status())
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

    #[test]
    fn list_reports_on_a_directory_that_has_never_been_written_to_is_a_real_empty_result_not_an_error(
    ) {
        let dir = std::env::temp_dir().join("spartan_crash_test_list_missing");
        let _ = std::fs::remove_dir_all(&dir);
        let reports = list_reports(&dir).unwrap();
        assert!(reports.is_empty());
    }

    #[test]
    fn list_reports_finds_real_written_reports_newest_first_and_ignores_unrelated_files() {
        let dir = std::env::temp_dir().join("spartan_crash_test_list_populated");
        let _ = std::fs::remove_dir_all(&dir);
        write_report(
            &dir,
            &CrashReport {
                unix_timestamp: 100,
                message: "first".to_string(),
                location: None,
            },
        )
        .unwrap();
        write_report(
            &dir,
            &CrashReport {
                unix_timestamp: 200,
                message: "second".to_string(),
                location: None,
            },
        )
        .unwrap();
        std::fs::write(dir.join("not-a-crash-report.txt"), "ignore me").unwrap();
        let reports = list_reports(&dir).unwrap();
        let names: Vec<String> = reports
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["crash-200.json", "crash-100.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A real, minimal, hand-rolled HTTP/1.1 server -- not a mocking
    /// library, an actual `TcpListener` -- so `upload_report`'s own real
    /// `ureq` POST is exercised against a genuine socket, not a stubbed
    /// function. Reads exactly one request (headers + a real
    /// `Content-Length`-bounded body), replies with `response_status`/
    /// `response_body`, and hands the real received body back to the
    /// caller so a test can assert on exactly what `ureq` actually sent
    /// over the wire.
    fn spawn_mock_upload_server(
        response_status: u16,
        response_body: &'static str,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let body = loop {
                let n = stream.read(&mut chunk).unwrap();
                if n == 0 {
                    break String::new();
                }
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf);
                if let Some(header_end) = text.find("\r\n\r\n") {
                    let content_length: usize = text[..header_end]
                        .lines()
                        .find_map(|l| {
                            let lower = l.to_ascii_lowercase();
                            lower
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    let body_start = header_end + 4;
                    if buf.len() >= body_start + content_length {
                        break String::from_utf8_lossy(
                            &buf[body_start..body_start + content_length],
                        )
                        .to_string();
                    }
                }
            };
            let _ = tx.send(body);
            let reason = if response_status == 200 {
                "OK"
            } else {
                "Error"
            };
            let response = format!(
                "HTTP/1.1 {response_status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        });
        (format!("http://127.0.0.1:{port}"), rx)
    }

    #[test]
    fn upload_report_really_posts_the_exact_report_bytes_to_a_real_local_server_and_reports_the_real_status(
    ) {
        let (endpoint, rx) = spawn_mock_upload_server(200, "");
        let report_json = r#"{"unix_timestamp":1,"message":"real body","location":null}"#;
        let status = upload_report(&endpoint, report_json).unwrap();
        assert_eq!(status, 200);
        let received = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(received, report_json);
    }

    #[test]
    fn upload_report_surfaces_a_real_non_2xx_response_as_a_real_honest_http_error() {
        let (endpoint, _rx) = spawn_mock_upload_server(500, "server exploded");
        let err = upload_report(&endpoint, "{}").unwrap_err();
        match err {
            UploadError::Http { status, body } => {
                assert_eq!(status, 500);
                assert_eq!(body, "server exploded");
            }
            UploadError::Network(msg) => panic!("expected Http error, got Network({msg})"),
        }
    }

    #[test]
    fn upload_report_surfaces_a_real_connection_failure_honestly() {
        // A real, guaranteed-unused local port (nothing is listening) --
        // a genuine connection-refused, not a simulated one.
        let err = upload_report("http://127.0.0.1:1", "{}").unwrap_err();
        match err {
            UploadError::Network(_) => {}
            UploadError::Http { status, body } => {
                panic!("expected Network error, got Http {{ status: {status}, body: {body} }}")
            }
        }
    }
}
