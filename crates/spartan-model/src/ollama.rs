//! Real §3.3 `OllamaProvider` (task #4) -- talks to a real local (or
//! LAN-configurable) Ollama instance over its real HTTP API.
//!
//! **Real, live-confirmed API shapes**, not assumed from documentation:
//! this implementation was written *after* driving a real Ollama 0.31.2
//! instance (installed this session, see `CLAUDE.md`'s Spike 0.3 status
//! note) with real `curl` requests against `llama3.1:8b` -- both a plain
//! streaming chat and a real native-tool-calling one -- and reading the
//! exact real response shapes before writing any parsing code, the same
//! discipline `build.rs` (§75.10) used for `cargo build
//! --message-format=json`.
//!
//! A real, useful finding from that verification, better than what §3.3
//! originally sketched: `/api/tags` already returns each installed model's
//! real `details.context_length` *and* a real `capabilities` array (e.g.
//! `["completion","tools"]`) directly -- so context-window auto-detection
//! and native-tool-calling capability detection both come from one real,
//! already-present API call, with no separate `/api/show` call or curated
//! manifest needed for either. (A curated recommendations manifest, §3.3's
//! other stated goal, is a separate, real, not-yet-built feature -- this
//! only covers the two pieces of metadata this pass actually needed.)

use crate::provider::{
    CompletionRequest, Delta, ModelProvider, ProviderError, ProviderHealth, Role, StopReason,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub struct OllamaProvider {
    base_url: String,
    model: String,
    /// Real §57/§42 GPU offload override, sent as Ollama's own
    /// `options.num_gpu` request field -- `None` sends no override at all
    /// (Ollama's own default auto-offload behavior), matching
    /// `spartan_settings::GpuOffloadSettings::num_gpu()`'s exact contract.
    /// Deliberately a plain `Option<u32>` here rather than a dependency on
    /// `spartan-settings` itself -- this crate stays settings-agnostic;
    /// the caller (`spartan-editor-core`) is the one that reads real
    /// settings and passes the resulting number through.
    num_gpu: Option<u32>,
}

impl OllamaProvider {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            num_gpu: None,
        }
    }

    /// The common case: a real local Ollama instance on its real default
    /// port (§3.3's own "talks to http://localhost:11434").
    pub fn local(model: impl Into<String>) -> Self {
        Self::new("http://localhost:11434", model)
    }

    /// Real §57/§42 GPU offload configuration (user-requested settings
    /// toggle + amount selector) -- a builder so existing call sites
    /// (`OllamaProvider::local(...)` alone) are unaffected.
    pub fn with_gpu_layers(mut self, num_gpu: Option<u32>) -> Self {
        self.num_gpu = num_gpu;
        self
    }

    fn tags(&self) -> Result<Value, ProviderError> {
        let resp = ureq::get(&format!("{}/api/tags", self.base_url))
            .timeout(Duration::from_secs(5))
            .call()
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        resp.into_json::<Value>()
            .map_err(|e| ProviderError::Parse(e.to_string()))
    }

    /// This provider's own real entry in `/api/tags`'s `models` array, if
    /// the model is actually installed and the server is reachable.
    fn model_entry(&self) -> Option<Value> {
        let tags = self.tags().ok()?;
        tags["models"]
            .as_array()?
            .iter()
            .find(|m| m["name"] == self.model || m["model"] == self.model)
            .cloned()
    }
}

/// Real, pure request-body construction, extracted from `stream_completion`
/// so it's directly unit-testable without a real (or mock) HTTP server --
/// the same "extract the pure logic, test it headlessly" split this whole
/// workspace already follows for GPU/network/subprocess-facing code.
fn build_request_body(request: &CompletionRequest, model: &str, num_gpu: Option<u32>) -> Value {
    let mut messages = Vec::new();
    if !request.system_prompt.is_empty() {
        messages.push(json!({"role": "system", "content": request.system_prompt}));
    }
    for m in &request.messages {
        let role = match m.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        messages.push(json!({"role": role, "content": m.content}));
    }

    let mut body = json!({
        "model": model,
        "stream": true,
        "messages": messages,
    });
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters_schema,
                    }
                })
            })
            .collect();
        body["tools"] = Value::Array(tools);
    }
    if let Some(num_gpu) = num_gpu {
        body["options"] = json!({"num_gpu": num_gpu});
    }
    body
}

