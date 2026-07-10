//! Real §4.1 agent state machine (task #5), first real increment of Leo.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    Planning,
    AwaitingApproval,
    Executing,
    Verifying,
    Done,
    Failed,
    Recovering,
}

impl AgentState {
    /// Real, enforced transition table matching §4.1's diagram
    /// (`Idle -> Planning -> AwaitingApproval -> Executing -> Verifying ->
    /// Done`, with `Executing/Verifying -> Failed -> Recovering ->
    /// Executing`) -- an invalid transition is a real programming error in
    /// the caller, not something to silently allow.
    ///
    /// `Planning -> Idle` and `Executing/Verifying -> Idle` (§75.73) are
    /// the real, user-initiated cancel transitions -- distinct from
    /// `AwaitingApproval -> Idle`'s existing "reject the plan" semantics,
    /// but the same real target state, since a cancelled task has nothing
    /// left to resume: `Agent::cancel` abandons whatever's in flight
    /// rather than trying to represent a fourth "Cancelled" terminal
    /// state the rest of this crate would then need to handle everywhere
    /// `Idle`/`Failed`/`Done` already are.
    pub fn can_transition_to(self, next: AgentState) -> bool {
        use AgentState::*;
        matches!(
            (self, next),
            (Idle, Planning)
                | (Planning, AwaitingApproval)
                | (Planning, Failed)
                | (Planning, Idle) // user cancels while planning
                | (AwaitingApproval, Executing)
                | (AwaitingApproval, Idle) // user rejects the plan
                | (Executing, Verifying)
                | (Executing, Failed)
                | (Executing, Idle) // user cancels mid-execution
                | (Verifying, Done)
                | (Verifying, Failed)
                | (Verifying, Idle) // user cancels during verification
                | (Failed, Recovering)
                | (Recovering, Executing)
                | (Recovering, Failed) // bounded retry exhausted
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_real_happy_path_is_all_valid_transitions() {
        let path = [
            AgentState::Idle,
            AgentState::Planning,
            AgentState::AwaitingApproval,
            AgentState::Executing,
            AgentState::Verifying,
            AgentState::Done,
        ];
        for pair in path.windows(2) {
            assert!(
                pair[0].can_transition_to(pair[1]),
                "expected {:?} -> {:?} to be valid",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn the_real_recovery_loop_is_valid() {
        assert!(AgentState::Executing.can_transition_to(AgentState::Failed));
        assert!(AgentState::Failed.can_transition_to(AgentState::Recovering));
        assert!(AgentState::Recovering.can_transition_to(AgentState::Executing));
    }

    #[test]
    fn skipping_approval_is_not_a_valid_transition() {
        assert!(!AgentState::Planning.can_transition_to(AgentState::Executing));
    }

    #[test]
    fn cancelling_from_planning_executing_or_verifying_returns_to_idle() {
        assert!(AgentState::Planning.can_transition_to(AgentState::Idle));
        assert!(AgentState::Executing.can_transition_to(AgentState::Idle));
        assert!(AgentState::Verifying.can_transition_to(AgentState::Idle));
    }

    #[test]
    fn cancelling_from_a_terminal_or_already_idle_state_is_not_a_valid_transition() {
        assert!(!AgentState::Idle.can_transition_to(AgentState::Idle));
        assert!(!AgentState::Done.can_transition_to(AgentState::Idle));
        assert!(!AgentState::Failed.can_transition_to(AgentState::Idle));
        assert!(!AgentState::Recovering.can_transition_to(AgentState::Idle));
    }

    #[test]
    fn done_is_terminal_no_transitions_out() {
        for state in [
            AgentState::Idle,
            AgentState::Planning,
            AgentState::AwaitingApproval,
            AgentState::Executing,
            AgentState::Verifying,
            AgentState::Done,
            AgentState::Failed,
            AgentState::Recovering,
        ] {
            assert!(!AgentState::Done.can_transition_to(state));
        }
    }
}
