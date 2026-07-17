//! Real, in-house LSP client speaking real JSON-RPC 2.0 over stdio, copied
//! (not imported) from `crates/spartan-editor-core/src/lsp.rs` -- that
//! crate's own doc comment there already explains it was itself promoted
//! verbatim from `spikes/lsp-spike`, proven against two independent real
//! servers (`rust-analyzer`, `pyright-langserver`) by that spike's own
//! tests. This is a second, deliberate promotion, not an extraction: the
//! wgpu reference shell (`spartan-editor-core`) is left completely
//! untouched rather than refactored to share this code, since that crate is
//! large, already heavily tested, and load-bearing for this whole project's
//! own "reference implementation" guarantee -- the same "promote via
//! duplication, don't risk the reference shell" precedent this project
//! already applied when `render-spike`/`lsp-spike` were first promoted into
//! `spartan-editor-core` itself. The real cost, named honestly: a bug fixed
//! here (or there) doesn't automatically fix the other copy.

use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

/// Converts a filesystem path to a `file://` URI per RFC 8089. See the
/// original `spartan-editor-core::lsp::path_to_file_uri` for the full
/// Windows-drive-letter rationale this copies verbatim.
pub fn path_to_file_uri(path: &std::path::Path) -> String {
    let mut normalized = path.display().to_string().replace('\\', "/");
    if let Some(first) = normalized.chars().next() {
        if first.is_ascii_alphabetic() && normalized.as_bytes().get(1) == Some(&b':') {
            normalized = format!("{}{}", first.to_ascii_lowercase(), &normalized[1..]);
        }
    }
    if normalized.starts_with('/') {
        format!("file://{normalized}")
    } else {
        format!("file:///{normalized}")
    }
}

/// Decodes RFC 3986 percent-escapes (`%XX`) in a URI path, assuming UTF-8.
/// A real, general fix -- not the narrower colon-only special case the
/// original `spartan-editor-core::lsp::same_file_uri` this was copied from
/// still carries (see that function's own history at §75.6/§75.45, which
/// only ever needed to handle a Windows-drive-letter colon). **A real bug
/// this crate's own live integration test found**: this crate's temp test
/// fixtures build a directory name from `{:?}` on a `std::thread::ThreadId`
/// (e.g. `ThreadId(1)`, matching this whole repo's own established
/// fixture-naming convention -- see `crates/spartan-devserver::
/// static_serve::tests::make_web_root`), and pyright genuinely
/// percent-encodes the literal parentheses in its own echoed URI
/// (`ThreadId%281%29`) -- the narrow colon-only decoder never matched that
/// against the locally-built, unescaped URI, so `wait_real_diagnostics`'s
/// own predicate never fired and the whole session timed out at 90s. Not
/// just a test-fixture quirk: any real project path containing parentheses,
/// spaces, or other URI-reserved characters would hit the identical failure
/// in production. Fixed at the root with a real, general percent-decoder
/// instead of special-casing one more character.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Compares two `file://` URIs for referring to the same path, tolerant of
/// any percent-escaping difference between real servers -- see
/// `percent_decode`'s own doc comment for the real bug this generalizes
/// away. A Windows drive letter's case is deliberately *not* normalized
/// here (unlike percent-escaping): `path_to_file_uri` already lowercases it
/// once at construction time for every URI this crate builds itself, and
/// real servers (rust-analyzer, VS Code's own client) already echo it
/// lowercase too -- so both sides are already consistent by construction,
/// the same real division of responsibility the original `spartan-editor-
/// core::lsp` copy this was adapted from already established.
pub(crate) fn same_file_uri(a: &str, b: &str) -> bool {
    percent_decode(a) == percent_decode(b)
}

fn read_message<R: BufRead>(reader: &mut R) -> std::io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(v) = trimmed.strip_prefix("Content-Length: ") {
            content_length = v.trim().parse().ok();
        }
    }
    let len = content_length.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "missing Content-Length header",
        )
    })?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    let value: Value = serde_json::from_slice(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Some(value))
}

fn write_message(stdin: &mut ChildStdin, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len())?;
    stdin.write_all(&body)?;
    stdin.flush()
}

/// A minimal in-house LSP client: real JSON-RPC 2.0 framing over a child
/// process's stdio, no third-party LSP crate.
pub struct LspClient {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    buffered: VecDeque<Value>,
    next_id: i64,
}

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// See `spartan-editor-core::lsp::INITIALIZE_TIMEOUT` (§75.45): a real,
/// live `kotlin-language-server` (a JVM process) needs real time past
/// `DEFAULT_TIMEOUT` to answer `initialize`.
pub const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(45);
/// rust-analyzer needs real time to load sysroot/std metadata and index
/// even a one-file crate; not tunable away.
pub const INDEXING_TIMEOUT: Duration = Duration::from_secs(90);

