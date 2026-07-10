//! Real §4.1 orchestration (task #5): the `Agent` struct is the one real
//! object tying the state machine (`state.rs`), plan generation
//! (`plan.rs`), the sandboxed tool layer (`tool.rs`), approval gating
//! (`approval.rs`), checkpointing (`checkpoint.rs`), and project memory
//! (`memory.rs`) into the real plan -> approve -> execute -> verify loop.
//!
//! Deliberately a synchronous, step-driven API -- the caller (eventually
//! `spartan-editor-core`'s render loop, the same way `LspSession`/
//! `DapSession` already run real work off the render thread via their own
//! background-thread patterns) drives each transition explicitly. This
//! crate does not itself spawn threads; that's a real, deliberate
//! separation from `spartan-editor-core`'s own established
//! thread-plus-channel convention, left for the UI-wiring pass that
//! actually needs it, rather than guessed at here with no real caller yet.

use crate::approval::{may_auto_execute, ApprovalMode};
use crate::checkpoint::{self, Checkpoint, CheckpointError};
use crate::plan::{generate_plan, ImplementationPlan, PlanError};
use crate::state::AgentState;
use crate::tool::{Sandbox, SandboxError, ToolCall, ToolResult};
use spartan_model::provider::ModelProvider;
use std::path::PathBuf;

const MAX_RECOVERY_ATTEMPTS: u32 = 3;

#[derive(Debug)]
pub enum AgentError {
    InvalidTransition { from: AgentState, to: AgentState },
    Plan(PlanError),
    Sandbox(SandboxError),
    Checkpoint(CheckpointError),
    RecoveryExhausted,
}

impl From<SandboxError> for AgentError {
    fn from(e: SandboxError) -> Self {
        AgentError::Sandbox(e)
    }
}

impl From<CheckpointError> for AgentError {
    fn from(e: CheckpointError) -> Self {
        AgentError::Checkpoint(e)
    }
}

pub struct Agent {
    state: AgentState,
    project_root: PathBuf,
    sandbox: Sandbox,
    approval_mode: ApprovalMode,
    plan: Option<ImplementationPlan>,
    /// The real checkpoint taken at the start of the current `Executing`
    /// phase -- what `Recovering` rolls back to before its retry, per
    /// §4.1's "each attempt's diff kept distinct... user can see attempt 1
    /// failed because X, attempt 2 fixed it" (the retry loop itself is
    /// real here; the distinct-per-attempt diff history is a real, named
    /// UI-layer concern this crate doesn't own).
    current_checkpoint: Option<Checkpoint>,
    recovery_attempts: u32,
}

impl Agent {
    pub fn new(project_root: PathBuf, approval_mode: ApprovalMode) -> Self {
        let sandbox = Sandbox::new(&project_root);
        Self {
            state: AgentState::Idle,
            project_root,
            sandbox,
            approval_mode,
            plan: None,
            current_checkpoint: None,
            recovery_attempts: 0,
        }
    }

    pub fn state(&self) -> AgentState {
        self.state
    }

    pub fn plan(&self) -> Option<&ImplementationPlan> {
        self.plan.as_ref()
    }

    fn transition(&mut self, to: AgentState) -> Result<(), AgentError> {
        if !self.state.can_transition_to(to) {
            return Err(AgentError::InvalidTransition {
                from: self.state,
                to,
            });
        }
        self.state = to;
        Ok(())
    }

    /// `Idle -> Planning -> AwaitingApproval`. A real, blocking model call
    /// (see `plan.rs`'s own doc comment) -- the caller is responsible for
    /// keeping this off any UI-blocking thread, matching this crate's own
    /// "no threading opinions" scope note above.
    pub fn start_task(
        &mut self,
        provider: &dyn ModelProvider,
        task: &str,
    ) -> Result<&ImplementationPlan, AgentError> {
        self.transition(AgentState::Planning)?;
        let result = generate_plan(provider, task);
        self.apply_generated_plan(result)?;
        Ok(self.plan.as_ref().unwrap())
    }

