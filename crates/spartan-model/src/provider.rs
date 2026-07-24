//! Real §3.1 `ModelProvider` abstraction (task #4), first real increment.
//!
//! **A real, deliberate adaptation from the spec's literal trait shape,
//! named up front rather than silently diverging.** §3.1 sketches
//! `ModelProvider` as `#[async_trait]` with `stream_completion` returning a
//! `Pin<Box<dyn Stream<...>>>`. This workspace has never used `async`/
//! `tokio` anywhere in real product code -- every real subprocess/network
//! integration so far (`lsp_session.rs`, `dap_session.rs`, `build.rs`,
//! `gui_bridge.rs`, and `fallback-parser-spike`'s own already-real `ureq`
//! usage) uses the same established pattern instead: blocking I/O on a
//! dedicated thread, results delivered back via `std::sync::mpsc`. Adopting
//! `tokio` for this one trait would be a much larger, separate
//! architectural decision (a second, competing concurrency model alongside
//! the thread+channel one already proven throughout this codebase), not
//! something to introduce as a side effect of one feature. This trait
//! keeps §3.1's real capability contract (the same conceptual methods,
//! streaming deltas) but is synchronous/blocking, with streaming delivered
//! via a callback (`&mut dyn FnMut(Delta)`) invoked once per real chunk as
//! it arrives over the HTTP response -- callers that want it off the render
//! thread spawn it the same way `gui_bridge::spawn_apply_edit_request`
//! (§75.42) already does.

use serde_json::Value;
use std::fmt;
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    /// A tool result being fed back to the model -- `tool_call_id` on the
    /// `Message` identifies which prior `ToolCallStart` it answers.
    Tool,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Set only on `Role::Tool` messages, matching the id a provider issued
    /// via `Delta::ToolCallStart` for the call this message answers.
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// A real JSON Schema description of one callable tool, matching §3.1's
/// `ToolDefinition` -- `parameters_schema` is passed through verbatim to
/// whichever provider's own native tool-calling format expects it (both
/// Ollama's and Anthropic's real APIs use plain JSON Schema for this).
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters_schema: Value,
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub system_prompt: String,
    pub max_tokens: usize,
    pub temperature: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    TextChunk(String),
    ToolCallStart {
        id: String,
        name: String,
    },
    /// A real, whole-tool-call JSON args payload, or a partial fragment of
    /// one -- named `partial_json` to match §3.1, but which real providers
    /// actually stream this incrementally varies (see `ollama.rs`'s own
    /// doc comment: Ollama's real API returns each tool call's arguments
    /// as one already-parsed JSON object per chunk, not incrementally
    /// streamed partial JSON text the way Anthropic's API does).
    ToolCallArgsChunk {
        id: String,
        partial_json: String,
    },
    ToolCallEnd {
        id: String,
    },
    Stop {
        reason: StopReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderHealth {
    Healthy,
    Unreachable,
    Unauthorized,
}

#[derive(Debug)]
pub enum ProviderError {
    Network(String),
    Http {
        status: u16,
        body: String,
    },
    Parse(String),
    /// A real error from a provider with no HTTP layer at all --
    /// `LlamaCppProvider`'s own in-process model load/tokenize/decode
    /// failures, none of which are meaningfully a "network," "HTTP," or
    /// "parse" error in the sense the other three variants mean.
    Local(String),
    /// Real §75.73-closing cooperative cancellation (task #269): a real,
    /// deliberate stop mid-stream via `stream_completion_cancellable`'s own
    /// cancel flag, not a genuine provider/network failure -- a caller that
    /// wants to distinguish "the user cancelled this" from "the provider
    /// actually broke" can match on this variant specifically rather than
    /// string-matching a `Display`ed message.
    Cancelled,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::Network(msg) => write!(f, "network error: {msg}"),
            ProviderError::Http { status, body } => {
                write!(f, "HTTP {status}: {body}")
            }
            ProviderError::Parse(msg) => write!(f, "parse error: {msg}"),
            ProviderError::Local(msg) => write!(f, "{msg}"),
            ProviderError::Cancelled => write!(f, "operation cancelled"),
        }
    }
}

impl std::error::Error for ProviderError {}

pub trait ModelProvider: Send + Sync {
    fn id(&self) -> &str;
    fn is_local(&self) -> bool;
    /// A real, live-queried context window where the provider's own API
    /// exposes one; implementations that can't query it return a
    /// documented static default rather than fabricating a number.
    fn context_window(&self) -> usize;
    fn supports_native_tool_calling(&self) -> bool;
    fn health_check(&self) -> ProviderHealth;

    /// Blocking call -- issues the real HTTP request and invokes `on_delta`
    /// once per real chunk as it arrives, returning once the provider signals
    /// the turn is over (or a real error occurs). See this module's own doc
    /// comment for why this is callback-based rather than `async`/`Stream`.
    fn stream_completion(
        &self,
        request: &CompletionRequest,
        on_delta: &mut dyn FnMut(Delta),
    ) -> Result<(), ProviderError>;

    /// Real §75.73-closing cooperative cancellation (task #269): identical
    /// contract to `stream_completion`, but a caller may set `cancel` to
    /// `true` (from another thread, e.g. a real user-initiated Leo cancel)
    /// to ask a real, possibly slow, already-in-flight streaming call to
    /// stop early with `ProviderError::Cancelled` instead of running to
    /// completion. This closes the exact gap `spartan-backend::leo_cancel`'s
    /// own doc comment has named since §75.73: cancel could only discard a
    /// late result via a generation counter, never actually stop the real
    /// background OS thread blocked on the model call itself.
    ///
    /// A real, honestly-scoped default: this base implementation ignores
    /// `cancel` entirely and simply delegates to `stream_completion` --
    /// genuine cooperative cancellation needs a provider whose own real
    /// streaming loop has a natural per-chunk checkpoint to check the flag
    /// at (a real NDJSON/SSE `for line in reader.lines()` loop, checked
    /// once per real line already received over the wire). `OllamaProvider`,
    /// `ClaudeProvider`, and `LiteLLMProvider` (and `LmStudioProvider`,
    /// which delegates straight through to its own inner `LiteLLMProvider`)
    /// override this for real; `LlamaCppProvider` does not -- its own real
    /// generation loop is in-process CPU/GPU token sampling with no network
    /// read to interleave a check into the same way, a real, deliberate,
    /// named v1 scope cut rather than an oversight (see its own module doc
    /// comment). A second, real, honestly-named limit that applies even to
    /// the providers that do override this: cancellation can only be
    /// observed *between* already-arrived chunks, never interrupt a single
    /// blocking read that's already waiting on the next one -- the same
    /// class of limit `crates/spartan-backend/src/subprocess.rs`'s own
    /// `wait_with_cancellation` (task #268) already carries for a real
    /// long-running child process, just for network I/O instead of a
    /// subprocess.
    fn stream_completion_cancellable(
        &self,
        request: &CompletionRequest,
        on_delta: &mut dyn FnMut(Delta),
        cancel: &AtomicBool,
    ) -> Result<(), ProviderError> {
        let _ = cancel;
        self.stream_completion(request, on_delta)
    }
}
