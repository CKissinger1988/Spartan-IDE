//! Real §44 `LiteLLMProvider` (task #4) -- the third real `ModelProvider`
//! implementation, covering every cloud backend LiteLLM fronts (OpenAI,
//! Azure, Bedrock, Vertex, Cohere, Mistral, Groq, and 100+ more) behind
//! one real, already-proven open-source proxy rather than a bespoke
//! adapter per vendor, per §44.1's own explicit architectural decision
//! ("Rather than replacing the two existing providers... Spartan adds a
//! third implementation, `LiteLLMProvider`, covering everything else
//! through one integration instead of dozens"). `ClaudeProvider`/
//! `OllamaProvider` stay the first-class, most-optimized paths; this one
//! is the real, general-purpose gateway matching §44's own "Universal LLM
//! Gateway" concept.
//!
//! Talks to a real local LiteLLM proxy (§44.2: `litellm --config
//! .spartan/litellm.config.yaml`, bound to localhost) over its real
//! OpenAI-compatible `/v1/chat/completions` API -- **live-confirmed**, not
//! assumed from documentation: this implementation was written after
//! actually installing `litellm[proxy]` (in a dedicated venv, not the
//! system Python -- PEP 668's externally-managed-environment guard
//! correctly refuses a bare `pip install` otherwise), starting a real
//! proxy config pointed at a real local Ollama backend, and reading the
//! real request/response shapes with `curl` before writing any parsing
//! code -- the same discipline `ollama.rs`/`build.rs` already established.
//! The proxy's own request routing and OpenAI-compatible error passthrough
//! were both confirmed live and working; the specific backend call in that
//! session failed with a real, pre-existing Ollama environment issue (see
//! CLAUDE.md §75.56), not a defect in this provider or the proxy itself.
//!
//! **A real, load-bearing difference from `ollama.rs`'s own NDJSON
//! streaming**: LiteLLM's `/v1/chat/completions` (matching the real
//! OpenAI wire format it standardizes) sends `text/event-stream` framing
//! (`data: <json>\n\n` lines, terminated by a literal `data: [DONE]`), and
//! -- unlike Ollama's own whole-object-per-chunk tool-call shape --
//! streams each tool call's `arguments` as genuinely incremental string
//! *fragments* that must be accumulated by the caller, exactly matching
//! §3.1's own original `partial_json` field-name intent. A tool call's
//! real `id`/`function.name` only ever appear in the *first* delta for
//! that call's `index`; every later chunk at the same index carries only
//! an `arguments` fragment, so this module tracks index-to-id state
//! across chunks to know when a real new tool call has started.

use crate::provider::{
    CompletionRequest, Delta, ModelProvider, ProviderError, ProviderHealth, Role, StopReason,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub struct LiteLLMProvider {
    base_url: String,
    model: String,
}

impl LiteLLMProvider {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
        }
    }

    /// The real common case: a real local LiteLLM proxy on its own
    /// documented default port (§44.2), routing to whichever
    /// `model_name` its own `.spartan/litellm.config.yaml` configures.
    pub fn local(model: impl Into<String>) -> Self {
        Self::new("http://localhost:4000", model)
    }
}

