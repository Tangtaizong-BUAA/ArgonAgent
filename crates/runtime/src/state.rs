//! Agent session state transition validator.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Created,
    Planning,
    WaitingForPlanApproval,
    RetrievingContext,
    Executing,
    WaitingForToolApproval,
    ApplyingPatch,
    RunningCommand,
    DiagnosingFailure,
    Reviewing,
    WaitingForUser,
    Completed,
    Failed,
    Cancelled,
}

pub fn can_transition(from: AgentState, to: AgentState) -> bool {
    use AgentState::*;
    matches!(
        (from, to),
        (Created, Planning)
            | (Created, Cancelled)
            | (Planning, Cancelled)
            | (Planning, WaitingForPlanApproval)
            | (Planning, RetrievingContext)
            | (Planning, Failed)
            | (WaitingForPlanApproval, Executing)
            | (WaitingForPlanApproval, RetrievingContext)
            | (WaitingForPlanApproval, WaitingForUser)
            | (WaitingForPlanApproval, Cancelled)
            | (RetrievingContext, Executing)
            | (RetrievingContext, Failed)
            | (RetrievingContext, Cancelled)
            | (Executing, WaitingForPlanApproval)
            | (Executing, WaitingForToolApproval)
            | (Executing, WaitingForUser)
            | (Executing, ApplyingPatch)
            | (Executing, RunningCommand)
            | (Executing, DiagnosingFailure)
            | (Executing, Reviewing)
            | (Executing, Failed)
            | (Executing, Cancelled)
            | (WaitingForToolApproval, RunningCommand)
            | (WaitingForToolApproval, ApplyingPatch)
            | (WaitingForToolApproval, Executing)
            | (WaitingForToolApproval, WaitingForUser)
            | (WaitingForToolApproval, Cancelled)
            | (ApplyingPatch, RunningCommand)
            | (ApplyingPatch, Executing)
            | (ApplyingPatch, Reviewing)
            | (ApplyingPatch, DiagnosingFailure)
            | (ApplyingPatch, Cancelled)
            | (RunningCommand, Executing)
            | (RunningCommand, DiagnosingFailure)
            | (RunningCommand, Reviewing)
            | (RunningCommand, Cancelled)
            | (DiagnosingFailure, Executing)
            | (DiagnosingFailure, WaitingForUser)
            | (DiagnosingFailure, Failed)
            | (DiagnosingFailure, Cancelled)
            | (Reviewing, Completed)
            | (Reviewing, DiagnosingFailure)
            | (Reviewing, Executing)
            | (Reviewing, WaitingForUser)
            | (Reviewing, Cancelled)
            | (WaitingForUser, Planning)
            | (WaitingForUser, Executing)
            | (WaitingForUser, Cancelled)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_expected_plan_path() {
        assert!(can_transition(AgentState::Created, AgentState::Planning));
        assert!(can_transition(
            AgentState::Planning,
            AgentState::WaitingForPlanApproval
        ));
        assert!(can_transition(
            AgentState::WaitingForPlanApproval,
            AgentState::RetrievingContext
        ));
    }

    #[test]
    fn rejects_terminal_transition() {
        assert!(!can_transition(
            AgentState::Completed,
            AgentState::Executing
        ));
        assert!(!can_transition(AgentState::Failed, AgentState::Planning));
    }

    #[test]
    fn allows_cancelling_active_runtime_states() {
        for state in [
            AgentState::Planning,
            AgentState::RetrievingContext,
            AgentState::Executing,
            AgentState::WaitingForToolApproval,
            AgentState::ApplyingPatch,
            AgentState::RunningCommand,
            AgentState::DiagnosingFailure,
            AgentState::Reviewing,
            AgentState::WaitingForUser,
        ] {
            assert!(can_transition(state, AgentState::Cancelled), "{state:?}");
        }
    }

    #[test]
    fn rejects_skipping_safety_boundary() {
        assert!(!can_transition(
            AgentState::Created,
            AgentState::ApplyingPatch
        ));
        assert!(!can_transition(
            AgentState::Planning,
            AgentState::RunningCommand
        ));
    }

    #[test]
    fn allows_continuing_execution_after_patch() {
        assert!(can_transition(
            AgentState::ApplyingPatch,
            AgentState::Executing
        ));
    }

    #[test]
    fn allows_failure_diagnosis_from_execution() {
        assert!(can_transition(
            AgentState::Executing,
            AgentState::DiagnosingFailure
        ));
    }
}
