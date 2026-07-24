//! Real §4.1 execute-step model round trip (task #5) -- turns an approved
//! `ImplementationPlan` into real, concrete tool calls, closing the exact
//! gap CLAUDE.md's own §75.47 named as the single largest remaining piece
//! of Leo's real plan -> approve -> execute -> verify loop: "approving a
//! plan creates a real checkpoint and then has nothing further to run --
//! spartan-leo has no model-facing step yet that turns an approved plan
//! into concrete tool calls." `agent.rs`'s `execute_call`/
//! `begin_verification`/`run_verification` have always been real and
//! tested against manually-constructed `ToolCall`s (see
//! `real_full_happy_path_plan_approve_execute_verify_done`); this module
//! is the missing piece that actually produces those calls from a real
//! model, one at a time, driven by an accumulating conversation history --
//! mirrors `plan.rs`'s own native-tool-calling approach (a fixed JSON
//! Schema per real tool, not free-text parsing) rather than inventing a
//! second parsing strategy.

use crate::plan::ImplementationPlan;
use crate::tool::ToolCall;
use serde_json::{json, Value};
use spartan_model::provider::{
    CompletionRequest, Delta, Message, ModelProvider, Role, ToolDefinition,
};
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone)]
pub enum ExecuteAction {
    Call(ToolCall),
    /// The model considers the plan fully executed -- `summary` is its own
    /// real, model-authored account of what it did, surfaced to the user
    /// rather than discarded.
    Done {
        summary: String,
    },
}

/// One real model round trip's result -- `call_id` is the real id the
/// provider issued for this tool call (via `Delta::ToolCallStart`),
/// needed so a caller can reply with a correctly-addressed `Role::Tool`
/// message once the call has actually run.
#[derive(Debug, Clone)]
pub struct ExecuteStep {
    pub call_id: String,
    pub action: ExecuteAction,
}

#[derive(Debug)]
pub enum ExecuteError {
    Provider(String),
    /// The model responded but called none of the four real tools this
    /// module defines -- surfaced distinctly, matching `PlanError::
    /// NoPlanProposed`'s own "never silently drops it or guesses" rule.
    NoActionProposed,
    MalformedCall {
        tool: String,
        raw: String,
        reason: String,
    },
}

impl std::fmt::Display for ExecuteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecuteError::Provider(msg) => write!(f, "model provider error: {msg}"),
            ExecuteError::NoActionProposed => {
                write!(f, "the model never called a real tool or task_complete")
            }
            ExecuteError::MalformedCall { tool, raw, reason } => {
                write!(f, "malformed {tool} call ({reason}): {raw}")
            }
        }
    }
}

impl std::error::Error for ExecuteError {}

const READ_FILE_TOOL: &str = "read_file";
const EDIT_FILE_TOOL: &str = "edit_file";
const RUN_TERMINAL_TOOL: &str = "run_terminal";
const SEARCH_FILES_TOOL: &str = "search_files";
const LIST_DIRECTORY_TOOL: &str = "list_directory";
const TASK_COMPLETE_TOOL: &str = "task_complete";

fn execute_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: READ_FILE_TOOL.to_string(),
            description: "Read a real file's contents, path relative to the project root."
                .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: EDIT_FILE_TOOL.to_string(),
            description: "Write real, complete file content to a path relative to the \
                project root, creating the file (and any parent directories) if it \
                doesn't exist yet. Always writes the *entire* file, not a diff."
                .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path", "content"]
            }),
        },
        ToolDefinition {
            name: RUN_TERMINAL_TOOL.to_string(),
            description: "Run a real shell command inside the project root.".to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {"command": {"type": "string"}},
                "required": ["command"]
            }),
        },
        ToolDefinition {
            name: SEARCH_FILES_TOOL.to_string(),
            description: "Search the real project for a plain substring across every real \
                text file (binary files and common noise directories like .git/node_modules/ \
                target are skipped automatically). Use this to find where something is \
                defined or used before guessing a file path. `path` optionally scopes the \
                search to one real subdirectory; omit it to search the whole project."
                .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "path": {"type": "string", "description": "Optional subdirectory to scope the search to"}
                },
                "required": ["pattern"]
            }),
        },
        ToolDefinition {
            name: LIST_DIRECTORY_TOOL.to_string(),
            description: "List the real, immediate contents (files and subdirectories) of \
                a real directory. `path` is relative to the project root; omit it to list \
                the real project root itself."
                .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Optional directory to list, relative to the project root"}
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: TASK_COMPLETE_TOOL.to_string(),
            description: "Call this exactly once, and only once, when the plan has been \
                fully executed and no further tool calls are needed."
                .to_string(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "summary": {"type": "string", "description": "What was actually done"}
                },
                "required": ["summary"]
            }),
        },
    ]
}

