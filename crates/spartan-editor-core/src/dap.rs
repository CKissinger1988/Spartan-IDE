//! Promoted from `spikes/dap-spike/src/lib.rs` (§39.2, §47.5, §47.7) --
//! a real, in-house DAP client speaking real Content-Length-framed JSON
//! over stdio, proven against two independent real adapters (`lldb-dap`,
//! `debugpy`) by that spike's own tests. Promoted verbatim except for two
//! new methods, `step_over`/`step_into`, added below -- the spike itself
//! never needed them, but they're the exact same one-line `request()`
//! wrapper shape as the already-promoted `continue_()`. Deliberately does
//! NOT promote `shift_anchor_for_insertion`/`anchor_at_marker`/
//! `line_1indexed_for_byte` (rope-anchored breakpoint persistence -- out of
//! scope for this pass, see §75.8) or `compile_fixture` (test-only harness
//! code, kept in `tests/` instead).

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
/// exchanges Content-Length-framed JSON messages over its stdio -- no
/// third-party DAP library.
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
    /// Spawns the given debug adapter binary (e.g. `lldb-dap`) and starts
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
        // blocks on a full pipe buffer (this adapter is known to dump a
        // crash backtrace to stderr during its own exit path -- see
        // `shutdown`'s doc comment).
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

    /// Fires a request without blocking for its response, returning the
    /// request's own sequence number so a caller can later match the
    /// eventual response via `wait_for`. Real, deliberate `pub` visibility
    /// (§75.45): some real adapters (`debugpy`, and now confirmed
    /// `kotlin-debug-adapter` too) defer a request's response until after
    /// later requests in the same handshake, so blocking on `request`
    /// immediately can deadlock -- callers driving a non-standard handshake
    /// sequence (see `dap_kotlin_cross_language.rs`) need this same
    /// fire-and-forget primitive `launch_and_break_with_body` already uses
    /// internally.
    pub fn send_request(&mut self, command: &str, arguments: Value) -> std::io::Result<i64> {
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
    /// on the same stream. Real, deliberate `pub` visibility (§75.45): see
    /// `send_request`'s own doc comment -- callers pairing it with a
    /// non-standard handshake need to match the eventual response
    /// themselves.
    pub fn wait_for<F: Fn(&Value) -> bool>(&mut self, pred: F, timeout: Duration) -> Option<Value> {
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
    /// `configurationDone`: initialize, launch, wait for `initialized`, set
    /// breakpoints, configurationDone, then wait for the program to
    /// actually stop. Convenience wrapper over `launch_and_break_with_body`
    /// for the "spawn a program at a path" launch shape `lldb-dap`/
    /// `debugpy`/`dlv` all share.
    pub fn launch_and_break(
        &mut self,
        program: &str,
        cwd: &str,
        source_path: &str,
        break_lines: &[i64],
    ) -> Option<Value> {
        self.launch_and_break_with_body(
            json!({
                "program": program,
                "args": Vec::<String>::new(),
                "cwd": cwd,
                "stopOnEntry": false,
            }),
            source_path,
            break_lines,
        )
    }

    /// The same real launch sequence as `launch_and_break`, but taking an
    /// arbitrary real `launch` request body -- a real, live finding
    /// (§75.45): `kotlin-debug-adapter`'s real `launch` request shape
    /// (`mainClass`/`projectRoot`, no `program`/`cwd`/`args`/`stopOnEntry`
    /// at all -- confirmed by reading its actual installed
    /// `KotlinDebugAdapter.kt` source, not assumed) is fundamentally
    /// different from every other adapter this crate has driven so far,
    /// since Kotlin/JVM debugging launches a named class on a resolved
    /// classpath rather than spawning an executable/script at a path.
    pub fn launch_and_break_with_body(
        &mut self,
        launch_body: Value,
        source_path: &str,
        break_lines: &[i64],
    ) -> Option<Value> {
        let init_resp = self.request(
            "initialize",
            json!({
                "clientID": "spartan-editor-core",
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
        // responds immediately, but `debugpy` defers the `launch` response
        // until *after* `configurationDone` -- the debuggee doesn't
        // actually start running until then. Blocking here for the
        // response before sending `setBreakpoints`/`configurationDone`
        // would deadlock against any adapter using that (spec-legal)
        // deferred pattern. Fire the request, keep its seq, and only
        // collect the response once it's safe to have not gotten it yet
        // either way.
        let launch_seq = self.send_request("launch", launch_body).ok()?;

        self.wait_event("initialized", DEFAULT_TIMEOUT)?;

        // A real, live finding (§75.45): `kotlin-debug-adapter`'s real
        // `setBreakpoints` handler throws a real
        // `NullPointerException: getName(...) must not be null` in its own
        // `DAPConverter.toInternalSource` if the DAP `Source` object's
        // optional `name` field is omitted -- confirmed with a raw
        // hand-crafted protocol probe against the real adapter before this
        // fix, not assumed. `lldb-dap`/`debugpy`/`dlv` have all tolerated a
        // `source` object with only `path` throughout this crate's history
        // (this same code path, unchanged, is still exercised by
        // `dap_integration.rs`/`dap_python_cross_language.rs`'s own
        // continued passing), so a real file-basename `name` is now always
        // included -- a harmless, DAP-spec-legal extra field for adapters
        // that don't need it, a real requirement for the one that does.
        let source_name = std::path::Path::new(source_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(source_path);
        let bp_lines: Vec<Value> = break_lines.iter().map(|l| json!({"line": l})).collect();
        let set_bp_resp = self.request(
            "setBreakpoints",
            json!({
                "source": {"path": source_path, "name": source_name},
                "breakpoints": bp_lines,
            }),
            DEFAULT_TIMEOUT,
        )?;

        self.request("configurationDone", json!({}), DEFAULT_TIMEOUT)?;

        // Now collect the launch response -- already buffered and instantly
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

    /// New, not promoted from the spike -- the same one-line `request()`
    /// wrapper shape as `continue_()` above, for the standard DAP "next"
    /// (step-over) command.
    pub fn step_over(&mut self, thread_id: i64) -> Option<Value> {
        self.request("next", json!({"threadId": thread_id}), DEFAULT_TIMEOUT)
    }

    /// New, not promoted from the spike -- same shape, for DAP "stepIn"
    /// (step-into).
    pub fn step_into(&mut self, thread_id: i64) -> Option<Value> {
        self.request("stepIn", json!({"threadId": thread_id}), DEFAULT_TIMEOUT)
    }

    /// Shuts down the adapter. This build of `lldb-dap` reliably SIGABRTs
    /// (`free(): invalid pointer`) somewhere in its own exit path
    /// immediately after replying to `disconnect` -- with or without a
    /// live debuggee. Treating that as a Spartan bug would be wrong;
    /// treating it as trustworthy would be worse. So: ask nicely, read the
    /// response if it comes, then unconditionally reap the child on a
    /// short leash rather than trusting it to exit cleanly -- the same
    /// "never trust a subprocess's own shutdown" discipline `LspClient`
    /// (and `LspSession`/this crate's `DapSession`) already follow.
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
