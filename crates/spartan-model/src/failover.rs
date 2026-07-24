//! Per-provider usage tracking + automatic failover -- a real `ModelProvider`
//! that wraps an ordered list of other providers and transparently falls
//! over to the next when one is unavailable or over its limit.
//!
//! **Concept origin, stated honestly**: this idea (track each provider's
//! usage; automatically switch to a fallback when a provider hits its API
//! limit) was adapted from the user's own earlier "Agent Deck" project
//! (`SpartanAI_IDE`) -- concept only, rebuilt fresh here against this
//! workspace's real `ModelProvider` trait, no code ported.
//!
//! **The failover rule is deliberately conservative and correct, not
//! optimistic**: a provider is only failed over when it errors *before
//! emitting any real output* (text or a tool call). That is exactly the
//! "provider is down / unauthorized / over quota" case a `429`/`401`/
//! connection error produces at request time. Once a provider has already
//! streamed real output to the caller, an error mid-stream is propagated
//! rather than silently restarting on a different provider -- restarting
//! would duplicate or corrupt already-delivered content. A real, named
//! limitation, not a bug.
//!
//! The delegating trait methods (`is_local`, `supports_native_tool_calling`,
//! `context_window`) are answered *conservatively* -- see each method -- so
//! a caller that relies on a capability never gets a wrapper that claims it
//! but might silently fail over to a provider lacking it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::provider::{CompletionRequest, Delta, ModelProvider, ProviderError, ProviderHealth};

/// Real per-provider usage counters. `*_chars` are character counts (a real,
/// cheap, provider-agnostic proxy for "how much text moved"), deliberately
/// **not** called "tokens" -- a true token count needs each provider's own
/// tokenizer, which the streaming API doesn't expose uniformly. Named for
/// what they actually are.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageStats {
    pub requests: u64,
    pub prompt_chars: u64,
    pub completion_chars: u64,
}

/// Thread-safe usage tracker keyed by provider id. Shared (`Arc`) so a UI
/// (e.g. a Settings "usage" panel, or Spartan Cloud's monitoring view) can
/// read a live snapshot while requests run on background threads.
#[derive(Debug, Default)]
pub struct UsageTracker {
    inner: Mutex<HashMap<String, UsageStats>>,
}

impl UsageTracker {
    pub fn new() -> Self {
        Self::default()
    }

    fn record_request(&self, provider_id: &str, prompt_chars: u64) {
        let mut map = self.inner.lock().expect("usage tracker mutex poisoned");
        let entry = map.entry(provider_id.to_string()).or_default();
        entry.requests += 1;
        entry.prompt_chars += prompt_chars;
    }

    fn record_completion(&self, provider_id: &str, completion_chars: u64) {
        let mut map = self.inner.lock().expect("usage tracker mutex poisoned");
        let entry = map.entry(provider_id.to_string()).or_default();
        entry.completion_chars += completion_chars;
    }

    /// A real point-in-time copy of every tracked provider's stats.
    pub fn snapshot(&self) -> HashMap<String, UsageStats> {
        self.inner
            .lock()
            .expect("usage tracker mutex poisoned")
            .clone()
    }

    /// Stats for one provider id, or a zeroed default if it's never been used.
    pub fn stats_for(&self, provider_id: &str) -> UsageStats {
        self.inner
            .lock()
            .expect("usage tracker mutex poisoned")
            .get(provider_id)
            .copied()
            .unwrap_or_default()
    }
}

/// A `ModelProvider` that tries an ordered chain of real providers, failing
/// over to the next when one is unavailable at request time. Records usage
/// for each attempt in a shared `UsageTracker`.
pub struct FailoverProvider {
    providers: Vec<Box<dyn ModelProvider>>,
    tracker: Arc<UsageTracker>,
}

impl FailoverProvider {
    /// Build a failover chain. `providers` is tried in order; it should be
    /// non-empty (an empty chain is handled gracefully -- never panics -- but
    /// can only ever error). Creates its own fresh `UsageTracker`.
    pub fn new(providers: Vec<Box<dyn ModelProvider>>) -> Self {
        Self {
            providers,
            tracker: Arc::new(UsageTracker::new()),
        }
    }

