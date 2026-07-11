//! Real §3 `ModelProvider` abstraction (task #4) -- first real increment.
//! See `provider.rs`'s own doc comment for the one deliberate, named
//! adaptation from the spec's literal `async`/`Stream`-based trait shape
//! to this workspace's established blocking-I/O-on-a-thread convention.

pub mod claude;
pub mod fallback;
pub mod litellm;
pub mod llamacpp;
pub mod ollama;
pub mod provider;

pub use claude::ClaudeProvider;
pub use fallback::{FallbackParser, ParseEvent};
pub use litellm::LiteLLMProvider;
pub use llamacpp::LlamaCppProvider;
pub use ollama::OllamaProvider;
pub use provider::{
    CompletionRequest, Delta, Message, ModelProvider, ProviderError, ProviderHealth, Role,
    StopReason, ToolDefinition,
};