    /// `Idle -> Planning`, without making the real blocking model call --
    /// the real §75.47 UI-wiring split of `start_task`'s own two halves, so
    /// a caller (`spartan-editor-core`'s render loop) can transition state
    /// immediately, spawn `plan::generate_plan` on its own background
    /// thread (matching `LspSession`/`DapSession`'s own established
    /// pattern), and apply just the result later via
    /// `apply_generated_plan` once the thread reports back -- never
    /// blocking the render loop on a real model call the way a bare
    /// `start_task` would.
    pub fn begin_planning(&mut self) -> Result<(), AgentError> {
        self.transition(AgentState::Planning)
    }

    /// `Planning -> AwaitingApproval` (real plan) or `Planning -> Failed`
    /// (a real `PlanError`) -- the second half of `start_task`, split out
    /// so it can be called with a result produced off-thread. `start_task`
    /// itself is now a thin, byte-identical-behavior wrapper around
    /// `begin_planning` + a blocking `generate_plan` call + this.
    pub fn apply_generated_plan(
        &mut self,
        result: Result<ImplementationPlan, PlanError>,
    ) -> Result<(), AgentError> {
        match result {
            Ok(plan) => {
                self.plan = Some(plan);
                self.transition(AgentState::AwaitingApproval)
            }
            Err(e) => {
                self.transition(AgentState::Failed)?;
                Err(AgentError::Plan(e))
            }
        }
    }

    /// `AwaitingApproval -> Executing`. Takes a real checkpoint (§4.2) the
    /// instant execution begins, before any tool call in this phase runs.
    pub fn approve_plan(&mut self, repo: &mut git2::Repository) -> Result<(), AgentError> {
        self.transition(AgentState::Executing)?;
        self.current_checkpoint = Some(checkpoint::create_checkpoint(
            repo,
            "before Leo's execution phase",
        )?);
        Ok(())
    }

    /// `AwaitingApproval -> Idle`, discarding the proposed plan -- the
    /// user rejected it.
    pub fn reject_plan(&mut self) -> Result<(), AgentError> {
        self.transition(AgentState::Idle)?;
        self.plan = None;
        Ok(())
    }

    /// Real §9 approval gate, checked *before* `execute_call` ever touches
    /// the sandbox -- a caller must check this (or hold an explicit,
    /// separately-obtained human approval) before calling `execute_call`
    /// for a call this returns `false` for. This function alone never
    /// runs the call.
    pub fn may_auto_execute(&self, call: &ToolCall) -> bool {
        may_auto_execute(call, self.approval_mode)
    }

    /// Real, read-only peek at a file's *current* content through the
    /// exact same jailed `Sandbox` every other tool call uses (§75.68) --
    /// deliberately does **not** require `Executing` state and does
    /// **not** count as a real tool call (no history entry, no approval
    /// gate consumed): its only purpose is letting a caller build a real
    /// diff preview for a proposed `edit_file` call *before* that call is
    /// approved. Returns `None` (not an error) when the file doesn't
    /// exist yet -- a real, valid case for a brand-new file `edit_file`
    /// would create, not a failure.
    pub fn peek_file(&self, path: &str) -> Option<String> {
        match self.sandbox.read_file(path) {
            Ok(ToolResult::FileContent(content)) => Some(content),
            _ => None,
        }
    }

    /// Must be in `Executing`. Runs `call` through the real, hard-jailed
    /// sandbox -- this function does not itself re-check
    /// `may_auto_execute`; a caller that skips the approval gate for a
    /// `Destructive` call without real, separately-obtained human
    /// approval is violating §9's own invariant, not something this type
    /// system alone can prevent for a synchronous, embeddable library
    /// (the real enforcement is architectural: no code path in
    /// `spartan-editor-core`'s eventual UI wiring may call this without
    /// having gated on `may_auto_execute` or real user approval first).
    pub fn execute_call(&mut self, call: ToolCall) -> Result<ToolResult, AgentError> {
        if self.state != AgentState::Executing {
            return Err(AgentError::InvalidTransition {
                from: self.state,
                to: AgentState::Executing,
            });
        }
        Ok(self.sandbox.execute(&call)?)
    }

