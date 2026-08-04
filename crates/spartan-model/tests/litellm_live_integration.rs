//! Real live verification of `LiteLLMProvider`'s actual network paths
//! (§75.99) -- the backlog row "Live `ClaudeProvider` / `LiteLLMProvider`
//! verification" says both providers are structurally complete but have
//! never been run against a real endpoint in this project's history. There
//! is no real LiteLLM proxy, no API key, and no model backend in this
//! environment, so this suite does the honest, still-fully-real thing: it
//! stands up a *real* local HTTP server (a `std::net::TcpListener` on
//! loopback, speaking the real OpenAI-compatible wire protocol the provider
//! is written against) and drives the provider through its real
//! `ureq`-backed HTTP/SSE code path end-to-end. What is verified here is
//! real: the real request shape (method/URL/JSON body), real
//! `text/event-stream` framing + SSE parsing, real per-chunk streaming
//! callbacks, real cooperative cancellation (§75.73/task #269), and real
//! liveness/health probes. What is NOT claimed (and cannot be in this
//! sandbox) is model intelligence -- the "model" behind the server is a
//! scripted wire fixture, exactly the boundary this project's own
//! established wire-faithful verification discipline draws.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use spartan_model::{
    CompletionRequest, Delta, LiteLLMProvider, ModelProvider, ProviderError, ProviderHealth,
    StopReason,
};

fn minimal_request() -> CompletionRequest {
    CompletionRequest {
        messages: vec![],
        tools: vec![],
        system_prompt: "You are a test model.".to_string(),
        max_tokens: 256,
        temperature: 0.2,
    }
}

/// Read one real HTTP request from the accepted socket: the request line,
/// headers, and the `Content-Length`-bounded body. Returns
/// `(request_line, body)`.
fn read_http_request(stream: &mut TcpStream) -> (String, String) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .expect("read request line");
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read header line");
        if line == "\r\n" || line == "\n" {
            break;
        }
        if line.to_ascii_lowercase().starts_with("content-length:") {
            content_length = line
                .split(':')
                .nth(1)
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).expect("read request body");
    }
    (request_line, String::from_utf8_lossy(&body).to_string())
}

/// Write a real HTTP response with SSE framing. Each entry becomes one real
/// `text/event-stream` frame (`data: {...}\n\n`).
fn write_sse_response(stream: &mut TcpStream, frames: &[&str]) {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
    stream
        .write_all(head.as_bytes())
        .expect("write response head");
    for frame in frames {
        stream
            .write_all(format!("data: {frame}\n\n").as_bytes())
            .expect("write sse frame");
        stream.flush().expect("flush sse frame");
    }
}

fn write_plain_response(stream: &mut TcpStream, status: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(resp.as_bytes()).expect("write response");
    stream.flush().expect("flush response");
}

/// Start a real loopback server running `handler` on the accepted
/// connection; returns the live port.
fn serve(handler: impl FnOnce(TcpStream) + Send + 'static) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        handler(stream);
    });
    port
}

#[test]
fn litellm_provider_streams_a_real_completion_over_real_http() {
    let port = serve(|mut stream| {
        let (request_line, body) = read_http_request(&mut stream);
        assert!(
            request_line.starts_with("POST /v1/chat/completions"),
            "real request line: {request_line}"
        );
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("real JSON body");
        assert_eq!(parsed["model"], "wire-model");
        assert_eq!(parsed["stream"], true);
        assert_eq!(parsed["messages"][0]["role"], "system");
        assert_eq!(parsed["messages"][0]["content"], "You are a test model.");

        write_sse_response(
            &mut stream,
            &[
                r#"{"choices":[{"delta":{"role":"assistant","content":""}}]}"#,
                r#"{"choices":[{"delta":{"content":"Hello "}}]}"#,
                r#"{"choices":[{"delta":{"content":"world"}}]}"#,
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
                "[DONE]",
            ],
        );
    });

    let provider = LiteLLMProvider::new(format!("http://127.0.0.1:{port}"), "wire-model");
    let mut deltas = Vec::new();
    provider
        .stream_completion(&minimal_request(), &mut |d| deltas.push(d))
        .expect("real stream should complete");

    assert_eq!(
        deltas,
        vec![
            Delta::TextChunk("Hello ".to_string()),
            Delta::TextChunk("world".to_string()),
            Delta::Stop {
                reason: StopReason::EndTurn
            },
        ]
    );
}