fn system_prompt(plan: &ImplementationPlan) -> String {
    format!(
        "You are Leo, a coding agent embedded in the Spartan IDE. The following plan has \
         already been approved by the user:\n\
         Goal: {}\n\
         Approach: {}\n\
         Files: {}\n\
         Risk notes: {}\n\n\
         Execute this plan by calling exactly one real tool per turn: search_files or \
         list_directory to explore the project when you're not certain of an exact path or \
         what a file currently contains, read_file to see a file's real current content \
         before editing it, edit_file to write a real change, or run_terminal to run a real \
         command. Prefer search_files/list_directory over guessing a path. After you have \
         finished, call task_complete exactly once with a real summary of what you did. \
         Never call more than one tool in a single turn.\n\n{}",
        plan.goal,
        plan.approach,
        plan.files.join(", "),
        plan.risk_notes,
        crate::persona::LEO_PERSONA,
    )
}

/// Runs one real model round trip: given the approved `plan` and the
/// `history` accumulated so far (empty on the very first call; a caller
/// appends an `Assistant`-role acknowledgement and a matching `Role::Tool`
/// reply after each real `ToolCall` actually runs, then passes the grown
/// `history` back in on the next call -- see `agent.rs`'s eventual UI-side
/// driver for the real accumulation loop), asks the model for exactly the
/// next action.
pub fn next_action(
    provider: &dyn ModelProvider,
    plan: &ImplementationPlan,
    history: &[Message],
) -> Result<ExecuteStep, ExecuteError> {
    next_action_cancellable(provider, plan, history, &AtomicBool::new(false))
}

/// Real §75.73-closing cooperative cancellation (task #269): identical to
/// `next_action`, but a caller can set `cancel` to `true` (from another
/// thread) to ask a real, possibly slow, already-in-flight model call to
/// stop early rather than run to completion -- `next_action` itself is now
/// a thin wrapper around this with a permanently-false flag, so every
/// existing caller/test is completely unaffected.
pub fn next_action_cancellable(
    provider: &dyn ModelProvider,
    plan: &ImplementationPlan,
    history: &[Message],
    cancel: &AtomicBool,
) -> Result<ExecuteStep, ExecuteError> {
    let request = CompletionRequest {
        messages: history.to_vec(),
        tools: execute_tool_definitions(),
        system_prompt: system_prompt(plan),
        max_tokens: 2048,
        temperature: 0.2,
    };

    let mut call_id = String::new();
    let mut call_name = String::new();
    let mut call_args = String::new();
    let mut saw_call = false;

    provider
        .stream_completion_cancellable(
            &request,
            &mut |delta| match delta {
                Delta::ToolCallStart { id, name } => {
                    if !saw_call {
                        call_id = id;
                        call_name = name;
                        saw_call = true;
                    }
                }
                Delta::ToolCallArgsChunk { id, partial_json } => {
                    if saw_call && id == call_id {
                        call_args.push_str(&partial_json);
                    }
                }
                Delta::ToolCallEnd { .. } | Delta::TextChunk(_) | Delta::Stop { .. } => {}
            },
            cancel,
        )
        .map_err(|e| ExecuteError::Provider(e.to_string()))?;

    if !saw_call {
        return Err(ExecuteError::NoActionProposed);
    }

    let parsed: Value =
        serde_json::from_str(&call_args).map_err(|e| ExecuteError::MalformedCall {
            tool: call_name.clone(),
            raw: call_args.clone(),
            reason: e.to_string(),
        })?;

    let action = match call_name.as_str() {
        READ_FILE_TOOL => {
            let path = require_str(&parsed, "path", &call_name, &call_args)?;
            ExecuteAction::Call(ToolCall::ReadFile { path })
        }
        EDIT_FILE_TOOL => {
            let path = require_str(&parsed, "path", &call_name, &call_args)?;
            let content = require_str(&parsed, "content", &call_name, &call_args)?;
            ExecuteAction::Call(ToolCall::EditFile { path, content })
        }
        RUN_TERMINAL_TOOL => {
            let command = require_str(&parsed, "command", &call_name, &call_args)?;
            ExecuteAction::Call(ToolCall::RunTerminal { command })
        }
        SEARCH_FILES_TOOL => {
            let pattern = require_str(&parsed, "pattern", &call_name, &call_args)?;
            let path = optional_str(&parsed, "path");
            ExecuteAction::Call(ToolCall::SearchFiles { pattern, path })
        }
        LIST_DIRECTORY_TOOL => {
            let path = optional_str(&parsed, "path");
            ExecuteAction::Call(ToolCall::ListDirectory { path })
        }
        TASK_COMPLETE_TOOL => {
            let summary = require_str(&parsed, "summary", &call_name, &call_args)?;
            ExecuteAction::Done { summary }
        }
        other => {
            return Err(ExecuteError::MalformedCall {
                tool: other.to_string(),
                raw: call_args,
                reason: "not one of read_file/edit_file/run_terminal/search_files/\
                    list_directory/task_complete"
                    .to_string(),
            })
        }
    };

    Ok(ExecuteStep { call_id, action })
}

