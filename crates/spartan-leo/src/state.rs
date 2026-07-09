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
    pub fn can_transition_to(self, next: AgentState) -> bool {
        use AgentState::*;
        matches!(
            (self, next),
            (Idle, Planning)
                | (Planning, AwaitingApproval)
                | (Planning, Failed)
                | (AwaitingApproval, Executing)
                | (AwaitingApproval, Idle) // user rejects the plan
                | (Executing, Verifying)
                | (Executing, Failed)
                | (Verifying, Done)
                | (Verifying, Failed)
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