    /// Build a failover chain sharing an existing `UsageTracker` (so a UI can
    /// hold its own `Arc` clone and read live stats).
    pub fn with_tracker(
        providers: Vec<Box<dyn ModelProvider>>,
        tracker: Arc<UsageTracker>,
    ) -> Self {
        Self { providers, tracker }
    }

    /// A shared handle to the usage tracker for live reads.
    pub fn tracker(&self) -> Arc<UsageTracker> {
        Arc::clone(&self.tracker)
    }
}

/// A cheap, honest proxy for request size: the total characters across the
/// system prompt and every message. Not a token count (see `UsageStats`).
fn prompt_chars(request: &CompletionRequest) -> u64 {
    let mut total = request.system_prompt.len() as u64;
    for m in &request.messages {
        total += m.content.len() as u64;
    }
    total
}

impl ModelProvider for FailoverProvider {
    fn id(&self) -> &str {
        "failover"
    }

    /// Local only if **every** provider in the chain is local -- if any
    /// fallback is a cloud provider, a caller must treat this wrapper as
    /// non-local (a request could reach that cloud provider), matching this
    /// codebase's privacy posture (§9).
    fn is_local(&self) -> bool {
        !self.providers.is_empty() && self.providers.iter().all(|p| p.is_local())
    }

    /// The smallest context window in the chain -- the only bound that is
    /// safe no matter which provider actually serves the request.
    fn context_window(&self) -> usize {
        self.providers
            .iter()
            .map(|p| p.context_window())
            .min()
            .unwrap_or(0)
    }

    /// Claims native tool calling only if **every** provider supports it --
    /// otherwise a tool-calling caller could be silently failed over to a
    /// provider that can't honor the tools (see `llamacpp` before §75.84).
    fn supports_native_tool_calling(&self) -> bool {
        !self.providers.is_empty()
            && self
                .providers
                .iter()
                .all(|p| p.supports_native_tool_calling())
    }

    /// Healthy if any provider is healthy (that one would serve requests).
    /// If none is healthy, reports `Unauthorized` when *every* failure is an
    /// auth failure (an actionable "fix your keys" signal), else `Unreachable`.
    fn health_check(&self) -> ProviderHealth {
        let mut all_unauthorized = !self.providers.is_empty();
        for p in &self.providers {
            match p.health_check() {
                ProviderHealth::Healthy => return ProviderHealth::Healthy,
                ProviderHealth::Unauthorized => {}
                ProviderHealth::Unreachable => all_unauthorized = false,
            }
        }
        if all_unauthorized {
            ProviderHealth::Unauthorized
        } else {
            ProviderHealth::Unreachable
        }
    }

    fn stream_completion(
        &self,
        request: &CompletionRequest,
        on_delta: &mut dyn FnMut(Delta),
    ) -> Result<(), ProviderError> {
        self.stream_completion_cancellable(request, on_delta, &AtomicBool::new(false))
    }

