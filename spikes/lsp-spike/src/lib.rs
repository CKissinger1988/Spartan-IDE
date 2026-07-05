//! Tier 0 Spike 0.2 (LSP half) — an in-house LSP client, speaking real
//! JSON-RPC 2.0 over stdio to a real `rust-analyzer` subprocess, plus the
//! debounced-didChange dispatcher §2.3 specifies. See docs/architecture-spec.md
//! §2.3, §20, §39.2, §47.5/§47.6.

use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

/// Converts a filesystem path to a `file://` URI per RFC 8089. Plain
/// `format!("file://{}", path.display())` is correct on Unix (an absolute
/// path already starts with `/`, giving the right triple slash) but wrong on
/// Windows in two independent ways, both found by running these tests for
/// real on Windows (every wait on real diagnostics silently timed out
/// instead of erroring loudly):
///
/// 1. `path.display()` yields backslashes and a bare drive letter
///    (`C:\Users\x`), producing `file://C:\Users\x` — missing the leading
///    `/` before the drive letter and using the wrong slash direction.
/// 2. Even once corrected to `file:///C:/Users/x`, rust-analyzer echoes the
///    drive letter lowercased in every URI it sends back (`file:///c:/...`).
///    Callers that correlate a `publishDiagnostics`/response URI back to the
///    file they opened via exact string equality — as this client's
///    `wait_notification` does — never match an uppercase-drive-letter
///    request, independent of whether real diagnostics ever arrive. VS
///    Code's own LSP client lowercases the drive letter for the same
///    reason; matching that convention here avoids the mismatch instead of
///    just hiding it.
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

/// Compares two `file://` URIs for referring to the same path, tolerant of
/// the one escaping difference actually observed between real servers:
/// pyright percent-encodes a drive-letter colon (`%3A`) where rust-analyzer
/// leaves it literal. Not a general URI-equivalence algorithm — just enough
/// to stop this client's own request/response matching from being silently
/// wrong about a colon it put there itself.
fn same_file_uri(a: &str, b: &str) -> bool {
    fn decode_colon(s: &str) -> String {
        s.replace("%3A", ":").replace("%3a", ":")
    }
    decode_colon(a) == decode_colon(b)
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
/// process's stdio, no third-party LSP crate — the same "own request queue,
/// full JSON-RPC/stdio transport" approach §2.3 specifies for the product.
pub struct LspClient {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    buffered: VecDeque<Value>,
    next_id: i64,
}

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// rust-analyzer needs real time to load the sysroot/std metadata and index
/// even a one-file crate; this is not tunable away, so tests that wait on
/// the first diagnostics pass use this instead of DEFAULT_TIMEOUT.
pub const INDEXING_TIMEOUT: Duration = Duration::from_secs(90);

impl LspClient {
    pub fn spawn(server_path: &str) -> std::io::Result<Self> {
        Self::spawn_with_args(server_path, &[])
    }

    /// Same as `spawn`, but for servers that need an argv flag to run in
    /// stdio mode (e.g. `pyright-langserver --stdio` — added for the
    /// cross-language check in §47.7 rather than for `rust-analyzer`, which
    /// needs no flags, so `spawn` is kept as the zero-arg convenience form).
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
        // rust-analyzer logs progress/diagnostics info to stderr; drain it in
        // the background so it never blocks on a full pipe buffer during a
        // long indexing pass.
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

    /// Waits for a message matching `pred`, buffering (in arrival order)
    /// anything skipped so it's still visible to a later wait — necessary
    /// because notifications (diagnostics, progress) interleave with
    /// request/response pairs on the same stream.
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

    /// Sends a request. `params: None` omits the `params` key entirely
    /// (some servers, rust-analyzer's `shutdown` included, reject an empty
    /// object `{}` where the spec calls for no params at all — found by
    /// manual protocol probe before writing this client; see §47.6).
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

    /// Waits for a notification (no `id`) with the given method name,
    /// additionally filtered by `pred` over its `params` — e.g. "does this
    /// publishDiagnostics report anything yet," since the server sends an
    /// initial empty diagnostics set before real analysis completes.
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
    pub fn open_project(&mut self, root_uri: &str, file_uri: &str, content: &str) -> Option<Value> {
        let init_resp = self.request(
            "initialize",
            Some(json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "workspaceFolders": [{"uri": root_uri, "name": "fixture-crate"}],
                "capabilities": {
                    "textDocument": {
                        "hover": {"contentFormat": ["plaintext", "markdown"]},
                        "completion": {"completionItem": {"snippetSupport": false}},
                        "publishDiagnostics": {},
                    }
                },
            })),
            DEFAULT_TIMEOUT,
        )?;
        init_resp.get("result")?;
        self.notify("initialized", json!({})).ok()?;
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {"uri": file_uri, "languageId": "rust", "version": 1, "text": content}
            }),
        )
        .ok()?;
        Some(init_resp)
    }

    /// Waits for the first `publishDiagnostics` notification that actually
    /// carries diagnostics (rust-analyzer publishes an empty set immediately
    /// on open, then real ones once indexing/analysis finishes).
    ///
    /// The `uri` on that notification is compared via `same_file_uri`, not
    /// raw string equality: real servers don't agree on how to spell the
    /// same Windows path. rust-analyzer echoes `file:///c:/...` (colon
    /// unencoded); pyright echoes `file:///c%3A/...` (colon percent-encoded,
    /// per RFC 3986). Both were observed by actually running this client
    /// against both servers — a naive `==` silently never matches on
    /// Windows against whichever server's convention wasn't guessed right.
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

    /// Graceful shutdown: `shutdown` request (no params — see the comment on
    /// `request`), then `exit` notification. Unlike the `dap-spike` finding
    /// against `lldb-dap`, rust-analyzer was observed to exit cleanly here;
    /// the bounded kill-fallback is still kept, on the same "never trust a
    /// subprocess's own shutdown" principle from §4.5, precautionary rather
    /// than compensating for an observed bug this time.
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

/// Coalesces a burst of edits into a single dispatch after `debounce` of
/// idle time — the client-side half of §2.3's "own request queue with
/// debounced didChange (150ms idle default)" claim. This does not need a
/// real language server to test: it's a pure scheduling policy over wall-clock
/// time, and is tested as such (§47.6) rather than only asserted in prose.
pub struct DidChangeDebouncer {
    debounce: Duration,
    last_edit: Option<Instant>,
    pending: bool,
    dispatched_count: u32,
}

impl DidChangeDebouncer {
    pub fn new(debounce: Duration) -> Self {
        Self {
            debounce,
            last_edit: None,
            pending: false,
            dispatched_count: 0,
        }
    }

    /// Call once per keystroke/edit event.
    pub fn on_edit(&mut self) {
        self.last_edit = Some(Instant::now());
        self.pending = true;
    }

    /// Call on a poll tick (or right before considering a dispatch). Returns
    /// true exactly once per idle period once enough time has elapsed since
    /// the last edit with no newer edit arriving in between.
    pub fn should_dispatch_now(&mut self) -> bool {
        let Some(last) = self.last_edit else {
            return false;
        };
        if !self.pending {
            return false;
        }
        if last.elapsed() >= self.debounce {
            self.pending = false;
            self.dispatched_count += 1;
            true
        } else {
            false
        }
    }

    pub fn dispatched_count(&self) -> u32 {
        self.dispatched_count
    }
}
