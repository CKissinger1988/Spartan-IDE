//! Real, executed integration tests against a real local Ollama instance
//! (task #4, §3.3). Self-skips (prints a message, doesn't fail) if Ollama
//! isn't reachable or the model isn't pulled -- matching this workspace's
//! established `lsp_integration.rs`/`dap_integration.rs`/
//! `gui_bridge_integration.rs` convention for real external-tool
//! dependencies that don't exist on every machine.

use spartan_model::provider::{CompletionRequest, Delta, Message, ModelProvider, StopReason};
use spartan_model::{OllamaProvider, ToolDefinition};

const MODEL: &str = "llama3.1:8b";

fn provider() -> OllamaProvider {
    OllamaProvider::local(MODEL)
}

fn model_available() -> bool {
    provider().health_check() == spartan_model::ProviderHealth::Healthy
        && provider().context_window() > 0
}

#[test]
fn real_health_check_against_a_real_local_server() {
    let p = provider();
    if p.health_check() != spartan_model::ProviderHealth::Healthy {
        eprintln!("SKIP: Ollama not reachable at http://localhost:11434");
        return;
    }
    assert_eq!(p.health_check(), spartan_model::ProviderHealth::Healthy);
}

#[test]
fn real_context_window_and_native_tool_support_are_queried_from_the_real_server() {
    if !model_available() {
        eprintln!("SKIP: Ollama not reachable or {MODEL} not pulled");
        return;
    }
    let p = provider();
    // Real, live-queried figure from `/api/tags` -- llama3.1's real
    // published context length is 131072; asserting a wide floor rather
    // than the exact number so this doesn't break if Ollama's own
    // metadata format changes shape slightly.
    assert!(p.context_window() >= 4096);
    // llama3.1:8b's real, live-queried `/api/tags` capabilities array
    // includes "tools" (confirmed via a real `curl` trial before this
    // code was written -- see `ollama.rs`'s own doc comment).
    assert!(p.supports_native_tool_calling());
}

#[test]
fn real_streaming_text_completion_against_a_real_local_model() {
    if !model_available() {
        eprintln!("SKIP: Ollama not reachable or {MODEL} not pulled");
        return;
    }
    let p = provider();
    let request = CompletionRequest {
        messages: vec![Message::user("Say hello in exactly one word.")],
        tools: vec![],
        system_prompt: String::new(),
        max_tokens: 50,
        temperature: 0.0,
    };
    let mut chunks = Vec::new();
    let mut saw_stop = false;
    p.stream_completion(&request, &mut |delta| {
        if let Delta::TextChunk(text) = &delta {
            chunks.push(text.clone());
        }
        if matches!(delta, Delta::Stop { .. }) {
            saw_stop = true;
        }
    })
    .expect("a real streaming completion against a real reachable server should succeed");

    assert!(
        saw_stop,
        "a real completion must end with a real Stop delta"
    );
    assert!(
        !chunks.concat().trim().is_empty(),
        "a real model should produce real, non-empty text"
    );
}

/// The most substantial real test in this suite: a real native
/// tool-calling round trip against a real local model, confirming
/// `OllamaProvider` doesn't just parse a *hand-written* fixture (unlike
/// `claude.rs`'s own fixture-based tests) but a real, model-generated
/// `tool_calls` payload from a real running server.
#[test]
fn real_native_tool_call_against_a_real_local_model() {
    if !model_available() {
        eprintln!("SKIP: Ollama not reachable or {MODEL} not pulled");
        return;
    }
    let p = provider();
    let request = CompletionRequest {
        messages: vec![Message::user(
            "Please read the file at src/main.rs so you can see what's in it.",
        )],
        tools: vec![ToolDefinition {
            name: "read_file".to_string(),
            description: "Reads a file's contents from disk".to_string(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
        }],
        system_prompt: String::new(),
        max_tokens: 200,
        temperature: 0.0,
    };

    let mut saw_start = false;
    let mut saw_end = false;
    let mut tool_name = String::new();
    let mut args_json = String::new();
    let mut stop_reason = None;

    p.stream_completion(&request, &mut |delta| match delta {
        Delta::ToolCallStart { name, .. } => {
            saw_start = true;
            tool_name = name;
        }
        Delta::ToolCallArgsChunk { partial_json, .. } => {
            args_json.push_str(&partial_json);
        }
        Delta::ToolCallEnd { .. } => saw_end = true,
        Delta::Stop { reason } => stop_reason = Some(reason),
        Delta::TextChunk(_) => {}
    })
    .expect("a real native tool-calling request should succeed");

    assert!(saw_start, "a real tool call should emit ToolCallStart");
    assert!(saw_end, "a real tool call should emit ToolCallEnd");
    assert_eq!(tool_name, "read_file");
    let parsed: serde_json::Value =
        serde_json::from_str(&args_json).expect("real tool args should be valid JSON");
    assert_eq!(parsed["path"], "src/main.rs");
    assert_eq!(stop_reason, Some(StopReason::ToolUse));
}
