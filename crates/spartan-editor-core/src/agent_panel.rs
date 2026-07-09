//! Real Agent mode UI (§8, §4, task #5, §75.47) -- first real wiring of
//! `spartan-leo`'s already-tested `Agent`/`plan::generate_plan` into this
//! crate's own UI, replacing §75.36's static placeholder text. Pure,
//! headlessly-tested display-text/state logic only, mirroring
//! `tab_bar.rs`/`mode_toggle.rs`/`command_palette.rs`'s own "no GPU
//! dependency in this module" split -- the actual background-thread model
//! call lives in `leo_bridge.rs`, and the keyboard/mouse wiring lives in
//! `main.rs`.
//!
//! Deliberately real and honestly scoped: this pass wires task input ->
//! real live plan generation -> approve (a real git checkpoint via
//! `spartan-leo::checkpoint`) / reject. It does **not** wire actual
//! execution of the approved plan's tool calls -- `spartan-leo` has no
//! model-facing "propose the concrete tool calls for this approved plan"
//! step yet (a real, separate, larger increment: a second real model
//! round trip with its own tool schema, not attempted here), so approving
//! a plan creates a real checkpoint and then has nothing further to run.
//! Named explicitly in the resulting UI text, not hidden.

use spartan_leo::plan::ImplementationPlan;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum AgentPanelState {
    /// No task typed yet.
    #[default]
    Idle,
    /// The user is typing a task description.
    EditingTask(String),
    /// A real background-thread `generate_plan` call is in flight.
    Planning { task: String },
    /// A real plan came back and is awaiting approve/reject.
    PlanReady { plan: ImplementationPlan },
    /// The plan was approved and a real checkpoint was created.
    Approved,
    /// A real `PlanError` (or a spawn-level failure) was reported.
    Error { message: String },
}

const HEADER: &str = "Agent mode -- Leo (§4, task #5)\n\
     Uses a real local Ollama instance (llama3.1:8b) if one is reachable\n\
     at localhost:11434 -- see CLAUDE.md §75.43/§75.47.\n\n";

/// The real, live display text for the current panel state -- rebuilt
/// every frame from live state, the same convention every other real
/// panel in this crate (`tab_bar`, `git_panel`, `command_palette`)
/// already follows.
pub fn build_panel_text(state: &AgentPanelState) -> String {
    match state {
        AgentPanelState::Idle => {
            format!("{HEADER}Type a task, then press Enter to ask Leo to plan it.\n\n> _")
        }
        AgentPanelState::EditingTask(task) => {
            format!("{HEADER}Type a task, then press Enter to ask Leo to plan it.\n\n> {task}_")
        }
        AgentPanelState::Planning { task } => {
            format!(
                "{HEADER}Leo is planning \"{task}\"...\n\n\
                 This is a real, live model call -- it can take a while on a\n\
                 local model with no GPU. The render loop stays responsive\n\
                 while this runs (§4, background-thread pattern)."
            )
        }
        AgentPanelState::PlanReady { plan } => {
            format!(
                "{HEADER}Real plan from Leo:\n\n\
                 Goal: {}\n\n\
                 Approach: {}\n\n\
                 Files: {}\n\n\
                 Risk notes: {}\n\n\
                 Enter to approve (creates a real checkpoint) -- Escape to reject.",
                plan.goal,
                plan.approach,
                if plan.files.is_empty() {
                    "(none named)".to_string()
                } else {
                    plan.files.join(", ")
                },
                plan.risk_notes,
            )
        }
        AgentPanelState::Approved => format!(
            "{HEADER}Plan approved. A real checkpoint was created (§4.2).\n\n\
             Automatic execution of the plan's tool calls is not wired yet --\n\
             Leo has no model-facing step that turns an approved plan into\n\
             concrete tool calls (see CLAUDE.md §75.47). No files were\n\
             modified by this approval.\n\n\
             Press Escape to start a new task."
        ),
        AgentPanelState::Error { message } => {
            format!(
                "{HEADER}Leo could not produce a plan:\n\n{message}\n\nPress Escape to try again."
            )
        }
    }
}

/// Whether the panel is in a state where typed characters should be
/// accepted as task text (`Idle`/`EditingTask` only).
pub fn accepts_typing(state: &AgentPanelState) -> bool {
    matches!(
        state,
        AgentPanelState::Idle | AgentPanelState::EditingTask(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_plan() -> ImplementationPlan {
        ImplementationPlan {
            goal: "Add a function".to_string(),
            approach: "Edit src/math.rs".to_string(),
            files: vec!["src/math.rs".to_string()],
            risk_notes: "Low risk".to_string(),
        }
    }

    #[test]
    fn idle_shows_the_real_prompt() {
        let text = build_panel_text(&AgentPanelState::Idle);
        assert!(text.contains("Type a task"));
        assert!(text.contains("llama3.1:8b"));
    }

    #[test]
    fn editing_task_echoes_the_real_typed_text() {
        let text = build_panel_text(&AgentPanelState::EditingTask("fix the bug".to_string()));
        assert!(text.contains("> fix the bug_"));
    }

    #[test]
    fn planning_names_the_real_task_being_planned() {
        let text = build_panel_text(&AgentPanelState::Planning {
            task: "fix the bug".to_string(),
        });
        assert!(text.contains("fix the bug"));
    }

    #[test]
    fn plan_ready_shows_every_real_plan_field() {
        let text = build_panel_text(&AgentPanelState::PlanReady {
            plan: sample_plan(),
        });
        assert!(text.contains("Add a function"));
        assert!(text.contains("Edit src/math.rs"));
        assert!(text.contains("src/math.rs"));
        assert!(text.contains("Low risk"));
    }

    #[test]
    fn plan_ready_with_no_files_says_so_explicitly() {
        let mut plan = sample_plan();
        plan.files.clear();
        let text = build_panel_text(&AgentPanelState::PlanReady { plan });
        assert!(text.contains("(none named)"));
    }

    #[test]
    fn approved_names_the_real_execution_gap() {
        let text = build_panel_text(&AgentPanelState::Approved);
        assert!(text.contains("checkpoint was created"));
        assert!(text.contains("not wired yet"));
    }

    #[test]
    fn error_shows_the_real_message() {
        let text = build_panel_text(&AgentPanelState::Error {
            message: "no tool call was ever proposed".to_string(),
        });
        assert!(text.contains("no tool call was ever proposed"));
    }

    #[test]
    fn only_idle_and_editing_task_accept_typing() {
        assert!(accepts_typing(&AgentPanelState::Idle));
        assert!(accepts_typing(&AgentPanelState::EditingTask(
            "x".to_string()
        )));
        assert!(!accepts_typing(&AgentPanelState::Planning {
            task: "x".to_string()
        }));
        assert!(!accepts_typing(&AgentPanelState::PlanReady {
            plan: sample_plan()
        }));
        assert!(!accepts_typing(&AgentPanelState::Approved));
        assert!(!accepts_typing(&AgentPanelState::Error {
            message: "x".to_string()
        }));
    }
}