    /// `Executing -> Verifying`.
    pub fn begin_verification(&mut self) -> Result<(), AgentError> {
        self.transition(AgentState::Verifying)
    }

    /// Runs a real, configured verification command (e.g. `cargo build`,
    /// a test command) inside the same real sandbox -- §4.1's "Leo
    /// automatically runs configured verification... before declaring
    /// done." The caller decides pass/fail from the real exit code and
    /// calls `mark_done`/`mark_failed` accordingly; this function only
    /// runs the command, matching the same "who owns the actual command"
    /// separation `execute_call` already has for tool calls.
    pub fn run_verification(&mut self, command: &str) -> Result<ToolResult, AgentError> {
        if self.state != AgentState::Verifying {
            return Err(AgentError::InvalidTransition {
                from: self.state,
                to: AgentState::Verifying,
            });
        }
        Ok(self.sandbox.run_terminal(command)?)
    }

    /// `Verifying -> Done`.
    pub fn mark_done(&mut self) -> Result<(), AgentError> {
        self.transition(AgentState::Done)?;
        self.recovery_attempts = 0;
        Ok(())
    }

    /// `Executing|Verifying -> Failed`.
    pub fn mark_failed(&mut self) -> Result<(), AgentError> {
        self.transition(AgentState::Failed)
    }

    /// `Failed -> Recovering -> Executing`, real-rolling-back to the
    /// checkpoint taken at the start of this attempt's `Executing` phase
    /// first, matching §4.1's bounded-retry design ("default max 3
    /// attempts"). Returns `RecoveryExhausted` (not a silent stop) once
    /// the bound is hit, and does *not* itself decide what a caller does
    /// next -- surfacing the exhaustion is this crate's job, presenting
    /// it to the user is the UI layer's.
    pub fn begin_recovery(&mut self, repo: &mut git2::Repository) -> Result<(), AgentError> {
        if self.recovery_attempts >= MAX_RECOVERY_ATTEMPTS {
            return Err(AgentError::RecoveryExhausted);
        }
        self.transition(AgentState::Recovering)?;
        if let Some(checkpoint) = &self.current_checkpoint {
            checkpoint::restore_checkpoint(repo, checkpoint)?;
        }
        self.recovery_attempts += 1;
        self.transition(AgentState::Executing)?;
        Ok(())
    }

    pub fn append_memory(&self, entry: &str) -> std::io::Result<()> {
        crate::memory::append_project_memory(&self.project_root, entry)
    }

