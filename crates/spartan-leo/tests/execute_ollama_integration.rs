//! Real, executed integration test against a real local Ollama instance
//! (task #5) -- the execute-step analogue of `plan_ollama_integration.rs`.
//! Proves the real pipeline `execute.rs`'s own unit tests only exercise
//! with an in-process fake: a real model, given a real approved plan,
//! producing a real tool call (or `task_complete`) via
//! `OllamaProvider`'s already-live-proven native tool-calling path
//! (§75.43). Self-skips if Ollama isn't reachable or the model isn't
//! pulled, matching every other real-external-tool integration test in
//! this repo.

use spartan_leo::execute::next_action;
use spartan_leo::plan::ImplementationPlan;
use spartan_model::{ModelProvider, OllamaProvider, ProviderHealth};

const MODEL: &str = "llama3.1:8b";

fn model_available() -> bool {
    let provider = OllamaProvider::local(MODEL);
    provider.health_check() == ProviderHealth::Healthy && provider.context_window() > 0
}

#[test]
fn real_ollama_produces_a_real_next_action_for_an_approved_plan() {
    if !model_available() {
        eprintln!("SKIP: Ollama not reachable or {MODEL} not pulled");
        return;
    }
    let provider = OllamaProvider::local(MODEL);
    let plan = ImplementationPlan {
        goal: "Add a function called `add` to src/math.rs".to_string(),
        approach: "Create src/math.rs with a public `add(a: i32, b: i32) -> i32` function"
            .to_string(),
        files: vec!["src/math.rs".to_string()],
        risk_notes: "Low risk, new file only".to_string(),
    };

    let step = next_action(&provider, &plan, &[])
        .expect("a real model with native tool support should propose a real next action");

    println!("real execute step from {MODEL}: {step:?}");
    // A real model given this plan should call a real tool (most likely
    // edit_file to create src/math.rs) rather than immediately claiming
    // completion with zero actions taken -- a loose but real, meaningful
    // assertion about real model behavior, not just that parsing succeeded.
    match &step.action {
        spartan_leo::execute::ExecuteAction::Call(_) => {}
        spartan_leo::execute::ExecuteAction::Done { summary } => {
            panic!("expected a real tool call before task_complete, got Done({summary:?})")
        }
    }
}