impl LspClient {
    pub fn spawn(server_path: &str) -> std::io::Result<Self> {
        Self::spawn_with_args(server_path, &[])
    }

    /// Same as `spawn`, but for servers that need an argv flag to run in
    /// stdio mode (e.g. `pyright-langserver --stdio`).
    pub fn spawn_with_args(server_path: &str, args: &[&str]) -> std::io::Result<Self> {
        let mut child = Command::new(server_path)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_message(&mut reader) {
                    Ok(Some(msg)) => {
                        if tx.send(msg).is_err() {
                            return;
                        }
                    }
                    Ok(None) | Err(_) => return,
                }
            }
        });
        // Servers log progress/diagnostics info to stderr; drain it in the
        // background so it never blocks on a full pipe buffer.
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut buf = String::new();
            let _ = reader.read_to_string(&mut buf);
        });

        Ok(Self {
            child,
            stdin,
            rx,
            buffered: VecDeque::new(),
            next_id: 0,
        })
    }

    fn next_message(&mut self, deadline: Instant) -> Option<Value> {
        if let Some(v) = self.buffered.pop_front() {
            return Some(v);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match self.rx.recv_timeout(remaining) {
            Ok(v) => Some(v),
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => None,
        }
    }

    fn wait_for<F: Fn(&Value) -> bool>(&mut self, pred: F, timeout: Duration) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        let mut skipped = VecDeque::new();
        let result = loop {
            match self.next_message(deadline) {
                Some(msg) => {
                    if pred(&msg) {
                        break Some(msg);
                    } else {
                        skipped.push_back(msg);
                    }
                }
                None => break None,
            }
        };
        for m in skipped.into_iter().rev() {
            self.buffered.push_front(m);
        }
        result
    }

    pub fn request(
        &mut self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Option<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let mut msg = json!({"jsonrpc": "2.0", "id": id, "method": method});
        if let Some(p) = params {
            msg["params"] = p;
        }
        write_message(&mut self.stdin, &msg).ok()?;
        self.wait_for(
            |m| m.get("id").and_then(Value::as_i64) == Some(id) && m.get("method").is_none(),
            timeout,
        )
    }

    pub fn notify(&mut self, method: &str, params: Value) -> std::io::Result<()> {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        write_message(&mut self.stdin, &msg)
    }

    pub fn wait_notification<F: Fn(&Value) -> bool>(
        &mut self,
        method: &str,
        pred: F,
        timeout: Duration,
    ) -> Option<Value> {
        self.wait_for(
            |m| {
                m.get("method").and_then(Value::as_str) == Some(method)
                    && m.get("id").is_none()
                    && pred(m.get("params").unwrap_or(&Value::Null))
            },
            timeout,
        )
    }

    /// initialize -> initialized -> didOpen, the standard LSP session start.
    /// `language_id` is a real LSP `languageId` (e.g. `"rust"`, `"python"`),
    /// not hardcoded to `"rust"` the way the single-language spike/wgpu-shell
    /// copy is -- this crate serves every Tier 1 language, not just one.
    pub fn open_project(
        &mut self,
        root_uri: &str,
        file_uri: &str,
        language_id: &str,
        content: &str,
    ) -> Option<Value> {
        let init_resp = self.request(
            "initialize",
            Some(json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "workspaceFolders": [{"uri": root_uri, "name": "project"}],
                "capabilities": {
                    "textDocument": {
                        "hover": {"contentFormat": ["plaintext", "markdown"]},
                        "completion": {"completionItem": {"snippetSupport": false}},
                        "definition": {"linkSupport": false},
                        "publishDiagnostics": {},
                    }
                },
            })),
            INITIALIZE_TIMEOUT,
        )?;
        init_resp.get("result")?;
        self.notify("initialized", json!({})).ok()?;
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {"uri": file_uri, "languageId": language_id, "version": 1, "text": content}
            }),
        )
        .ok()?;
        Some(init_resp)
    }

    /// Waits for the first `publishDiagnostics` notification that actually
    /// carries diagnostics. Can never observe diagnostics *clearing back to
    /// empty* by design -- callers driving an ongoing session use
    /// `wait_notification` directly instead, as `session.rs` does.
    pub fn wait_real_diagnostics(
        &mut self,
        file_uri: &str,
        timeout: Duration,
    ) -> Option<Vec<Value>> {
        let msg = self.wait_notification(
            "textDocument/publishDiagnostics",
            |params| {
                params
                    .get("uri")
                    .and_then(Value::as_str)
                    .is_some_and(|u| same_file_uri(u, file_uri))
                    && params
                        .get("diagnostics")
                        .and_then(Value::as_array)
                        .map(|a| !a.is_empty())
                        .unwrap_or(false)
            },
            timeout,
        )?;
        msg["params"]["diagnostics"].as_array().cloned()
    }

    /// Real `textDocument/hover`, ported verbatim from `spartan-editor-
    /// core::lsp::LspClient::hover` (§75.6) -- named as a real, unstarted
    /// gap in this crate's own `lsp_integration.rs` doc comment ("no
    /// hover/completion IPC methods exist yet") until this pass.
    pub fn hover(&mut self, file_uri: &str, line: i64, character: i64) -> Option<Value> {
        self.request(
            "textDocument/hover",
            Some(json!({
                "textDocument": {"uri": file_uri},
                "position": {"line": line, "character": character},
            })),
            DEFAULT_TIMEOUT,
        )
    }

    /// Real `textDocument/completion`, ported verbatim from the same
    /// reference method -- real and tested here, but (unlike `hover`)
    /// has no real caller anywhere in this workspace yet: a completion
    /// *dropdown* UI is a real, separate, larger increment than a hover
    /// tooltip, not attempted this pass.
    pub fn completion(&mut self, file_uri: &str, line: i64, character: i64) -> Option<Value> {
        self.request(
            "textDocument/completion",
            Some(json!({
                "textDocument": {"uri": file_uri},
                "position": {"line": line, "character": character},
            })),
            DEFAULT_TIMEOUT,
        )
    }

    /// Real `textDocument/definition` -- the third real query method
    /// following `hover`/`completion`'s exact shape. A real LSP `definition`
    /// response is `Location | Location[] | LocationLink[] | null`; that
    /// shape is passed through unparsed here, same division of
    /// responsibility `hover`/`completion` already establish (this crate's
    /// job is the wire request, not response normalization).
    pub fn definition(&mut self, file_uri: &str, line: i64, character: i64) -> Option<Value> {
        self.request(
            "textDocument/definition",
            Some(json!({
                "textDocument": {"uri": file_uri},
                "position": {"line": line, "character": character},
            })),
            DEFAULT_TIMEOUT,
        )
    }

    pub fn did_change_full(
        &mut self,
        file_uri: &str,
        version: i64,
        new_text: &str,
    ) -> std::io::Result<()> {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": {"uri": file_uri, "version": version},
                "contentChanges": [{"text": new_text}],
            }),
        )
    }

    /// Graceful shutdown: `shutdown` request, then `exit` notification, with
    /// a bounded kill-fallback that never trusts the subprocess's own
    /// shutdown. Blocks for up to ~7s worst case.
    pub fn shutdown(mut self) {
        let _ = self.request("shutdown", None, Duration::from_secs(5));
        let _ = self.notify("exit", Value::Null);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match self.child.try_wait() {
                Ok(Some(_status)) => return,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(20));
                }
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact real bug this crate's own live integration test found:
    /// a percent-encoded parenthesis in a real server's echoed URI must
    /// still compare equal to the unescaped original.
    #[test]
    fn same_file_uri_matches_a_real_percent_encoded_parenthesis() {
        assert!(same_file_uri(
            "file:///tmp/spartan-lsp-debug-pyright-14694-ThreadId(1)/main.py",
            "file:///tmp/spartan-lsp-debug-pyright-14694-ThreadId%281%29/main.py",
        ));
    }

    #[test]
    fn path_to_file_uri_lowercases_a_windows_drive_letter_at_construction_time() {
        // `same_file_uri` itself does not case-normalize (see its own doc
        // comment) -- both sides are already consistent because
        // `path_to_file_uri` lowercases once here, and real servers already
        // echo the drive letter lowercase too.
        let uri = path_to_file_uri(&std::path::PathBuf::from("C:\\Users\\x\\main.rs"));
        assert_eq!(uri, "file:///c:/Users/x/main.rs");
    }

    #[test]
    fn same_file_uri_rejects_genuinely_different_paths() {
        assert!(!same_file_uri(
            "file:///tmp/a/main.py",
            "file:///tmp/b/main.py",
        ));
    }

    #[test]
    fn percent_decode_leaves_a_trailing_lone_percent_untouched() {
        // A malformed/truncated escape at the very end of the string must
        // not panic or read out of bounds.
        assert_eq!(percent_decode("abc%"), "abc%");
        assert_eq!(percent_decode("abc%2"), "abc%2");
    }
}