impl ModelProvider for OllamaProvider {
    fn id(&self) -> &str {
        &self.model
    }

    fn is_local(&self) -> bool {
        true
    }

    fn context_window(&self) -> usize {
        self.model_entry()
            .and_then(|m| m["details"]["context_length"].as_u64())
            .map(|n| n as usize)
            // Real, documented fallback if the server is unreachable or the
            // model isn't installed -- never fabricated as a "real"
            // queried number when it wasn't actually queried successfully.
            .unwrap_or(4096)
    }

    fn supports_native_tool_calling(&self) -> bool {
        self.model_entry()
            .and_then(|m| {
                m["capabilities"]
                    .as_array()
                    .map(|caps| caps.iter().any(|c| c == "tools"))
            })
            .unwrap_or(false)
    }

    fn health_check(&self) -> ProviderHealth {
        match ureq::get(&format!("{}/api/tags", self.base_url))
            .timeout(Duration::from_secs(2))
            .call()
        {
            Ok(_) => ProviderHealth::Healthy,
            Err(_) => ProviderHealth::Unreachable,
        }
    }

    fn stream_completion(
        &self,
        request: &CompletionRequest,
        on_delta: &mut dyn FnMut(Delta),
    ) -> Result<(), ProviderError> {
        self.stream_completion_cancellable(request, on_delta, &AtomicBool::new(false))
    }