    /// Real §75.73-closing cooperative cancellation (task #269): the same
    /// real per-provider failover logic `stream_completion` always had,
    /// but each attempt goes through the provider's own real
    /// `stream_completion_cancellable` (only `OllamaProvider`/
    /// `ClaudeProvider`/`LiteLLMProvider`/`LmStudioProvider` genuinely act
    /// on it; the rest fall back to their own default, matching
    /// `ModelProvider`'s own doc comment) -- plus a real check *between*
    /// providers, so a cancellation that fires while provider 1 is being
    /// tried doesn't then go on to try provider 2 as if nothing happened.
    fn stream_completion_cancellable(
        &self,
        request: &CompletionRequest,
        on_delta: &mut dyn FnMut(Delta),
        cancel: &AtomicBool,
    ) -> Result<(), ProviderError> {
        let chars = prompt_chars(request);
        let mut last_err: Option<ProviderError> = None;

        for provider in &self.providers {
            if cancel.load(Ordering::SeqCst) {
                return Err(ProviderError::Cancelled);
            }
            self.tracker.record_request(provider.id(), chars);
            let mut emitted_output = false;
            let mut completion_chars = 0u64;

            let result = {
                let mut wrapper = |delta: Delta| {
                    match &delta {
                        Delta::TextChunk(t) => {
                            completion_chars += t.len() as u64;
                            emitted_output = true;
                        }
                        Delta::ToolCallStart { .. }
                        | Delta::ToolCallArgsChunk { .. }
                        | Delta::ToolCallEnd { .. } => {
                            emitted_output = true;
                        }
                        // A terminal Stop is not "real output" -- it doesn't
                        // block a failover on its own.
                        Delta::Stop { .. } => {}
                    }
                    on_delta(delta);
                };
                provider.stream_completion_cancellable(request, &mut wrapper, cancel)
            };

            self.tracker
                .record_completion(provider.id(), completion_chars);

            match result {
                Ok(()) => return Ok(()),
                Err(ProviderError::Cancelled) => return Err(ProviderError::Cancelled),
                Err(e) => {
                    if emitted_output {
                        // Real output already reached the caller on this
                        // provider -- a mid-stream error can't be cleanly
                        // retried elsewhere, so surface it rather than
                        // duplicating content.
                        return Err(e);
                    }
                    // Failed before any output: a genuine "provider down /
                    // over limit" case -- fall over to the next provider.
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            ProviderError::Local("failover chain has no providers configured".to_string())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Delta, StopReason};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A configurable fake provider for exercising the failover logic without
    /// any network. `behavior` decides what it does when asked to stream.
    struct FakeProvider {
        id: String,
        local: bool,
        tools: bool,
        ctx: usize,
        health: ProviderHealth,
        behavior: Behavior,
        calls: Arc<AtomicUsize>,
    }

    #[derive(Clone)]
    enum Behavior {
        /// Emit a text chunk then succeed.
        SucceedWith(String),
        /// Error immediately, before any output.
        FailEarly,
        /// Emit a text chunk, THEN error (mid-stream failure).
        FailAfterOutput(String),
    }

    impl FakeProvider {
        fn new(id: &str, behavior: Behavior) -> Self {
            Self {
                id: id.to_string(),
                local: true,
                tools: true,
                ctx: 8192,
                health: ProviderHealth::Healthy,
                behavior,
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl ModelProvider for FakeProvider {
        fn id(&self) -> &str {
            &self.id
        }
        fn is_local(&self) -> bool {
            self.local
        }
        fn context_window(&self) -> usize {
            self.ctx
        }
        fn supports_native_tool_calling(&self) -> bool {
            self.tools
        }
        fn health_check(&self) -> ProviderHealth {
            self.health
        }
        fn stream_completion(
            &self,
            _request: &CompletionRequest,
            on_delta: &mut dyn FnMut(Delta),
        ) -> Result<(), ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.behavior {
                Behavior::SucceedWith(text) => {
                    on_delta(Delta::TextChunk(text.clone()));
                    on_delta(Delta::Stop {
                        reason: StopReason::EndTurn,
                    });
                    Ok(())
                }
                Behavior::FailEarly => Err(ProviderError::Http {
                    status: 429,
                    body: "rate limited".to_string(),
                }),
                Behavior::FailAfterOutput(text) => {
                    on_delta(Delta::TextChunk(text.clone()));
                    Err(ProviderError::Network("dropped mid-stream".to_string()))
                }
            }
        }
    }

    fn req() -> CompletionRequest {
        CompletionRequest {
            messages: vec![crate::provider::Message::user("hello there")],
            tools: vec![],
            system_prompt: "sys".to_string(),
            max_tokens: 64,
            temperature: 0.0,
        }
    }

    fn collect_text(provider: &dyn ModelProvider, request: &CompletionRequest) -> (String, bool) {
        let mut out = String::new();
        let mut on_delta = |d: Delta| {
            if let Delta::TextChunk(t) = d {
                out.push_str(&t);
            }
        };
        let ok = provider.stream_completion(request, &mut on_delta).is_ok();
        (out, ok)
    }

    #[test]
    fn fails_over_to_the_next_provider_when_the_first_errors_early() {
        let first = FakeProvider::new("primary", Behavior::FailEarly);
        let first_calls = Arc::clone(&first.calls);
        let second = FakeProvider::new("backup", Behavior::SucceedWith("from backup".to_string()));
        let second_calls = Arc::clone(&second.calls);

        let failover = FailoverProvider::new(vec![Box::new(first), Box::new(second)]);
        let (text, ok) = collect_text(&failover, &req());

        assert!(ok, "failover must succeed when a backup can serve");
        assert_eq!(text, "from backup");
        assert_eq!(first_calls.load(Ordering::SeqCst), 1, "primary was tried");
        assert_eq!(second_calls.load(Ordering::SeqCst), 1, "backup was used");
    }

    #[test]
    fn does_not_fail_over_after_real_output_was_already_emitted() {
        let first = FakeProvider::new("primary", Behavior::FailAfterOutput("partial".to_string()));
        let second = FakeProvider::new("backup", Behavior::SucceedWith("unused".to_string()));
        let second_calls = Arc::clone(&second.calls);

        let failover = FailoverProvider::new(vec![Box::new(first), Box::new(second)]);
        let (text, ok) = collect_text(&failover, &req());

        assert!(!ok, "a mid-stream error must be surfaced, not swallowed");
        assert_eq!(
            text, "partial",
            "the already-emitted output reached the caller"
        );
        assert_eq!(
            second_calls.load(Ordering::SeqCst),
            0,
            "the backup must NOT be tried once real output was emitted"
        );
    }

    #[test]
    fn errors_when_every_provider_fails_early() {
        let first = FakeProvider::new("a", Behavior::FailEarly);
        let second = FakeProvider::new("b", Behavior::FailEarly);
        let failover = FailoverProvider::new(vec![Box::new(first), Box::new(second)]);
        let (_text, ok) = collect_text(&failover, &req());
        assert!(!ok, "all-fail must surface an error");
    }

    #[test]
    fn tracks_usage_per_provider() {
        let first = FakeProvider::new("primary", Behavior::FailEarly);
        let second = FakeProvider::new("backup", Behavior::SucceedWith("hello world".to_string()));
        let failover = FailoverProvider::new(vec![Box::new(first), Box::new(second)]);
        let tracker = failover.tracker();

        let _ = collect_text(&failover, &req());

        let primary = tracker.stats_for("primary");
        let backup = tracker.stats_for("backup");
        assert_eq!(primary.requests, 1, "primary got one recorded request");
        assert_eq!(primary.completion_chars, 0, "primary emitted nothing");
        assert_eq!(backup.requests, 1);
        assert_eq!(
            backup.completion_chars,
            "hello world".len() as u64,
            "backup's real completion size was recorded"
        );
        assert!(primary.prompt_chars > 0, "prompt size was recorded");
    }

    #[test]
    fn delegating_methods_are_conservative() {
        // is_local: false if ANY provider is non-local.
        let mut cloud = FakeProvider::new("cloud", Behavior::FailEarly);
        cloud.local = false;
        let local = FakeProvider::new("local", Behavior::FailEarly);
        let mixed = FailoverProvider::new(vec![Box::new(cloud), Box::new(local)]);
        assert!(
            !mixed.is_local(),
            "a chain with a cloud provider is not local"
        );

        // supports tools: false if ANY provider lacks it.
        let mut no_tools = FakeProvider::new("no-tools", Behavior::FailEarly);
        no_tools.tools = false;
        let with_tools = FakeProvider::new("tools", Behavior::FailEarly);
        let chain = FailoverProvider::new(vec![Box::new(no_tools), Box::new(with_tools)]);
        assert!(
            !chain.supports_native_tool_calling(),
            "tool support requires ALL providers to support it"
        );

        // context_window: the minimum in the chain.
        let mut small = FakeProvider::new("small", Behavior::FailEarly);
        small.ctx = 4096;
        let mut large = FakeProvider::new("large", Behavior::FailEarly);
        large.ctx = 128_000;
        let chain = FailoverProvider::new(vec![Box::new(large), Box::new(small)]);
        assert_eq!(
            chain.context_window(),
            4096,
            "the safe bound is the minimum"
        );
    }

    #[test]
    fn a_pre_cancelled_flag_stops_the_chain_before_trying_the_first_provider() {
        let first = FakeProvider::new("primary", Behavior::SucceedWith("unused".to_string()));
        let first_calls = Arc::clone(&first.calls);
        let failover = FailoverProvider::new(vec![Box::new(first)]);
        let cancel_flag = std::sync::atomic::AtomicBool::new(true);
        let result = failover.stream_completion_cancellable(&req(), &mut |_| {}, &cancel_flag);
        assert!(
            matches!(result, Err(ProviderError::Cancelled)),
            "an already-cancelled flag must stop the chain immediately: {result:?}"
        );
        assert_eq!(
            first_calls.load(Ordering::SeqCst),
            0,
            "a provider already-cancelled before the chain even starts must never be tried"
        );
    }

    #[test]
    fn cancellation_stops_the_chain_even_after_real_output_was_emitted() {
        // Real, deliberate difference from a genuine mid-stream *error*
        // (`does_not_fail_over_after_real_output_was_already_emitted`
        // above): a real cancellation must never fail over to try a second
        // provider just because the first one had already streamed some
        // real output before the cancel flag flipped -- the whole chain
        // stops, matching a real user's "no, stop entirely" intent.
        struct CancellingProvider {
            calls: Arc<AtomicUsize>,
        }
        impl ModelProvider for CancellingProvider {
            fn id(&self) -> &str {
                "cancelling"
            }
            fn is_local(&self) -> bool {
                true
            }
            fn context_window(&self) -> usize {
                4096
            }
            fn supports_native_tool_calling(&self) -> bool {
                true
            }
            fn health_check(&self) -> ProviderHealth {
                ProviderHealth::Healthy
            }
            fn stream_completion(
                &self,
                _request: &CompletionRequest,
                _on_delta: &mut dyn FnMut(Delta),
            ) -> Result<(), ProviderError> {
                unreachable!("this test always calls the cancellable path")
            }
            fn stream_completion_cancellable(
                &self,
                _request: &CompletionRequest,
                on_delta: &mut dyn FnMut(Delta),
                _cancel: &std::sync::atomic::AtomicBool,
            ) -> Result<(), ProviderError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                on_delta(Delta::TextChunk("partial output".to_string()));
                Err(ProviderError::Cancelled)
            }
        }

        let primary_calls = Arc::new(AtomicUsize::new(0));
        let primary = CancellingProvider {
            calls: Arc::clone(&primary_calls),
        };
        let backup = FakeProvider::new("backup", Behavior::SucceedWith("unused".to_string()));
        let backup_calls = Arc::clone(&backup.calls);
        let failover = FailoverProvider::new(vec![Box::new(primary), Box::new(backup)]);

        let mut text = String::new();
        let cancel_flag = std::sync::atomic::AtomicBool::new(false);
        let result = failover.stream_completion_cancellable(
            &req(),
            &mut |d| {
                if let Delta::TextChunk(t) = d {
                    text.push_str(&t);
                }
            },
            &cancel_flag,
        );

        assert!(matches!(result, Err(ProviderError::Cancelled)));
        assert_eq!(text, "partial output");
        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            backup_calls.load(Ordering::SeqCst),
            0,
            "a real cancellation must not fail over to a second provider"
        );
    }

    #[test]
    fn health_is_healthy_if_any_provider_is_healthy() {
        let mut down = FakeProvider::new("down", Behavior::FailEarly);
        down.health = ProviderHealth::Unreachable;
        let up = FakeProvider::new("up", Behavior::FailEarly); // Healthy by default
        let chain = FailoverProvider::new(vec![Box::new(down), Box::new(up)]);
        assert_eq!(chain.health_check(), ProviderHealth::Healthy);

        // All unauthorized -> Unauthorized (actionable signal).
        let mut a = FakeProvider::new("a", Behavior::FailEarly);
        a.health = ProviderHealth::Unauthorized;
        let mut b = FakeProvider::new("b", Behavior::FailEarly);
        b.health = ProviderHealth::Unauthorized;
        let chain = FailoverProvider::new(vec![Box::new(a), Box::new(b)]);
        assert_eq!(chain.health_check(), ProviderHealth::Unauthorized);
    }
}
