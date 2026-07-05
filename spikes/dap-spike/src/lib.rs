//! Tier 0 Spike 0.2 (DAP half) — an in-house Debug Adapter Protocol client,
//! speaking DAP over stdio to a real `lldb-dap` subprocess, and a rope-anchored
//! breakpoint model proving persistence through an edit that shifts line numbers.
//! See docs/architecture-spec.md §2.3, §32, §39.2.

use ropey::Rope;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

/// Reads one Content-Length-framed DAP/LSP-style message from a reader.
/// Returns `Ok(None)` on clean EOF.
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

/// A minimal in-house DAP client. Spawns a debug adapter subprocess and
/// exchanges Content-Length-framed JSON messages over its stdio, exactly as
/// Spartan's real LSP/DAP clients will (§2.3) — no third-party DAP library.
pub struct DapClient {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    /// Messages read off the wire but not yet claimed by a waiter, kept in
    /// arrival order so a later `wait_for` can still find an event that
    /// arrived before anyone was looking for it.
    buffered: VecDeque<Value>,
    next_seq: i64,
}

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

impl DapClient {
    /// Spawns the given debug adapter binary (e.g. `lldb-dap-18`) and starts
    /// a background reader thread that deserializes messages off its stdout.
    pub fn spawn(adapter_path: &str) -> std::io::Result<Self> {
        let mut child = Command::new(adapter_path)
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
                    Ok(None) => return, // clean EOF
                    Err(_) => return,   // malformed stream; stop reading
                }
            }
        });
        // Drain stderr in the background so a chatty/crashing adapter never
        // blocks on a full pipe buffer (see the crash-on-shutdown finding
        // documented in docs/architecture-spec.md §47.5 — this adapter is
        // known to dump a crash backtrace to stderr during its own exit path).
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
            next_seq: 0,
        })
    }

    fn send_request(&mut self, command: &str, arguments: Value) -> std::io::Result<i64> {
        self.next_seq += 1;
        let seq = self.next_seq;
        let msg = json!({
            "seq": seq,
            "type": "request",
            "command": command,
            "arguments": arguments,
        });
        write_message(&mut self.stdin, &msg)?;
        Ok(seq)
    }

    /// Pulls the next raw message, checking the buffer before the channel.
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

    /// Waits for a message matching `pred`, buffering (in order) anything
    /// that doesn't match so a subsequent wait can still see it. This is
    /// necessary because DAP interleaves request responses with async events
    /// on the same stream.
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
        // put back anything we skipped, in original order, ahead of whatever
        // is already buffered from a previous partial drain
        for m in skipped.into_iter().rev() {
            self.buffered.push_front(m);
        }
        result
    }

    /// Sends a request and waits for its matching response. Returns the full
    /// response envelope (caller can check `success` / read `body`).
    pub fn request(&mut self, command: &str, arguments: Value, timeout: Duration) -> Option<Value> {
        let seq = self.send_request(command, arguments).ok()?;
        self.wait_for(
            |m| {
                m.get("type").and_then(Value::as_str) == Some("response")
                    && m.get("request_seq").and_then(Value::as_i64) == Some(seq)
            },
            timeout,
        )
    }

    /// Waits for a named event (e.g. "stopped", "initialized", "exited").
    pub fn wait_event(&mut self, name: &str, timeout: Duration) -> Option<Value> {
        self.wait_for(
            |m| {
                m.get("type").and_then(Value::as_str) == Some("event")
                    && m.get("event").and_then(Value::as_str) == Some(name)
            },
            timeout,
        )
    }

    /// The standard DAP launch sequence for an adapter that supports
    /// `configurationDone` (confirmed against lldb-dap by manual protocol
    /// probe before writing this client — see §47.5): initialize, launch,
    /// wait for `initialized`, set breakpoints, configurationDone, then wait
    /// for the program to actually stop.
    pub fn launch_and_break(
        &mut self,
        program: &str,
        cwd: &str,
        source_path: &str,
        break_lines: &[i64],
    ) -> Option<Value> {
        let init_resp = self.request(
            "initialize",
            json!({
                "clientID": "spartan-dap-spike",
                "adapterID": "lldb-dap",
                "pathFormat": "path",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "supportsVariableType": true,
            }),
            DEFAULT_TIMEOUT,
        )?;
        if init_resp.get("success").and_then(Value::as_bool) != Some(true) {
            return None;
        }

        // Send `launch` but do not block on its response yet: `lldb-dap`
        // responds immediately, but `debugpy` (confirmed by cross-language
        // testing, see §47.7) defers the `launch` response until *after*
        // `configurationDone` — the debuggee doesn't actually start running
        // until then. Blocking here for the response before sending
        // `setBreakpoints`/`configurationDone` would deadlock against any
        // adapter using that (spec-legal) deferred pattern: the client waits
        // for a response the server won't send until the client sends the
        // next request. Fire the request, keep its seq, and only collect the
        // response once it's safe to have not gotten it yet either way.
        let launch_seq = self
            .send_request(
                "launch",
                json!({
                    "program": program,
                    "args": Vec::<String>::new(),
                    "cwd": cwd,
                    "stopOnEntry": false,
                }),
            )
            .ok()?;

        self.wait_event("initialized", DEFAULT_TIMEOUT)?;

        let bp_lines: Vec<Value> = break_lines.iter().map(|l| json!({"line": l})).collect();
        let set_bp_resp = self.request(
            "setBreakpoints",
            json!({
                "source": {"path": source_path},
                "breakpoints": bp_lines,
            }),
            DEFAULT_TIMEOUT,
        )?;

        self.request("configurationDone", json!({}), DEFAULT_TIMEOUT)?;

        // Now collect the launch response — already buffered and instantly
        // available for an adapter like lldb-dap that answered right away,
        // or arriving now for one like debugpy that was waiting for exactly
        // this point in the sequence.
        let launch_resp = self.wait_for(
            |m| {
                m.get("type").and_then(Value::as_str) == Some("response")
                    && m.get("request_seq").and_then(Value::as_i64) == Some(launch_seq)
            },
            DEFAULT_TIMEOUT,
        )?;
        if launch_resp.get("success").and_then(Value::as_bool) != Some(true) {
            return None;
        }

        // `stopped` may already have been queued ahead of configurationDone's
        // own response by the reader thread; wait_for's buffering handles that.
        let stopped = self.wait_event("stopped", DEFAULT_TIMEOUT)?;
        Some(json!({"setBreakpoints": set_bp_resp, "stopped": stopped}))
    }

    pub fn stack_trace(&mut self, thread_id: i64) -> Option<Value> {
        self.request(
            "stackTrace",
            json!({"threadId": thread_id}),
            DEFAULT_TIMEOUT,
        )
    }

    pub fn scopes(&mut self, frame_id: i64) -> Option<Value> {
        self.request("scopes", json!({"frameId": frame_id}), DEFAULT_TIMEOUT)
    }

    pub fn variables(&mut self, variables_reference: i64) -> Option<Value> {
        self.request(
            "variables",
            json!({"variablesReference": variables_reference}),
            DEFAULT_TIMEOUT,
        )
    }

    pub fn continue_(&mut self, thread_id: i64) -> Option<Value> {
        self.request("continue", json!({"threadId": thread_id}), DEFAULT_TIMEOUT)
    }

    /// Shuts down the adapter. This build of `lldb-dap` (LLVM 18.1, Ubuntu
    /// 24.04 package) reliably SIGABRTs (`free(): invalid pointer`) somewhere
    /// in its own exit path immediately after replying to `disconnect` — with
    /// or without a live debuggee, confirmed by manual protocol probe (§47.5
    /// documents the exact reproduction). Treating that as a Spartan bug
    /// would be wrong; treating it as trustworthy would be worse. So: ask
    /// nicely, read the response if it comes, then unconditionally reap the
    /// child on a short leash rather than trusting it to exit cleanly. This
    /// directly matches §4.5's sandboxed-subprocess discipline (never trust a
    /// subprocess's own shutdown) and should be carried into the real client.
    pub fn shutdown(mut self) {
        let _ = self.request("disconnect", json!({}), Duration::from_secs(2));
        let deadline = Instant::now() + Duration::from_millis(500);
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

/// Compiles `source` (a full Rust source file body) to a debug binary at
/// `bin_path`, writing the source to `src_path` first. Returns once rustc
/// exits; panics with rustc's stderr on failure (this is spike-harness code,
/// not production — a real language-profile invocation goes through §20.2's
/// sandboxed Task model instead).
pub fn compile_fixture(source: &str, src_path: &std::path::Path, bin_path: &std::path::Path) {
    std::fs::write(src_path, source).expect("write fixture source");
    let output = Command::new("rustc")
        .arg("-g")
        .arg("-o")
        .arg(bin_path)
        .arg(src_path)
        .output()
        .expect("run rustc");
    assert!(
        output.status.success(),
        "rustc failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Models a breakpoint anchored to a persistent rope byte-position rather
/// than a raw line number (§2.3's stated design: "breakpoint state lives in
/// the rope's metadata layer ... breakpoints attach to persistent rope
/// positions, not raw line numbers"). Given an edit that inserts `inserted_len`
/// bytes at `insert_at_byte`, a marker positioned at-or-after the insertion
/// point shifts forward by the inserted length; a marker strictly before it
/// is untouched. This is the same offset-transform every rope/CRDT marker
/// scheme uses under the hood.
pub fn shift_anchor_for_insertion(
    anchor_byte: usize,
    insert_at_byte: usize,
    inserted_len: usize,
) -> usize {
    if insert_at_byte <= anchor_byte {
        anchor_byte + inserted_len
    } else {
        anchor_byte
    }
}

/// Finds the byte offset of the first non-whitespace character on the line
/// containing `marker` (e.g. a `// BREAKPOINT_TARGET` comment), using a real
/// `ropey::Rope` — the same data structure Spartan's core buffer is built on
/// (§2.1) — rather than raw string indexing, so this spike exercises the
/// actual dependency the product will use.
pub fn anchor_at_marker(rope: &Rope, marker: &str) -> usize {
    let text = rope.to_string();
    let marker_byte = text.find(marker).expect("marker present in source");
    let line_idx = rope.byte_to_line(marker_byte);
    let line_start_byte = rope.line_to_byte(line_idx);
    let line = rope.line(line_idx).to_string();
    let indent = line.len() - line.trim_start().len();
    line_start_byte + indent
}

/// 1-indexed DAP line number for a byte offset in `rope`.
pub fn line_1indexed_for_byte(rope: &Rope, byte_offset: usize) -> i64 {
    (rope.byte_to_line(byte_offset) + 1) as i64
}
