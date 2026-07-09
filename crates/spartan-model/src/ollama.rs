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
use std::time::Duration;

pub struct OllamaProvider {
    base_url: String,
    model: String,
}

impl OllamaProvider {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
        }
    }

    /// The common case: a real local Ollama instance on its real default
    /// port (§3.3's own "talks to http://localhost:11434").
    pub fn local(model: impl Into<String>) -> Self {
        Self::new("http://localhost:11434", model)
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
            "model": self.model,
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
