//! Real `LmStudioProvider` (§57) -- a fifth `ModelProvider`, for LM Studio's
//! own local server. LM Studio exposes the exact same OpenAI-compatible
//! `/v1/chat/completions` + `/v1/models` wire format that `LiteLLMProvider`
//! already speaks, so this provider is built by **composition**, not
//! duplication: it wraps a `LiteLLMProvider` pointed at LM Studio's documented
//! default (`http://localhost:1234`) and delegates the real SSE streaming and
//! request-body construction to it verbatim.
//!
//! The one load-bearing difference is deliberate and captured here rather than
//! copied wrong: **`is_local()` returns `true`**. Unlike LiteLLM (a proxy whose
//! whole purpose is fanning out to real *cloud* backends, so its own
//! `is_local()` is correctly `false`, §44.1), LM Studio runs the model itself,
//! in-process on this machine -- so a §9/§44.3 "is this call's data staying on
//! this machine" privacy rule must treat it as local, exactly like
//! `OllamaProvider`. `health_check()` is also overridden to hit LM Studio's own
//! `/v1/models` (it has no LiteLLM-proxy `/health/liveliness` endpoint).
//!
//! **Honesty note**: this provider is structurally complete and its non-network
//! behavior is unit-tested, but it has **not** been live-verified against a real
//! running LM Studio instance in this project's environment (none is installed
//! here) -- the same honest status `ClaudeProvider` carries for its own live
//! path. Its streaming/parsing is the identical, live-confirmed
//! `LiteLLMProvider` code, so the wire handling is proven; only the LM-Studio-
//! specific base URL + `/v1/models` health probe are unexercised live.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::litellm::LiteLLMProvider;
use crate::provider::{CompletionRequest, Delta, ModelProvider, ProviderError, ProviderHealth};

pub struct LmStudioProvider {
    inner: LiteLLMProvider,
    base_url: String,
    model: String,
}

impl LmStudioProvider {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        let base_url = base_url.into();
        let model = model.into();
        Self {
            inner: LiteLLMProvider::new(base_url.clone(), model.clone()),
            base_url,
            model,
        }
    }

    /// The real common case: a local LM Studio server on its own documented
    /// default port (1234), serving whichever model LM Studio itself has
    /// loaded/selected.
    pub fn local(model: impl Into<String>) -> Self {
        Self::new("http://localhost:1234", model)
    }
}

impl ModelProvider for LmStudioProvider {
    fn id(&self) -> &str {
        &self.model
    }

    /// LM Studio runs the model in-process on this machine -- a real local
    /// runtime and privacy boundary, unlike LiteLLM's cloud-fanning proxy.
    fn is_local(&self) -> bool {
        true
    }

    fn context_window(&self) -> usize {
        // Same honest fallback LiteLLM uses; LM Studio does expose per-model
        // metadata via /v1/models, but querying it is separate follow-on work.
        self.inner.context_window()
    }

    fn supports_native_tool_calling(&self) -> bool {
        // Delegated: LM Studio supports OpenAI-format tool calling for models
        // that implement it, the same assumption LiteLLM makes.
        self.inner.supports_native_tool_calling()
    }

    fn health_check(&self) -> ProviderHealth {
        // LM Studio has no LiteLLM `/health/liveliness`; its OpenAI-compatible
        // `/v1/models` is the real liveness probe.
        match ureq::get(&format!("{}/v1/models", self.base_url))
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
        // Verbatim OpenAI-compatible SSE streaming -- the identical,
        // live-confirmed LiteLLMProvider path.
        self.inner.stream_completion(request, on_delta)
    }

    /// Real §75.73-closing cooperative cancellation (task #269) -- delegates
    /// straight through to the inner `LiteLLMProvider`'s own real
    /// cancellable path, matching how every other method on this provider
    /// is composed rather than duplicated.
    fn stream_completion_cancellable(
        &self,
        request: &CompletionRequest,
        on_delta: &mut dyn FnMut(Delta),
        cancel: &AtomicBool,
    ) -> Result<(), ProviderError> {
        self.inner
            .stream_completion_cancellable(request, on_delta, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_points_at_lm_studios_default_port_and_reports_local() {
        let p = LmStudioProvider::local("some-model");
        assert_eq!(p.id(), "some-model");
        assert!(
            p.is_local(),
            "LM Studio runs the model on-device -- it is a real local runtime"
        );
        assert!(p.base_url.contains("localhost:1234"));
    }

    #[test]
    fn health_check_is_unreachable_when_no_server_is_running() {
        // Nothing is listening on this port in the test environment, so the
        // real probe must report Unreachable, not panic.
        let p = LmStudioProvider::new("http://127.0.0.1:1", "m");
        assert_eq!(p.health_check(), ProviderHealth::Unreachable);
    }

    #[test]
    fn delegated_capability_answers_match_the_inner_openai_shape() {
        let p = LmStudioProvider::local("m");
        // These delegate to the shared OpenAI-compatible provider.
        assert_eq!(p.context_window(), 4096);
        assert!(p.supports_native_tool_calling());
    }
}
