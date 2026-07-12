//! Real §4.1 plan generation (task #5): "Leo produces an
//! `ImplementationPlan` artifact (structured: goal, approach, file list,
//! risk notes) before any file write. This is a real data object, not
//! just chat text." Uses the real `ModelProvider` trait (§75.43) --
//! deliberately drives the model's real native tool-calling path (proven
//! live against `llama3.1:8b` in §75.43's own `ollama_integration.rs`)
//! rather than free-text parsing: the plan is requested as a single,
//! required tool call with a fixed JSON Schema, so a syntactically valid
//! response is a real, structured `ImplementationPlan`, not prose this
//! module would need to guess how to parse.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use spartan_model::provider::{CompletionRequest, Delta, Message, ModelProvider, ToolDefinition};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImplementationPlan {
    pub goal: String,
    pub approach: String,
    #[serde(deserialize_with = "deserialize_files")]
    pub files: Vec<String>,
    pub risk_notes: String,
}

/// Real, live-found model-output quirk (§75.46): a real `llama3.1:8b`
/// tool call, on one real run out of several, returned `files` as a
/// JSON-encoded *string* containing an array literal
/// (`"[\"src/math.rs\"]"`) rather than a real JSON array -- a known real
/// failure mode of smaller/quantized local models generating structured
/// tool-call arguments, not a Spartan bug (confirmed by real, live
/// integration test runs, not assumed -- three distinct real shapes
/// observed across repeated live runs against the same real model: a
/// real array, a JSON-encoded string containing one, and a
/// Python-repr-style string using single quotes instead of JSON's
/// required double quotes, e.g. `"['src/math.rs']"`). `serde_json`
/// correctly rejected all three outright before this fix, matching
/// §3.4's own "must be surfaced, never silently dropped" -- this
/// deserializer keeps that same strictness for anything that isn't one
/// of the real, observed shapes, while accepting all three rather than
/// making every real user hit the same live failures these test runs
/// did.
fn deserialize_files<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum FilesField {
        Array(Vec<String>),
        JsonEncodedString(String),
    }
    let raw = match FilesField::deserialize(deserializer)? {
        FilesField::Array(files) => return Ok(files),
        FilesField::JsonEncodedString(s) => s,
    };
    if let Ok(files) = serde_json::from_str::<Vec<String>>(&raw) {
        return Ok(files);
    }
    // Real, live-found fallback: a Python-repr-style list
    // (`['a.rs', 'b.rs']`) is not valid JSON (single quotes aren't
    // legal string delimiters), but swapping quote characters is a
    // well-known, pragmatic recovery for this specific, real,
    // repeatedly-observed shape -- attempted only after strict JSON
    // parsing has already failed, never instead of it.
    let single_to_double_quoted = raw.replace('\'', "\"");
    serde_json::from_str(&single_to_double_quoted).map_err(|e| {
        serde::de::Error::custom(format!(
            "'files' was a string but not a real JSON array or Python-repr-style list: {e}"
        ))
    })
}

#[derive(Debug, Clone)]
pub enum PlanError {
    Provider(String),
    /// The model responded but never made the real `propose_plan` tool
    /// call at all -- surfaced distinctly from a malformed one, matching
    /// §3.4's "never silently drops it or guesses" rule.
    NoPlanProposed,
    /// A real tool call happened, but its arguments didn't parse into a
    /// real `ImplementationPlan` -- the raw JSON is kept for a caller to
    /// show the user, not discarded.
    MalformedPlan {
        raw: String,
        reason: String,
    },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::Provider(msg) => write!(f, "model provider error: {msg}"),
            PlanError::NoPlanProposed => {
                write!(f, "the model never called propose_plan")
            }
            PlanError::MalformedPlan { raw, reason } => {
                write!(f, "malformed plan ({reason}): {raw}")
            }
        }
    }
}

impl std::error::Error for PlanError {}

const PLAN_TOOL_NAME: &str = "propose_plan";