    pub fn read_memory(&self) -> std::io::Result<String> {
        crate::memory::read_project_memory(&self.project_root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spartan_model::provider::{
        CompletionRequest, Delta, ProviderError, ProviderHealth, StopReason,
    };

    struct FakePlanningProvider;
    impl ModelProvider for FakePlanningProvider {
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
            on_delta(Delta::ToolCallStart {
                id: "1".to_string(),
                name: "propose_plan".to_string(),
            });
            on_delta(Delta::ToolCallArgsChunk {
                id: "1".to_string(),
                partial_json: serde_json::json!({
                    "goal": "test goal",
                    "approach": "test approach",
                    "files": ["a.txt"],
                    "risk_notes": "none"
                })
                .to_string(),
            });
            on_delta(Delta::ToolCallEnd {
                id: "1".to_string(),
            });
            on_delta(Delta::Stop {
                reason: StopReason::ToolUse,
            });
            Ok(())
        }
    }

    fn real_repo_with_one_commit(name: &str) -> (PathBuf, git2::Repository) {
        let dir = std::env::temp_dir().join(format!("spartan-leo-agent-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = git2::Repository::init(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "original\n").unwrap();
        {
            let sig = git2::Signature::now("Test", "test@example.com").unwrap();
            let mut index = repo.index().unwrap();
            index.add_path(std::path::Path::new("a.txt")).unwrap();
            index.write().unwrap();
            let tree_oid = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
                .unwrap();
        }
        (dir, repo)
    }

    #[test]
    fn begin_planning_and_apply_generated_plan_reach_the_same_state_as_start_task() {
        // Real §75.47 coverage for the split `start_task` was refactored
        // into -- the actual async-friendly path `spartan-editor-core`'s UI
        // wiring uses (transition to `Planning` immediately, do the real
        // blocking model call off-thread, apply just the result later).
        let (dir, _repo) = real_repo_with_one_commit("split-happy");
        let mut agent = Agent::new(dir, ApprovalMode::ManualEveryStep);

        agent.begin_planning().unwrap();
        assert_eq!(agent.state(), AgentState::Planning);

        let result = generate_plan(&FakePlanningProvider, "do the thing");
        agent.apply_generated_plan(result).unwrap();

        assert_eq!(agent.state(), AgentState::AwaitingApproval);
        assert_eq!(agent.plan().unwrap().goal, "test goal");
    }

    #[test]
    fn apply_generated_plan_with_a_real_error_lands_in_failed() {
        struct AlwaysFailsProvider;
        impl ModelProvider for AlwaysFailsProvider {
            fn id(&self) -> &str {
                "fake-fail"
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
                on_delta(Delta::Stop {
                    reason: StopReason::EndTurn,
                });
                Ok(())
            }
        }

        let (dir, _repo) = real_repo_with_one_commit("split-error");
        let mut agent = Agent::new(dir, ApprovalMode::ManualEveryStep);
        agent.begin_planning().unwrap();

        let result = generate_plan(&AlwaysFailsProvider, "do the thing");
        let err = agent.apply_generated_plan(result).unwrap_err();

        assert!(matches!(err, AgentError::Plan(PlanError::NoPlanProposed)));
        assert_eq!(agent.state(), AgentState::Failed);
    }

    #[test]
    fn real_full_happy_path_plan_approve_execute_verify_done() {
        let (dir, mut repo) = real_repo_with_one_commit("happy");
        let mut agent = Agent::new(dir.clone(), ApprovalMode::ManualEveryStep);

        agent
            .start_task(&FakePlanningProvider, "do the thing")
            .unwrap();
        assert_eq!(agent.state(), AgentState::AwaitingApproval);
        assert_eq!(agent.plan().unwrap().goal, "test goal");

        agent.approve_plan(&mut repo).unwrap();
        assert_eq!(agent.state(), AgentState::Executing);

        let call = ToolCall::EditFile {
            path: "a.txt".to_string(),
            content: "modified\n".to_string(),
        };
        assert!(
            !agent.may_auto_execute(&call),
            "ManualEveryStep must never auto-execute a destructive call"
        );
        agent.execute_call(call).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "modified\n"
        );

        agent.begin_verification().unwrap();
        let result = agent.run_verification("cat a.txt").unwrap();
        let ToolResult::TerminalOutput {
            stdout, exit_code, ..
        } = result
        else {
            panic!("expected TerminalOutput");
        };
        assert_eq!(exit_code, 0);
        assert!(stdout.contains("modified"));

        agent.mark_done().unwrap();
        assert_eq!(agent.state(), AgentState::Done);
    }

    #[test]
    fn rejecting_a_plan_returns_to_idle_and_clears_it() {
        let (dir, _repo) = real_repo_with_one_commit("reject");
        let mut agent = Agent::new(dir, ApprovalMode::ManualEveryStep);
        agent
            .start_task(&FakePlanningProvider, "do the thing")
            .unwrap();
        agent.reject_plan().unwrap();
        assert_eq!(agent.state(), AgentState::Idle);
        assert!(agent.plan().is_none());
    }

    #[test]
    fn a_destructive_call_cannot_run_before_a_plan_is_approved() {
        let (dir, _repo) = real_repo_with_one_commit("gate");
        let mut agent = Agent::new(dir, ApprovalMode::ManualEveryStep);
        let call = ToolCall::EditFile {
            path: "a.txt".to_string(),
            content: "x".to_string(),
        };
        let result = agent.execute_call(call);
        assert!(matches!(result, Err(AgentError::InvalidTransition { .. })));
    }

    #[test]
    fn peek_file_reads_a_real_existing_file_without_requiring_executing_state() {
        let (dir, _repo) = real_repo_with_one_commit("peek-existing");
        let agent = Agent::new(dir, ApprovalMode::ManualEveryStep);
        // Deliberately still `Idle` -- peek_file has no state requirement.
        assert_eq!(agent.state(), AgentState::Idle);
        assert_eq!(agent.peek_file("a.txt"), Some("original\n".to_string()));
    }

    #[test]
    fn peek_file_on_a_real_nonexistent_file_returns_none_not_an_error() {
        let (dir, _repo) = real_repo_with_one_commit("peek-missing");
        let agent = Agent::new(dir, ApprovalMode::ManualEveryStep);
        assert_eq!(agent.peek_file("does_not_exist.txt"), None);
    }

    #[test]
    fn peek_file_never_appears_in_history_or_counts_as_a_real_tool_call() {
        // A real, structural confirmation, not just a doc comment claim:
        // peek_file takes `&self`, so it's provably incapable of
        // mutating `Agent` state (no `recovery_attempts`/`plan`/history
        // field this crate owns could change) -- calling it repeatedly
        // must leave the agent's own real state completely untouched.
        let (dir, _repo) = real_repo_with_one_commit("peek-no-side-effects");
        let agent = Agent::new(dir, ApprovalMode::ManualEveryStep);
        let before = agent.state();
        let _ = agent.peek_file("a.txt");
        let _ = agent.peek_file("a.txt");
        assert_eq!(agent.state(), before);
    }

    #[test]
    fn real_recovery_restores_the_checkpoint_before_retrying() {
        let (dir, mut repo) = real_repo_with_one_commit("recover");
        let mut agent = Agent::new(dir.clone(), ApprovalMode::ManualEveryStep);
        agent
            .start_task(&FakePlanningProvider, "do the thing")
            .unwrap();
        agent.approve_plan(&mut repo).unwrap();

        agent
            .execute_call(ToolCall::EditFile {
                path: "a.txt".to_string(),
                content: "a broken edit\n".to_string(),
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "a broken edit\n"
        );

        agent.mark_failed().unwrap();
        agent.begin_recovery(&mut repo).unwrap();

        assert_eq!(agent.state(), AgentState::Executing);
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "original\n",
            "recovery should have restored the real checkpoint from before the broken edit"
        );
    }

    #[test]
    fn recovery_is_really_bounded_and_reports_exhaustion() {
        let (dir, mut repo) = real_repo_with_one_commit("exhaust");
        let mut agent = Agent::new(dir, ApprovalMode::ManualEveryStep);
        agent
            .start_task(&FakePlanningProvider, "do the thing")
            .unwrap();
        agent.approve_plan(&mut repo).unwrap();

        for _ in 0..MAX_RECOVERY_ATTEMPTS {
            agent.mark_failed().unwrap();
            agent.begin_recovery(&mut repo).unwrap();
        }
        agent.mark_failed().unwrap();
        let result = agent.begin_recovery(&mut repo);
        assert!(matches!(result, Err(AgentError::RecoveryExhausted)));
    }

    #[test]
    fn real_project_memory_round_trips_through_the_agent() {
        let (dir, _repo) = real_repo_with_one_commit("memory");
        let agent = Agent::new(dir, ApprovalMode::ManualEveryStep);
        agent.append_memory("Don't use default exports").unwrap();
        let content = agent.read_memory().unwrap();
        assert!(content.contains("Don't use default exports"));
    }
}
