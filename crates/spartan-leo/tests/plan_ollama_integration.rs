//! Real, executed integration test against a real local Ollama instance
//! (task #5). Proves the whole real pipeline `plan.rs`'s own unit tests
//! only exercise with an in-process fake: a real model, driven through
//! `OllamaProvider`'s already-live-proven native tool-calling path
//! (§75.43), producing a real `propose_plan` tool call this crate then
//! parses into a real `ImplementationPlan`. Self-skips (rather than
//! fails) if Ollama isn't reachable or the model isn't pulled, matching
//! `spartan-model`'s own `ollama_integration.rs` convention.

use spartan_leo::plan::generate_plan;
use spartan_model::{ModelProvider, OllamaProvider, ProviderHealth};

const MODEL: &str = "llama3.1:8b";

fn model_available() -> bool {
    let provider = OllamaProvider::local(MODEL);
    provider.health_check() == ProviderHealth::Healthy && provider.context_window() > 0
}

#[test]
fn real_ollama_produces_a_real_parseable_plan() {
    if !model_available() {
        eprintln!("SKIP: Ollama not reachable or {MODEL} not pulled");
        return;
    }
    let provider = OllamaProvider::local(MODEL);
    let plan = generate_plan(
        &provider,
        "Add a function called `add` to src/math.rs that adds two integers and returns the result.",
    )
    .expect("a real model with native tool support should produce a real, parseable plan");

    assert!(
        !plan.goal.is_empty(),
        "a real plan should have a real, non-empty goal"
    );
    assert!(
        !plan.approach.is_empty(),
        "a real plan should have a real, non-empty approach"
    );
    println!("real plan from {MODEL}: {plan:?}");
}