fn plan_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: PLAN_TOOL_NAME.to_string(),
        description: "Propose a structured implementation plan for the requested task. \
            Always call this exactly once before any other action."
            .to_string(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "goal": {"type": "string", "description": "One-sentence statement of what this plan accomplishes"},
                "approach": {"type": "string", "description": "How the goal will be accomplished, in prose"},
                "files": {"type": "array", "items": {"type": "string"}, "description": "Real file paths this plan expects to touch"},
                "risk_notes": {"type": "string", "description": "Anything risky or uncertain about this plan"}
            },
            "required": ["goal", "approach", "files", "risk_notes"]
        }),
    }
}

fn system_prompt() -> String {
    format!(
        "You are Leo, a coding agent embedded in the Spartan IDE. \
         Before taking any action, you must call the propose_plan tool exactly once with a \
         real, concrete plan for the user's request. Do not call any other tool until the \
         plan is approved.\n\n{}",
        crate::persona::LEO_PERSONA,
    )
}

/// Real, live-found need (§75.46): a real `llama3.1:8b` call occasionally
/// returns a `files` value serde can't parse even after
/// `deserialize_files`'s own real fallback handling (a third or later
/// malformed shape neither of the two already-fixed real ones covers) --
/// confirmed by repeated real live runs, not assumed. Rather than chase
/// an unbounded number of real, specific malformed shapes one at a time,
/// `generate_plan` retries the *whole model call* a bounded number of
/// times on a bad response, matching how a real, thoughtful agent
/// implementation should behave anyway (this is not solely a test-fixing
/// measure) -- §3.4's own "local-model-driven agent runs default to
/// manual-approve-every-step... since tool-call fidelity is lower"
/// already acknowledges real local models don't always get structured
/// output right on the first try. Only retries `NoPlanProposed`/
/// `MalformedPlan` (the model responded but the response itself was bad);
/// a real `Provider` error (network/auth/server failure) is returned
/// immediately, since retrying identically is unlikely to help and
/// would just slow down surfacing a real, actionable failure.
const MAX_PLAN_ATTEMPTS: u32 = 3;

pub fn generate_plan(
    provider: &dyn ModelProvider,
    task: &str,
) -> Result<ImplementationPlan, PlanError> {
    let mut last_error = PlanError::NoPlanProposed;
    for _ in 0..MAX_PLAN_ATTEMPTS {
        match generate_plan_once(provider, task) {
            Ok(plan) => return Ok(plan),
            Err(e @ PlanError::Provider(_)) => return Err(e),
            Err(e) => last_error = e,
        }
    }
    Err(last_error)
}

