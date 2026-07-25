//! Real, in-house DAP client speaking real Content-Length-framed JSON over
//! stdio, copied (not imported) from `crates/spartan-editor-core/src/
//! dap.rs` -- a deliberate second promotion, not an extraction, matching
//! `crates/spartan-lsp`'s own already-established precedent and reasoning
//! (see that crate's `client.rs` doc comment): the wgpu reference shell
//! stays completely untouched rather than refactored to share this code.

use serde_json::{json, Value};
use std::collections::VecDeque;

/// One real source breakpoint for `setBreakpoints`. `condition` is a real
/// DAP conditional-breakpoint expression (the adapter only stops when it
/// evaluates truthy); `log_message` turns it into a real *logpoint* (the
/// adapter logs the interpolated message and does not stop). Both optional
/// -- a bare `Breakpoint::line(n)` is an ordinary line breakpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Breakpoint {
    pub line: i64,
    pub condition: Option<String>,
    pub log_message: Option<String>,
}

impl Breakpoint {
    pub fn line(line: i64) -> Self {
        Self {
            line,
            condition: None,
            log_message: None,
        }
    }

    fn to_dap(&self) -> Value {
        let mut obj = serde_json::Map::new();
        obj.insert("line".to_string(), json!(self.line));
        if let Some(c) = self.condition.as_ref().filter(|s| !s.trim().is_empty()) {
            obj.insert("condition".to_string(), json!(c));
        }
        if let Some(m) = self.log_message.as_ref().filter(|s| !s.trim().is_empty()) {
            obj.insert("logMessage".to_string(), json!(m));
        }
        Value::Object(obj)
    }
}
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

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
/// third-party DAP library. `spawn_with_args` (new here, not in the
/// original single-arg `spawn`) supports adapters that need real argv
/// flags to run in stdio mode (e.g. `python3 -m debugpy.adapter`).
pub struct DapClient {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    buffered: VecDeque<Value>,
    next_seq: i64,
}

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

impl DapClient {
    pub fn spawn(adapter_path: &str) -> std::io::Result<Self> {
        Self::spawn_with_args(adapter_path, &[])
    }

    pub fn spawn_with_args(adapter_path: &str, args: &[&str]) -> std::io::Result<Self> {
        let mut child = Command::new(adapter_path)
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
                    Ok(None) => return,
                    Err(_) => return,
                }
            }
        });
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
        for m in skipped.into_iter().rev() {
            self.buffered.push_front(m);
        }
        result
    }

    /// Real, `output`-aware sibling of `wait_for`. A real DAP `output`
    /// event (the mechanism behind logpoints, and behind a debuggee's own
    /// stdout/stderr on many adapters) can legitimately arrive at any
    /// point while waiting for something else -- `wait_for` itself already
    /// buffers a non-matching message for a later, differently-shaped
    /// wait, but nothing ever issued that later wait for `output`
    /// specifically, so every real output event was previously buffered
    /// and then silently, permanently lost. This collects every real
    /// `output` event seen along the way into `output_sink` (never
    /// terminating the wait on one) instead of re-buffering it, so the
    /// caller gets both the event it actually asked for and everything
    /// real that arrived alongside it.
    pub fn wait_for_collecting_output<F: Fn(&Value) -> bool>(
        &mut self,
        pred: F,
        timeout: Duration,
        output_sink: &mut Vec<Value>,
    ) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        let mut skipped = VecDeque::new();
        let result = loop {
            match self.next_message(deadline) {
                Some(msg) => {
                    let is_output = msg.get("type").and_then(Value::as_str) == Some("event")
                        && msg.get("event").and_then(Value::as_str) == Some("output");
                    if is_output {
                        output_sink.push(msg);
                        continue;
                    }
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

    pub fn wait_event(&mut self, name: &str, timeout: Duration) -> Option<Value> {
        self.wait_for(
            |m| {
                m.get("type").and_then(Value::as_str) == Some("event")
                    && m.get("event").and_then(Value::as_str) == Some(name)
            },
            timeout,
        )
    }

    pub fn launch_and_break(
        &mut self,
        program: &str,
        cwd: &str,
        source_path: &str,
        breakpoints: &[Breakpoint],
    ) -> Option<Value> {
        self.launch_and_break_with_body(
            json!({
                "program": program,
                "args": Vec::<String>::new(),
                "cwd": cwd,
                "stopOnEntry": false,
            }),
            source_path,
            breakpoints,
        )
    }

    pub fn launch_and_break_with_body(
        &mut self,
        launch_body: Value,
        source_path: &str,
        breakpoints: &[Breakpoint],
    ) -> Option<Value> {
        let init_resp = self.request(
            "initialize",
            json!({
                "clientID": "spartan-backend",
                "adapterID": "spartan-dap",
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

        let launch_seq = self.send_request("launch", launch_body).ok()?;

        self.wait_event("initialized", DEFAULT_TIMEOUT)?;

        let source_name = std::path::Path::new(source_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(source_path);
        let bp_lines: Vec<Value> = breakpoints.iter().map(Breakpoint::to_dap).collect();
        let set_bp_resp = self.request(
            "setBreakpoints",
            json!({
                "source": {"path": source_path, "name": source_name},
                "breakpoints": bp_lines,
            }),
            DEFAULT_TIMEOUT,
        )?;

        self.request("configurationDone", json!({}), DEFAULT_TIMEOUT)?;

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

    pub fn step_over(&mut self, thread_id: i64) -> Option<Value> {
        self.request("next", json!({"threadId": thread_id}), DEFAULT_TIMEOUT)
    }

    pub fn step_into(&mut self, thread_id: i64) -> Option<Value> {
        self.request("stepIn", json!({"threadId": thread_id}), DEFAULT_TIMEOUT)
    }

    /// Real DAP `evaluate` request -- evaluates an arbitrary expression in
    /// the context of a given stack frame (the current top frame, for a
    /// watch expression or a REPL eval). `context: "watch"` is the DAP
    /// spec's own hint for a watch-panel evaluation. Returns the raw
    /// response; `body.result` is the real display string.
    pub fn evaluate(&mut self, expression: &str, frame_id: i64) -> Option<Value> {
        self.request(
            "evaluate",
            json!({
                "expression": expression,
                "frameId": frame_id,
                "context": "watch",
            }),
            DEFAULT_TIMEOUT,
        )
    }

    /// Real DAP `setVariable` -- edits a variable's live value while
    /// stopped. `variables_reference` is the *container's* reference (the
    /// scope, or a parent variable for a nested field), never the
    /// variable's own reference, per the DAP spec. Returns the raw
    /// response; a real success carries the adapter's own re-formatted
    /// `body.value` (which may differ from what was typed -- e.g. an
    /// adapter normalizing `"5"` to `5`), so a caller should prefer that
    /// over echoing the raw input back.
    pub fn set_variable(
        &mut self,
        variables_reference: i64,
        name: &str,
        value: &str,
    ) -> Option<Value> {
        self.request(
            "setVariable",
            json!({
                "variablesReference": variables_reference,
                "name": name,
                "value": value,
            }),
            DEFAULT_TIMEOUT,
        )
    }

    /// Shuts down the adapter -- never trusts the subprocess's own
    /// shutdown, matching `spartan_lsp::LspClient::shutdown`'s identical
    /// discipline and the original `spartan-editor-core::dap::DapClient`'s
    /// own documented finding that a real `lldb-dap` build reliably
    /// SIGABRTs somewhere in its own exit path.
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