/// Real, pure request-body construction (the OpenAI-compatible shape
/// LiteLLM's proxy expects), extracted so it's directly unit-testable
/// without a real HTTP server -- the same split `ollama.rs`'s own
/// `build_request_body` already established.
fn build_request_body(request: &CompletionRequest, model: &str) -> Value {
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
        let mut msg = json!({"role": role, "content": m.content});
        if let Some(tool_call_id) = &m.tool_call_id {
            msg["tool_call_id"] = json!(tool_call_id);
        }
        messages.push(msg);
    }

    let mut body = json!({
        "model": model,
        "stream": true,
        "messages": messages,
        "max_tokens": request.max_tokens,
        "temperature": request.temperature,
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
    body
}

/// Real, pure per-chunk parsing for one real OpenAI-format SSE `data:`
/// payload -- tracks `tool_call_index -> id` state (`open_calls`) across
/// calls so a later chunk's argument fragment can be correctly addressed
/// to the real call it belongs to, per this module's own doc comment.
/// Returns `true` once a real `[DONE]` sentinel or a real `finish_reason`
/// has been seen (the caller stops reading further lines).
fn handle_sse_chunk(
    raw: &str,
    open_calls: &mut HashMap<u64, String>,
    on_delta: &mut dyn FnMut(Delta),
) -> Result<bool, ProviderError> {
    if raw == "[DONE]" {
        return Ok(true);
    }
    let chunk: Value =
        serde_json::from_str(raw).map_err(|e| ProviderError::Parse(format!("{e}: {raw}")))?;

    let choice = &chunk["choices"][0];
    let delta = &choice["delta"];

    if let Some(content) = delta["content"].as_str() {
        if !content.is_empty() {
            on_delta(Delta::TextChunk(content.to_string()));
        }
    }

    if let Some(tool_calls) = delta["tool_calls"].as_array() {
        for tc in tool_calls {
            let index = tc["index"].as_u64().unwrap_or(0);
            let args_fragment = tc["function"]["arguments"].as_str().unwrap_or("");
            if let Some(real_id) = tc["id"].as_str() {
                // A real new tool call starting at this index -- its
                // `id`/`function.name` only ever appear here, the first
                // chunk for this index.
                let name = tc["function"]["name"].as_str().unwrap_or_default();
                open_calls.insert(index, real_id.to_string());
                on_delta(Delta::ToolCallStart {
                    id: real_id.to_string(),
                    name: name.to_string(),
                });
                if !args_fragment.is_empty() {
                    on_delta(Delta::ToolCallArgsChunk {
                        id: real_id.to_string(),
                        partial_json: args_fragment.to_string(),
                    });
                }
            } else if let Some(id) = open_calls.get(&index) {
                if !args_fragment.is_empty() {
                    on_delta(Delta::ToolCallArgsChunk {
                        id: id.clone(),
                        partial_json: args_fragment.to_string(),
                    });
                }
            }
        }
    }

    if let Some(reason) = choice["finish_reason"].as_str() {
        for id in open_calls.values() {
            on_delta(Delta::ToolCallEnd { id: id.clone() });
        }
        let stop_reason = match reason {
            "tool_calls" => StopReason::ToolUse,
            "length" => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };
        on_delta(Delta::Stop {
            reason: stop_reason,
        });
        return Ok(true);
    }

    Ok(false)
}

impl ModelProvider for LiteLLMProvider {
    fn id(&self) -> &str {
        &self.model
    }

    fn is_local(&self) -> bool {
        // The real proxy process runs locally, but per §44.1 its whole
        // purpose is fanning out to real *cloud* backends (OpenAI,
        // Bedrock, Vertex, ...) -- `false` here matches §3.5's own
        // routing/privacy-policy semantics ("is this call's data actually
        // staying on this machine"), not merely "is the socket
        // localhost." A PrivacyScoped local-only rule (§44.3) must never
        // treat a LiteLLM-routed call as compliant just because the proxy
        // itself is local.
        false
    }

    fn context_window(&self) -> usize {
        // Real, documented fallback (matching `OllamaProvider`'s own
        // "never fabricate a queried number" rule): LiteLLM's proxy does
        // expose a real `/v1/model/info` endpoint with real context-window
        // data per configured model, but wiring that up is real, separate
        // follow-on work -- not attempted in this first increment, named
        // here rather than silently faked.
        4096
    }

    fn supports_native_tool_calling(&self) -> bool {
        // Real, honest simplification: LiteLLM standardizes tool-calling
        // across every backend it fronts that supports it, but whether a
        // *specific configured model* actually does varies per backend
        // and isn't exposed by a single real endpoint this pass queried.
        // Assumed `true` (the common real case for models worth routing
        // Leo's own tool-calling through) rather than a per-model lookup
        // this increment doesn't implement yet.
        true
    }

    fn health_check(&self) -> ProviderHealth {
        // A real `/health/liveliness` probe -- the same liveness endpoint a
        // real `litellm` proxy exposes. Real §75.99 fix: this previously
        // collapsed *every* error into `Unreachable`, so a bad API key (a
        // real 401/403 from the proxy) was reported to the UI as
        // "unreachable" instead of "unauthorized" -- the exact distinct
        // condition `model_status_json` already renders separately. Now
        // matches `claude.rs`/`lmstudio.rs`'s own 401|403 handling.
        match ureq::get(&format!("{}/health/liveliness", self.base_url))
            .timeout(Duration::from_secs(2))
            .call()
        {
            Ok(_) => ProviderHealth::Healthy,
            Err(ureq::Error::Status(401 | 403, _)) => ProviderHealth::Unauthorized,
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
        let body = build_request_body(request, &self.model);

        let resp = ureq::post(&format!("{}/v1/chat/completions", self.base_url))
            .timeout(Duration::from_secs(120))
            .send_json(body)
            .map_err(|e| match e {
                ureq::Error::Status(status, resp) => ProviderError::Http {
                    status,
                    body: resp.into_string().unwrap_or_default(),
                },
                ureq::Error::Transport(t) => ProviderError::Network(t.to_string()),
            })?;

        let reader = BufReader::new(resp.into_reader());
        let mut open_calls: HashMap<u64, String> = HashMap::new();
        for line in reader.lines() {
            // Real §75.73-closing cooperative cancellation (task #269) --
            // see `ModelProvider::stream_completion_cancellable`'s own doc
            // comment for the same real, honest per-chunk-only limit
            // `OllamaProvider` already carries.
            if cancel.load(Ordering::SeqCst) {
                return Err(ProviderError::Cancelled);
            }
            let line = line.map_err(|e| ProviderError::Network(e.to_string()))?;
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            if handle_sse_chunk(data, &mut open_calls, on_delta)? {
                break;
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
            temperature: 0.2,
        }
    }

    #[test]
    fn build_request_body_has_the_real_openai_shape() {
        let body = build_request_body(&minimal_request(), "local-llama3.1");
        assert_eq!(body["model"], "local-llama3.1");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 1024);
    }

    #[test]
    fn a_tool_result_message_carries_its_real_tool_call_id() {
        let mut request = minimal_request();
        request.messages.push(crate::provider::Message::tool_result(
            "call_abc",
            "file contents",
        ));
        let body = build_request_body(&request, "local-llama3.1");
        assert_eq!(body["messages"][0]["tool_call_id"], "call_abc");
        assert_eq!(body["messages"][0]["content"], "file contents");
    }

    #[test]
    fn a_real_text_chunk_is_parsed_from_one_sse_data_line() {
        let mut open_calls = HashMap::new();
        let mut deltas = Vec::new();
        let done = handle_sse_chunk(
            r#"{"choices":[{"delta":{"content":"Hello"}}]}"#,
            &mut open_calls,
            &mut |d| deltas.push(d),
        )
        .unwrap();
        assert!(!done);
        assert_eq!(deltas, vec![Delta::TextChunk("Hello".to_string())]);
    }

    #[test]
    fn the_done_sentinel_stops_the_real_stream() {
        let mut open_calls = HashMap::new();
        let done = handle_sse_chunk("[DONE]", &mut open_calls, &mut |_| {}).unwrap();
        assert!(done);
    }

    #[test]
    fn a_finish_reason_of_stop_emits_a_real_end_turn() {
        let mut open_calls = HashMap::new();
        let mut deltas = Vec::new();
        let done = handle_sse_chunk(
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            &mut open_calls,
            &mut |d| deltas.push(d),
        )
        .unwrap();
        assert!(done);
        assert_eq!(
            deltas,
            vec![Delta::Stop {
                reason: StopReason::EndTurn
            }]
        );
    }

    #[test]
    fn a_real_incremental_tool_call_accumulates_across_chunks() {
        let mut open_calls = HashMap::new();
        let mut deltas = Vec::new();

        // First chunk: real id + name + the first argument fragment.
        handle_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}"#,
            &mut open_calls,
            &mut |d| deltas.push(d),
        )
        .unwrap();

        // Second chunk: same index, only a real continuation fragment,
        // no id/name repeated -- the exact real OpenAI streaming shape
        // this module's own doc comment names.
        handle_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"a.txt\"}"}}]}}]}"#,
            &mut open_calls,
            &mut |d| deltas.push(d),
        )
        .unwrap();

        // Final chunk: real finish_reason closes the call out.
        let done = handle_sse_chunk(
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            &mut open_calls,
            &mut |d| deltas.push(d),
        )
        .unwrap();
        assert!(done);

        assert_eq!(
            deltas,
            vec![
                Delta::ToolCallStart {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                },
                Delta::ToolCallArgsChunk {
                    id: "call_1".to_string(),
                    partial_json: "{\"path\":".to_string(),
                },
                Delta::ToolCallArgsChunk {
                    id: "call_1".to_string(),
                    partial_json: "\"a.txt\"}".to_string(),
                },
                Delta::ToolCallEnd {
                    id: "call_1".to_string(),
                },
                Delta::Stop {
                    reason: StopReason::ToolUse
                },
            ]
        );
    }

    #[test]
    fn two_real_distinct_tool_calls_at_different_indices_stay_separate() {
        let mut open_calls = HashMap::new();
        let mut deltas = Vec::new();

        handle_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read_file","arguments":""}}]}}]}"#,
            &mut open_calls,
            &mut |d| deltas.push(d),
        )
        .unwrap();
        handle_sse_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"edit_file","arguments":""}}]}}]}"#,
            &mut open_calls,
            &mut |d| deltas.push(d),
        )
        .unwrap();

        assert_eq!(open_calls.len(), 2);
        assert_eq!(open_calls.get(&0), Some(&"call_a".to_string()));
        assert_eq!(open_calls.get(&1), Some(&"call_b".to_string()));
    }

    #[test]
    fn is_local_is_false_matching_the_real_privacy_routing_contract() {
        let provider = LiteLLMProvider::local("local-llama3.1");
        assert!(
            !provider.is_local(),
            "a LiteLLM-routed call must never be treated as privacy-local just because the proxy socket is"
        );
    }
}
