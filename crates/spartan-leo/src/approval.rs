//! Real §9/§4.1 approval gating (task #5): "some users want to approve
//! every plan, others only want approval on destructive operations" --
//! this module classifies each real `ToolCall` and decides, given a real
//! `ApprovalMode`, whether it may run without a human in the loop. Per
//! CLAUDE.md's own non-negotiable rule: "Local-model outputs are never
//! trusted with elevated destructive actions... without explicit
//! approval, regardless of routing mode or autonomy setting" -- `Destructive`
//! calls are never auto-approved by this module, full stop, no autonomy
//! level overrides it.

use crate::tool::ToolCall;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalMode {
    /// Every tool call, safe or destructive, needs explicit approval.
    ManualEveryStep,
    /// `Safe` calls run without asking; `Destructive` calls always still
    /// need approval (see this module's own doc comment -- this is not
    /// overridable).
    AutoApproveSafe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskClass {
    Safe,
    Destructive,
}

/// Real, conservative classification -- `run_terminal` is always
/// `Destructive` (an arbitrary shell command can do anything a file-scoped
/// tool call can't be limited to), `edit_file` is `Destructive` (it
/// mutates real user files), `read_file`/`search_files`/`list_directory`
/// are all `Safe` (read-only, never touch the filesystem beyond reading
/// it).
pub fn classify(call: &ToolCall) -> RiskClass {
    match call {
        ToolCall::ReadFile { .. } => RiskClass::Safe,
        ToolCall::SearchFiles { .. } => RiskClass::Safe,
        ToolCall::ListDirectory { .. } => RiskClass::Safe,
        ToolCall::EditFile { .. } => RiskClass::Destructive,
        ToolCall::RunTerminal { .. } => RiskClass::Destructive,
    }
}

/// Real gate: the one function every tool call must pass through before
/// `Sandbox::execute` runs it. Returns `true` only when it's safe to
/// proceed *without* asking a human first -- `false` means the caller
/// must obtain real, explicit approval (however the UI surfaces that)
/// before calling `Sandbox::execute`.
pub fn may_auto_execute(call: &ToolCall, mode: ApprovalMode) -> bool {
    matches!(
        (classify(call), mode),
        (RiskClass::Safe, ApprovalMode::AutoApproveSafe)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_file_is_classified_safe() {
        assert_eq!(
            classify(&ToolCall::ReadFile { path: "x".into() }),
            RiskClass::Safe
        );
    }

    #[test]
    fn search_files_is_classified_safe() {
        assert_eq!(
            classify(&ToolCall::SearchFiles {
                pattern: "x".into(),
                path: None
            }),
            RiskClass::Safe
        );
    }

    #[test]
    fn list_directory_is_classified_safe() {
        assert_eq!(
            classify(&ToolCall::ListDirectory { path: None }),
            RiskClass::Safe
        );
    }

    #[test]
    fn edit_file_is_classified_destructive() {
        assert_eq!(
            classify(&ToolCall::EditFile {
                path: "x".into(),
                content: "y".into()
            }),
            RiskClass::Destructive
        );
    }

    #[test]
    fn run_terminal_is_classified_destructive() {
        assert_eq!(
            classify(&ToolCall::RunTerminal {
                command: "ls".into()
            }),
            RiskClass::Destructive
        );
    }

    #[test]
    fn manual_every_step_never_auto_executes_even_a_safe_call() {
        let call = ToolCall::ReadFile { path: "x".into() };
        assert!(!may_auto_execute(&call, ApprovalMode::ManualEveryStep));
    }

    #[test]
    fn auto_approve_safe_auto_executes_a_real_read() {
        let call = ToolCall::ReadFile { path: "x".into() };
        assert!(may_auto_execute(&call, ApprovalMode::AutoApproveSafe));
    }

    #[test]
    fn auto_approve_safe_never_auto_executes_a_destructive_call() {
        let call = ToolCall::EditFile {
            path: "x".into(),
            content: "y".into(),
        };
        assert!(!may_auto_execute(&call, ApprovalMode::AutoApproveSafe));
        let call = ToolCall::RunTerminal {
            command: "rm -rf /".into(),
        };
        assert!(!may_auto_execute(&call, ApprovalMode::AutoApproveSafe));
    }
}