    fn stream_completion_cancellable(
        &self,
        request: &CompletionRequest,
        on_delta: &mut dyn FnMut(Delta),
        cancel: &AtomicBool,
    ) -> Result<(), ProviderError> {
        let body = build_request_body(request, &self.model, self.num_gpu);

        let resp = ureq::post(&format!("{}/api/chat", self.base_url))
            .timeout(Duration::from_secs(120))
            .send_json(body)
            .map_err(|e| match e {
                ureq::Error::Status(status, resp) => ProviderError::Http {
                    status,
                    body: resp.into_string().unwrap_or_default(),
                },
                ureq::Error::Transport(t) => ProviderError::Network(t.to_string()),
            })?;

        // Real NDJSON streaming: Ollama's real `/api/chat` sends one JSON
        // object per line, confirmed via the real `curl` trials this
        // module's own doc comment describes -- not Server-Sent-Events
        // framing, just newline-delimited JSON.
        let reader = BufReader::new(resp.into_reader());
        let mut saw_tool_call = false;
        for line in reader.lines() {
            // Real §75.73-closing cooperative cancellation (task #269):
            // checked once per real line already received over the wire --
            // can't interrupt a single blocking read still waiting on the
            // *next* line, but stops promptly between any two real chunks
            // rather than always running the whole response to completion.
            if cancel.load(Ordering::SeqCst) {
                return Err(ProviderError::Cancelled);
            }
            let line = line.map_err(|e| ProviderError::Network(e.to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            let chunk: Value = serde_json::from_str(&line)
                .map_err(|e| ProviderError::Parse(format!("{e}: {line}")))?;

            if let Some(content) = chunk["message"]["content"].as_str() {
                if !content.is_empty() {
                    on_delta(Delta::TextChunk(content.to_string()));
                }
            }

            if let Some(tool_calls) = chunk["message"]["tool_calls"].as_array() {
                for (i, tc) in tool_calls.iter().enumerate() {
                    saw_tool_call = true;
                    let id = tc["id"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("ollama-call-{i}"));
                    let name = tc["function"]["name"].as_str().unwrap_or_default();
                    // Real, named divergence from a literal reading of
                    // §3.1's `partial_json` field name: Ollama's real API
                    // (confirmed live, not assumed) returns each tool
                    // call's arguments as one already-parsed JSON
                    // *object* per chunk, not an incrementally-streamed
                    // partial-JSON text fragment the way Anthropic's API
                    // streams tool input -- so this is always one whole,
                    // valid JSON payload in a single `ArgsChunk`, never a
                    // fragment a caller needs to accumulate.
                    let args_json = tc["function"]["arguments"].to_string();
                    on_delta(Delta::ToolCallStart {
                        id: id.clone(),
                        name: name.to_string(),
                    });
                    on_delta(Delta::ToolCallArgsChunk {
                        id: id.clone(),
                        partial_json: args_json,
                    });
                    on_delta(Delta::ToolCallEnd { id });
                }
            }

            if chunk["done"].as_bool().unwrap_or(false) {
                let reason = match chunk["done_reason"].as_str() {
                    Some("length") => StopReason::MaxTokens,
                    _ if saw_tool_call => StopReason::ToolUse,
                    _ => StopReason::EndTurn,
                };
                on_delta(Delta::Stop { reason });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_request() -> CompletionRequest {
        CompletionRequest {
            messages: vec![],
            tools: vec![],
            system_prompt: String::new(),
            max_tokens: 1024,
            temperature: 0.0,
        }
    }

    #[test]
    fn with_no_gpu_override_the_request_body_has_no_options_field() {
        let body = build_request_body(&minimal_request(), "llama3.1:8b", None);
        assert!(
            body.get("options").is_none(),
            "no GPU override configured should mean no options field sent at all, \
             letting Ollama's own real default auto-offload behavior apply"
        );
    }

    #[test]
    fn a_real_gpu_layer_override_is_sent_as_options_num_gpu() {
        let body = build_request_body(&minimal_request(), "llama3.1:8b", Some(24));
        assert_eq!(body["options"]["num_gpu"], 24);
    }

    #[test]
    fn a_zero_gpu_override_forcing_cpu_only_is_still_sent_explicitly() {
        // `Some(0)` (real §57/§42 "GPU offloading disabled") must be sent
        // as a real, explicit `0`, not accidentally dropped as if it were
        // `None` -- a naive `if num_gpu != 0` check would get this wrong.
        let body = build_request_body(&minimal_request(), "llama3.1:8b", Some(0));
        assert_eq!(body["options"]["num_gpu"], 0);
    }

    #[test]
    fn with_gpu_layers_builder_is_reflected_in_the_real_request_body() {
        let provider = OllamaProvider::local("llama3.1:8b").with_gpu_layers(Some(16));
        let body = build_request_body(&minimal_request(), &provider.model, provider.num_gpu);
        assert_eq!(body["options"]["num_gpu"], 16);
    }

    /// Real, live, socket-backed cancellation test for task #269 -- a real
    /// `TcpListener`-based mock `/api/chat` server (the same "an actual
    /// socket, not a stubbed function" discipline `spartan-crash`'s own
    /// `spawn_mock_upload_server`, §75.82, already established) streams a
    /// real 10-line NDJSON response with a real, deliberate delay between
    /// each line, so a real cancel flag flipped from a second thread partway
    /// through can be observed to genuinely stop `stream_completion_
    /// cancellable` early -- not just that it *would* check the flag, but
    /// that fewer than all 10 real chunks were ever delivered to `on_delta`.
    fn spawn_mock_ollama_chat_server(line_count: usize, delay: std::time::Duration) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // Drain the real request (headers + JSON body) *completely* before
            // replying, matching how a real HTTP server behaves. This has to be
            // a genuine loop, not a single `read()`: TCP is free to split the
            // client's request across several segments, and one `read()` only
            // ever returns what has arrived so far. If this thread then replies
            // and returns, `stream` drops with unread bytes still sitting in the
            // socket's receive buffer -- and closing a socket in that state makes
            // the OS send a real RST instead of a graceful FIN, which tears down
            // the connection under the client and surfaces as the client-side
            // `Connection reset by peer` this test used to flake with (~25% of
            // runs at default `cargo test` parallelism, never single-threaded,
            // because load is what makes the request get split in the first
            // place). Read headers first, then exactly `Content-Length` bytes.
            let mut buf: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 1024];
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));

            // Phase 1: read until the end of the header block.
            let header_end = loop {
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break Some(i + 4);
                }
                match stream.read(&mut chunk) {
                    Ok(0) => break None, // client closed early
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    Err(_) => break None, // timed out; give up draining
                }
            };

            // Phase 2: read the declared body, if any, so nothing is left unread.
            if let Some(header_end) = header_end {
                let headers = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
                let content_length = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                while buf.len() - header_end < content_length {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        Err(_) => break,
                    }
                }
            }