#[test]
fn litellm_provider_streams_a_real_incremental_tool_call_over_real_http() {
    let port = serve(|mut stream| {
        let (request_line, _body) = read_http_request(&mut stream);
        assert!(request_line.starts_with("POST /v1/chat/completions"));
        write_sse_response(
            &mut stream,
            &[
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","function":{"name":"read_file","arguments":""}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"/tmp/x\"}"}}]}}]}"#,
                r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
                "[DONE]",
            ],
        );
    });

    let provider = LiteLLMProvider::new(format!("http://127.0.0.1:{port}"), "wire-model");
    let mut deltas = Vec::new();
    provider
        .stream_completion(&minimal_request(), &mut |d| deltas.push(d))
        .expect("real tool-call stream should complete");

    assert_eq!(
        deltas,
        vec![
            Delta::ToolCallStart {
                id: "call_abc".to_string(),
                name: "read_file".to_string(),
            },
            Delta::ToolCallArgsChunk {
                id: "call_abc".to_string(),
                partial_json: "{\"path\":".to_string(),
            },
            Delta::ToolCallArgsChunk {
                id: "call_abc".to_string(),
                partial_json: "\"/tmp/x\"}".to_string(),
            },
            Delta::ToolCallEnd {
                id: "call_abc".to_string(),
            },
            Delta::Stop {
                reason: StopReason::ToolUse
            },
        ]
    );
}

#[test]
fn litellm_provider_cooperative_cancellation_cuts_a_real_stream() {
    let port = serve(|mut stream| {
        read_http_request(&mut stream);
        write_sse_response(
            &mut stream,
            &[
                r#"{"choices":[{"delta":{"role":"assistant","content":""}}]}"#,
                r#"{"choices":[{"delta":{"content":"first"}}]}"#,
            ],
        );
        // Hold the connection open long enough that the client has time to
        // observe its own cancel flag (set inside the delta callback) and
        // return before we close -- without this the test would race.
        thread::sleep(Duration::from_secs(2));
    });

    let provider = LiteLLMProvider::new(format!("http://127.0.0.1:{port}"), "wire-model");
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_flag = Arc::clone(&cancel);
    let mut deltas = Vec::new();

    // Runs synchronously: the callback sets the real cancel flag on the very
    // first delta, and `stream_completion_cancellable` checks it at the top
    // of its next read loop iteration *before* blocking -- so it returns
    // Err(Cancelled) without ever waiting on the server's held-open socket.
    let result = provider.stream_completion_cancellable(
        &minimal_request(),
        &mut |d| {
            deltas.push(d);
            cancel_flag.store(true, Ordering::SeqCst);
        },
        &cancel,
    );

    assert!(
        matches!(result, Err(ProviderError::Cancelled)),
        "expected Err(Cancelled), got {result:?}"
    );
    assert_eq!(
        deltas.len(),
        1,
        "only the pre-cancel delta should have been delivered"
    );
    assert!(matches!(&deltas[0], Delta::TextChunk(_)));
}

#[test]
fn litellm_provider_health_check_returns_the_real_wire_condition() {
    // 200 -> Healthy.
    let healthy_port = serve(|mut stream| {
        let (request_line, _body) = read_http_request(&mut stream);
        assert!(request_line.starts_with("GET /health/liveliness"));
        write_plain_response(&mut stream, "200 OK", "{\"status\":\"ok\"}");
    });
    let provider = LiteLLMProvider::new(format!("http://127.0.0.1:{healthy_port}"), "wire-model");
    assert_eq!(provider.health_check(), ProviderHealth::Healthy);

    // 401 -> Unauthorized (the real §75.99 fix: previously collapsed to
    // Unreachable, so a bad key was reported as "unreachable" in the UI).
    let unauthorized_port = serve(|mut stream| {
        read_http_request(&mut stream);
        write_plain_response(&mut stream, "401 Unauthorized", "{\"error\":\"bad key\"}");
    });
    let provider = LiteLLMProvider::new(
        format!("http://127.0.0.1:{unauthorized_port}"),
        "wire-model",
    );
    assert_eq!(provider.health_check(), ProviderHealth::Unauthorized);

    // No server at all -> Unreachable.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let dead_port = listener.local_addr().unwrap().port();
    drop(listener);
    let provider = LiteLLMProvider::new(format!("http://127.0.0.1:{dead_port}"), "wire-model");
    assert_eq!(provider.health_check(), ProviderHealth::Unreachable);
}