fn require_str(parsed: &Value, field: &str, tool: &str, raw: &str) -> Result<String, ExecuteError> {
    parsed
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ExecuteError::MalformedCall {
            tool: tool.to_string(),
            raw: raw.to_string(),
            reason: format!("missing or non-string '{field}'"),
        })
}

/// A real, genuinely optional string field (`search_files`'s/
/// `list_directory`'s own `path`) -- `None` for both a missing field and
/// an explicit real empty string, since either one means "the whole
/// project root" to `Sandbox::search_files`/`list_directory`.
fn optional_str(parsed: &Value, field: &str) -> Option<String> {
    parsed
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Builds the real `Assistant` + `Role::Tool` message pair a caller
/// appends to `history` after actually running a `ToolCall` this module
/// proposed -- kept here (not left for each caller to hand-assemble) so
/// the exact real shape `next_action`'s own request expects stays in one
/// place.
pub fn append_tool_result(history: &mut Vec<Message>, call_id: &str, result_text: &str) {
    history.push(Message {
        role: Role::Assistant,
        content: String::new(),
        tool_call_id: Some(call_id.to_string()),
    });
    history.push(Message {
        role: Role::Tool,
        content: result_text.to_string(),
        tool_call_id: Some(call_id.to_string()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use spartan_model::provider::{ProviderError, ProviderHealth, StopReason};

    fn sample_plan() -> ImplementationPlan {
        ImplementationPlan {
            goal: "test goal".to_string(),
            approach: "test approach".to_string(),
            files: vec!["a.txt".to_string()],
            risk_notes: "none".to_string(),
        }
    }

    struct FakeProvider {
        tool: Option<(&'static str, Value)>,
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
            if let Some((name, args)) = &self.tool {
                on_delta(Delta::ToolCallStart {
                    id: "call_1".to_string(),
                    name: name.to_string(),
                });
                on_delta(Delta::ToolCallArgsChunk {
                    id: "call_1".to_string(),
                    partial_json: args.to_string(),
                });
                on_delta(Delta::ToolCallEnd {
                    id: "call_1".to_string(),
                });
                on_delta(Delta::Stop {
                    reason: StopReason::ToolUse,
                });
            } else {
                on_delta(Delta::TextChunk("no tool call".to_string()));
                on_delta(Delta::Stop {
                    reason: StopReason::EndTurn,
                });
            }
            Ok(())
        }
    }

    #[test]
    fn a_real_read_file_call_parses_correctly() {
        let provider = FakeProvider {
            tool: Some((READ_FILE_TOOL, json!({"path": "a.txt"}))),
        };
        let step = next_action(&provider, &sample_plan(), &[]).unwrap();
        assert_eq!(step.call_id, "call_1");
        assert!(matches!(
            step.action,
            ExecuteAction::Call(ToolCall::ReadFile { path }) if path == "a.txt"
        ));
    }

    #[test]
    fn a_real_edit_file_call_parses_correctly() {
        let provider = FakeProvider {
            tool: Some((
                EDIT_FILE_TOOL,
                json!({"path": "a.txt", "content": "new content"}),
            )),
        };
        let step = next_action(&provider, &sample_plan(), &[]).unwrap();
        match step.action {
            ExecuteAction::Call(ToolCall::EditFile { path, content }) => {
                assert_eq!(path, "a.txt");
                assert_eq!(content, "new content");
            }
            other => panic!("expected EditFile, got {other:?}"),
        }
    }

    #[test]
    fn a_real_run_terminal_call_parses_correctly() {
        let provider = FakeProvider {
            tool: Some((RUN_TERMINAL_TOOL, json!({"command": "cargo build"}))),
        };
        let step = next_action(&provider, &sample_plan(), &[]).unwrap();
        assert!(matches!(
            step.action,
            ExecuteAction::Call(ToolCall::RunTerminal { command }) if command == "cargo build"
        ));
    }

    #[test]
    fn a_real_search_files_call_parses_correctly_with_a_scoped_path() {
        let provider = FakeProvider {
            tool: Some((
                SEARCH_FILES_TOOL,
                json!({"pattern": "fn foo", "path": "src"}),
            )),
        };
        let step = next_action(&provider, &sample_plan(), &[]).unwrap();
        match step.action {
            ExecuteAction::Call(ToolCall::SearchFiles { pattern, path }) => {
                assert_eq!(pattern, "fn foo");
                assert_eq!(path.as_deref(), Some("src"));
            }
            other => panic!("expected SearchFiles, got {other:?}"),
        }
    }

    #[test]
    fn a_real_search_files_call_with_no_path_defaults_to_the_whole_project() {
        let provider = FakeProvider {
            tool: Some((SEARCH_FILES_TOOL, json!({"pattern": "TODO"}))),
        };
        let step = next_action(&provider, &sample_plan(), &[]).unwrap();
        assert!(matches!(
            step.action,
            ExecuteAction::Call(ToolCall::SearchFiles { pattern, path })
                if pattern == "TODO" && path.is_none()
        ));
    }

    #[test]
    fn a_real_list_directory_call_parses_correctly() {
        let provider = FakeProvider {
            tool: Some((LIST_DIRECTORY_TOOL, json!({"path": "src"}))),
        };
        let step = next_action(&provider, &sample_plan(), &[]).unwrap();
        assert!(matches!(
            step.action,
            ExecuteAction::Call(ToolCall::ListDirectory { path }) if path.as_deref() == Some("src")
        ));
    }

    #[test]
    fn a_real_list_directory_call_with_no_args_at_all_lists_the_root() {
        let provider = FakeProvider {
            tool: Some((LIST_DIRECTORY_TOOL, json!({}))),
        };
        let step = next_action(&provider, &sample_plan(), &[]).unwrap();
        assert!(matches!(
            step.action,
            ExecuteAction::Call(ToolCall::ListDirectory { path }) if path.is_none()
        ));
    }

    #[test]
    fn a_real_task_complete_call_parses_as_done() {
        let provider = FakeProvider {
            tool: Some((TASK_COMPLETE_TOOL, json!({"summary": "did the thing"}))),
        };
        let step = next_action(&provider, &sample_plan(), &[]).unwrap();
        assert!(matches!(
            step.action,
            ExecuteAction::Done { summary } if summary == "did the thing"
        ));
    }

    #[test]
    fn no_tool_call_is_a_real_distinct_error() {
        let provider = FakeProvider { tool: None };
        let result = next_action(&provider, &sample_plan(), &[]);
        assert!(matches!(result, Err(ExecuteError::NoActionProposed)));
    }

    #[test]
    fn a_missing_required_field_is_a_real_malformed_call_not_a_panic() {
        let provider = FakeProvider {
            tool: Some((EDIT_FILE_TOOL, json!({"path": "a.txt"}))),
        };
        let result = next_action(&provider, &sample_plan(), &[]);
        assert!(matches!(result, Err(ExecuteError::MalformedCall { .. })));
    }

    #[test]
    fn an_unrecognized_tool_name_is_a_real_malformed_call() {
        let provider = FakeProvider {
            tool: Some(("delete_everything", json!({}))),
        };
        let result = next_action(&provider, &sample_plan(), &[]);
        assert!(matches!(result, Err(ExecuteError::MalformedCall { .. })));
    }

    #[test]
    fn append_tool_result_builds_a_real_addressable_reply() {
        let mut history = Vec::new();
        append_tool_result(&mut history, "call_1", "file contents here");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role as u8, Role::Assistant as u8);
        assert_eq!(history[0].tool_call_id, Some("call_1".to_string()));
        assert_eq!(history[1].role as u8, Role::Tool as u8);
        assert_eq!(history[1].content, "file contents here");
        assert_eq!(history[1].tool_call_id, Some("call_1".to_string()));
    }

    /// Real §75.95 check: the sarcastic-persona instruction is genuinely
    /// present in the real execute-step system prompt, alongside the
    /// real plan details it's built from -- not just co-located in
    /// `persona.rs` and never actually wired in.
    #[test]
    fn the_real_system_prompt_carries_the_real_leo_persona_and_the_real_plan() {
        let prompt = system_prompt(&sample_plan());
        assert!(prompt.contains(crate::persona::LEO_PERSONA));
        assert!(prompt.contains("test goal"));
        assert!(prompt.contains("task_complete"));
    }

    /// Real §75.73-closing cooperative cancellation (task #269) -- a real
    /// fake overriding `stream_completion_cancellable` directly (the exact
    /// method `next_action_cancellable` actually calls) confirms a real
    /// provider-level cancellation propagates as `ExecuteError::Provider`,
    /// matching `next_action`'s own existing error-surfacing behavior for
    /// any other real provider error.
    struct CancellingProvider;

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
        fn health_check(&self) -> spartan_model::provider::ProviderHealth {
            spartan_model::provider::ProviderHealth::Healthy
        }
        fn stream_completion(
            &self,
            _request: &CompletionRequest,
            _on_delta: &mut dyn FnMut(Delta),
        ) -> Result<(), spartan_model::provider::ProviderError> {
            unreachable!("this test always calls the cancellable path")
        }
        fn stream_completion_cancellable(
            &self,
            _request: &CompletionRequest,
            _on_delta: &mut dyn FnMut(Delta),
            _cancel: &AtomicBool,
        ) -> Result<(), spartan_model::provider::ProviderError> {
            Err(spartan_model::provider::ProviderError::Cancelled)
        }
    }

    #[test]
    fn a_real_provider_level_cancellation_is_surfaced_as_a_real_provider_error() {
        let cancel = AtomicBool::new(false);
        let result = next_action_cancellable(&CancellingProvider, &sample_plan(), &[], &cancel);
        match result {
            Err(ExecuteError::Provider(msg)) => assert!(msg.contains("cancelled"), "got: {msg}"),
            other => panic!("expected ExecuteError::Provider, got {other:?}"),
        }
    }

    /// `next_action` (the non-cancellable wrapper) must remain completely
    /// unaffected -- a real, permanently-false internal flag means a
    /// provider's own `stream_completion_cancellable` override never
    /// actually observes a cancellation through this path.
    #[test]
    fn next_action_itself_never_triggers_cancellation() {
        let provider = FakeProvider {
            tool: Some((TASK_COMPLETE_TOOL, json!({"summary": "done"}))),
        };
        let result = next_action(&provider, &sample_plan(), &[]);
        assert!(result.is_ok());
    }
}