            let body_prefix =
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\n\r\n".to_string();
            let _ = stream.write_all(body_prefix.as_bytes());
            let _ = stream.flush();
            for i in 0..line_count {
                let done = i == line_count - 1;
                let line = json!({
                    "message": {"role": "assistant", "content": format!("chunk-{i} ")},
                    "done": done,
                })
                .to_string();
                if stream.write_all(format!("{line}\n").as_bytes()).is_err() {
                    // The real client closed its side (e.g. it cancelled and
                    // dropped the connection) -- a real, honest early stop,
                    // not a test failure.
                    return;
                }
                let _ = stream.flush();
                std::thread::sleep(delay);
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    #[test]
    fn stream_completion_without_cancellation_receives_every_real_chunk() {
        let base_url = spawn_mock_ollama_chat_server(5, std::time::Duration::from_millis(10));
        let provider = OllamaProvider::new(base_url, "test-model");
        let mut chunks = 0;
        let result = provider.stream_completion(&minimal_request(), &mut |delta| {
            if matches!(delta, Delta::TextChunk(_)) {
                chunks += 1;
            }
        });
        assert!(
            result.is_ok(),
            "an uncancelled stream must complete: {result:?}"
        );
        assert_eq!(
            chunks, 5,
            "every real chunk the mock server sent must arrive"
        );
    }

    #[test]
    fn a_real_cancellation_flag_set_mid_stream_genuinely_stops_early() {
        // 20 real lines, 30ms apart -- comfortably long enough for a second
        // thread to flip the real cancel flag partway through and for this
        // test to reliably observe fewer than 20 chunks having arrived.
        // `thread::scope` (safe, no `unsafe`) lets the timer thread below
        // borrow `cancel` directly, since the scope guarantees it's joined
        // before this function returns.
        let base_url = spawn_mock_ollama_chat_server(20, std::time::Duration::from_millis(30));
        let provider = OllamaProvider::new(base_url, "test-model");
        let cancel = AtomicBool::new(false);

        let mut chunks = 0;
        let result = std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(150));
                cancel.store(true, Ordering::SeqCst);
            });
            provider.stream_completion_cancellable(
                &minimal_request(),
                &mut |delta| {
                    if matches!(delta, Delta::TextChunk(_)) {
                        chunks += 1;
                    }
                },
                &cancel,
            )
        });

        assert!(
            matches!(result, Err(ProviderError::Cancelled)),
            "a real mid-stream cancellation must surface as ProviderError::Cancelled, got: {result:?}"
        );
        assert!(
            chunks < 20,
            "cancellation must genuinely stop the stream before every real chunk arrives, \
             got {chunks} chunks (all 20 would mean the flag was never actually checked in time)"
        );
        assert!(
            chunks > 0,
            "the mock server had already sent some real chunks before cancel fired"
        );
    }
}