fn generate_plan_once(
    provider: &dyn ModelProvider,
    task: &str,
) -> Result<ImplementationPlan, PlanError> {
    let request = CompletionRequest {
        messages: vec![Message::user(task)],
        tools: vec![plan_tool_definition()],
        system_prompt: system_prompt(),
        max_tokens: 1024,
        temperature: 0.2,
    };

    let mut plan_args = String::new();
    let mut saw_plan_call = false;
    let mut current_call_name = String::new();

    provider
        .stream_completion(&request, &mut |delta| match delta {
            Delta::ToolCallStart { name, .. } => {
                current_call_name = name;
            }
            Delta::ToolCallArgsChunk { partial_json, .. } => {
                if current_call_name == PLAN_TOOL_NAME {
                    saw_plan_call = true;
                    plan_args.push_str(&partial_json);
                }
            }
            Delta::ToolCallEnd { .. } | Delta::TextChunk(_) | Delta::Stop { .. } => {}
        })
        .map_err(|e| PlanError::Provider(e.to_string()))?;

    if !saw_plan_call {
        return Err(PlanError::NoPlanProposed);
    }

    serde_json::from_str::<ImplementationPlan>(&plan_args).map_err(|e| PlanError::MalformedPlan {
        raw: plan_args,
        reason: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use spartan_model::provider::{ProviderError, ProviderHealth, StopReason};

    /// A real, in-process fake `ModelProvider` -- not a live model call
    /// (that's `plan_kotlin... no, `plan_ollama_integration.rs`'s job) --
    /// exercises `generate_plan`'s own parsing/error-handling logic in
    /// isolation, matching this crate's own established "unit-test the
    /// logic, integration-test the live call separately" split.
    struct FakeProvider {
        tool_call_args: Option<String>,
    }

    impl ModelProvider for FakeProvider {
        fn id(&self) -> &str {
            "fake"
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
            on_delta: &mut dyn FnMut(Delta),
        ) -> Result<(), ProviderError> {
            if let Some(args) = &self.tool_call_args {
                on_delta(Delta::ToolCallStart {
                    id: "call_1".to_string(),
                    name: PLAN_TOOL_NAME.to_string(),
                });
                on_delta(Delta::ToolCallArgsChunk {
                    id: "call_1".to_string(),
                    partial_json: args.clone(),
                });
                on_delta(Delta::ToolCallEnd {
                    id: "call_1".to_string(),
                });
                on_delta(Delta::Stop {
                    reason: StopReason::ToolUse,
                });
            } else {
                on_delta(Delta::TextChunk("I won't make a plan.".to_string()));
                on_delta(Delta::Stop {
                    reason: StopReason::EndTurn,
                });
            }
            Ok(())
        }
    }

    #[test]
    fn a_real_valid_tool_call_parses_into_a_real_plan() {
        let provider = FakeProvider {
            tool_call_args: Some(
                json!({
                    "goal": "Add dark mode",
                    "approach": "Add a theme toggle and CSS variables",
                    "files": ["src/theme.rs", "src/main.rs"],
                    "risk_notes": "Low risk, additive only"
                })
                .to_string(),
            ),
        };
        let plan = generate_plan(&provider, "add dark mode").unwrap();
        assert_eq!(plan.goal, "Add dark mode");
        assert_eq!(plan.files, vec!["src/theme.rs", "src/main.rs"]);
    }

    #[test]
    fn no_tool_call_is_a_real_distinct_error() {
        let provider = FakeProvider {
            tool_call_args: None,
        };
        let result = generate_plan(&provider, "add dark mode");
        assert!(matches!(result, Err(PlanError::NoPlanProposed)));
    }

    #[test]
    fn malformed_tool_call_args_are_surfaced_not_silently_dropped() {
        let provider = FakeProvider {
            tool_call_args: Some("{not valid json".to_string()),
        };
        let result = generate_plan(&provider, "add dark mode");
        match result {
            Err(PlanError::MalformedPlan { raw, .. }) => {
                assert_eq!(raw, "{not valid json");
            }
            other => panic!("expected MalformedPlan, got {other:?}"),
        }
    }

    #[test]
    fn missing_required_field_is_a_real_malformed_plan_not_a_panic() {
        let provider = FakeProvider {
            tool_call_args: Some(json!({"goal": "only a goal"}).to_string()),
        };
        let result = generate_plan(&provider, "add dark mode");
        assert!(matches!(result, Err(PlanError::MalformedPlan { .. })));
    }

    /// Real regression test for a real bug a live Ollama integration test
    /// caught (§75.46): `llama3.1:8b` sometimes returns `files` as a
    /// JSON-encoded string instead of a real array.
    #[test]
    fn a_real_json_encoded_string_files_field_is_accepted_like_a_real_array() {
        let provider = FakeProvider {
            tool_call_args: Some(
                json!({
                    "goal": "Add a function",
                    "approach": "Write it",
                    "files": "[\"src/math.rs\"]",
                    "risk_notes": "none"
                })
                .to_string(),
            ),
        };
        let plan = generate_plan(&provider, "add a function").unwrap();
        assert_eq!(plan.files, vec!["src/math.rs".to_string()]);
    }

    #[test]
    fn a_files_field_that_is_a_string_but_not_valid_json_is_still_a_real_malformed_plan() {
        let provider = FakeProvider {
            tool_call_args: Some(
                json!({
                    "goal": "Add a function",
                    "approach": "Write it",
                    "files": "not json at all",
                    "risk_notes": "none"
                })
                .to_string(),
            ),
        };
        let result = generate_plan(&provider, "add a function");
        assert!(matches!(result, Err(PlanError::MalformedPlan { .. })));
    }

    /// Real regression test for a second real live-observed shape
    /// (§75.46): `llama3.1:8b` also sometimes returns `files` as a
    /// Python-repr-style string (single-quoted, not valid JSON).
    #[test]
    fn a_real_python_repr_style_files_string_is_accepted() {
        let provider = FakeProvider {
            tool_call_args: Some(
                json!({
                    "goal": "Add a function",
                    "approach": "Write it",
                    "files": "['src/math.rs', 'src/lib.rs']",
                    "risk_notes": "none"
                })
                .to_string(),
            ),
        };
        let plan = generate_plan(&provider, "add a function").unwrap();
        assert_eq!(
            plan.files,
            vec!["src/math.rs".to_string(), "src/lib.rs".to_string()]
        );
    }

    /// A real fake returning a different, scripted response each call --
    /// exercises `generate_plan`'s real bounded-retry loop (§75.46), not
    /// just its single-call parsing logic.
    struct SequencedFakeProvider {
        responses: std::sync::Mutex<std::vec::IntoIter<Option<String>>>,
    }

    impl SequencedFakeProvider {
        fn new(responses: Vec<Option<String>>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses.into_iter()),
            }
        }
    }

    impl ModelProvider for SequencedFakeProvider {
        fn id(&self) -> &str {
            "fake-sequenced"
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
            on_delta: &mut dyn FnMut(Delta),
        ) -> Result<(), ProviderError> {
            let next = self.responses.lock().unwrap().next().flatten();
            if let Some(args) = next {
                on_delta(Delta::ToolCallStart {
                    id: "1".to_string(),
                    name: PLAN_TOOL_NAME.to_string(),
                });
                on_delta(Delta::ToolCallArgsChunk {
                    id: "1".to_string(),
                    partial_json: args,
                });
                on_delta(Delta::ToolCallEnd {
                    id: "1".to_string(),
                });
                on_delta(Delta::Stop {
                    reason: StopReason::ToolUse,
                });
            } else {
                on_delta(Delta::TextChunk("no tool call this time".to_string()));
                on_delta(Delta::Stop {
                    reason: StopReason::EndTurn,
                });
            }
            Ok(())
        }
    }

    #[test]
    fn a_malformed_first_response_is_retried_and_a_later_good_one_succeeds() {
        let valid = json!({
            "goal": "g", "approach": "a", "files": ["x.rs"], "risk_notes": "none"
        })
        .to_string();
        let provider =
            SequencedFakeProvider::new(vec![Some("{not valid json".to_string()), Some(valid)]);
        let plan = generate_plan(&provider, "task").unwrap();
        assert_eq!(plan.files, vec!["x.rs".to_string()]);
    }

    #[test]
    fn retries_are_really_bounded_and_the_final_error_is_still_surfaced() {
        let provider = SequencedFakeProvider::new(vec![
            Some("{not valid json".to_string()),
            Some("{also not valid".to_string()),
            Some("{still not valid".to_string()),
        ]);
        let result = generate_plan(&provider, "task");
        assert!(matches!(result, Err(PlanError::MalformedPlan { .. })));
    }

    /// Real §75.95 check: the sarcastic-persona instruction is genuinely
    /// present in the real system prompt sent with every plan request, not
    /// just written in `persona.rs` and never actually wired in.
    #[test]
    fn the_real_system_prompt_carries_the_real_leo_persona() {
        assert!(system_prompt().contains(crate::persona::LEO_PERSONA));
    }

    /// The structural instruction (call propose_plan exactly once) must
    /// still be present alongside the persona -- the persona is layered
    /// on top of the real functional contract, never replacing it.
    #[test]
    fn the_real_system_prompt_still_requires_calling_propose_plan() {
        assert!(system_prompt().contains("propose_plan"));
    }
}
