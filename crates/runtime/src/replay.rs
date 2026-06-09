//! Event log replay and session snapshot helpers.
//!
//! This module is deliberately read-only. It reconstructs enough Product Kernel
//! state from JSONL events to support resume gates, harness assertions, and GUI
//! summaries without mutating the original log.

use crate::event_log::{EventLog, EventLogError};
use crate::state::AgentState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayHealth {
    Completed,
    BlockedForPermission,
    BlockedForPlanApproval,
    InProgress,
    Failed,
    Cancelled,
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionReplaySnapshot {
    pub project_id: String,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub sequence: u64,
    pub last_event_type: String,
    pub inferred_state: AgentState,
    pub health: ReplayHealth,
    pub tool_calls_requested: usize,
    pub tool_calls_completed: usize,
    pub tool_results_recorded: usize,
    pub model_calls_started: usize,
    pub model_calls_completed: usize,
    pub model_calls_blocked: usize,
    pub model_streams_completed: usize,
    pub permissions_requested: usize,
    pub permissions_decided: usize,
    pub patches_applied: usize,
    pub pending_permission_ids: Vec<String>,
    pub pending_plan_approval_ids: Vec<String>,
    pub can_resume_without_user: bool,
}

impl SessionReplaySnapshot {
    pub fn to_line(&self) -> String {
        format!(
            "session={:?} state={:?} health={:?} events={} tools={}/{} models={}/{} permissions={}/{} pending_permissions={} patches={} resumable={}",
            self.session_id,
            self.inferred_state,
            self.health,
            self.sequence,
            self.tool_calls_completed,
            self.tool_calls_requested,
            self.model_calls_completed,
            self.model_calls_started,
            self.permissions_decided,
            self.permissions_requested,
            self.pending_permission_ids.len(),
            self.patches_applied,
            self.can_resume_without_user
        )
    }
}

pub fn replay_event_log(log: &EventLog) -> Result<SessionReplaySnapshot, EventLogError> {
    let mut snapshot = SessionReplaySnapshot {
        project_id: String::new(),
        session_id: None,
        task_id: None,
        sequence: 0,
        last_event_type: String::new(),
        inferred_state: AgentState::Created,
        health: ReplayHealth::InProgress,
        tool_calls_requested: 0,
        tool_calls_completed: 0,
        tool_results_recorded: 0,
        model_calls_started: 0,
        model_calls_completed: 0,
        model_calls_blocked: 0,
        model_streams_completed: 0,
        permissions_requested: 0,
        permissions_decided: 0,
        patches_applied: 0,
        pending_permission_ids: Vec::new(),
        pending_plan_approval_ids: Vec::new(),
        can_resume_without_user: false,
    };

    for event in log.iter() {
        snapshot.project_id = event.project_id.clone();
        snapshot.session_id = event.session_id.clone();
        snapshot.task_id = event.task_id.clone();
        snapshot.sequence = event.sequence;
        snapshot.last_event_type = event.event_type.clone();

        match event.event_type.as_str() {
            "session.state_changed" => {
                if let Some(state) = extract_json_string(&event.payload_json, "to_state")
                    .and_then(|value| parse_agent_state(&value))
                {
                    snapshot.inferred_state = state;
                }
            }
            "tool.call_requested" => snapshot.tool_calls_requested += 1,
            "tool.call_completed" => snapshot.tool_calls_completed += 1,
            "tool.result_recorded" => snapshot.tool_results_recorded += 1,
            "model.call_started" => snapshot.model_calls_started += 1,
            "model.call_completed" => snapshot.model_calls_completed += 1,
            "model.call_blocked" => snapshot.model_calls_blocked += 1,
            "model.stream_completed" => snapshot.model_streams_completed += 1,
            "permission.requested" => {
                snapshot.permissions_requested += 1;
                if let Some(permission_id) =
                    extract_json_string(&event.payload_json, "permission_id")
                {
                    push_unique(&mut snapshot.pending_permission_ids, permission_id);
                }
            }
            "permission.decided" => {
                snapshot.permissions_decided += 1;
                if let Some(permission_id) =
                    extract_json_string(&event.payload_json, "permission_id")
                {
                    snapshot
                        .pending_permission_ids
                        .retain(|candidate| candidate != &permission_id);
                }
            }
            "plan.approval_requested" => {
                if let Some(plan_approval_id) =
                    extract_json_string(&event.payload_json, "plan_approval_id")
                {
                    push_unique(&mut snapshot.pending_plan_approval_ids, plan_approval_id);
                }
            }
            "plan.approval_decided" => {
                if let Some(plan_approval_id) =
                    extract_json_string(&event.payload_json, "plan_approval_id")
                {
                    snapshot
                        .pending_plan_approval_ids
                        .retain(|candidate| candidate != &plan_approval_id);
                }
            }
            "patch.applied" => snapshot.patches_applied += 1,
            _ => {}
        }
    }

    snapshot.health = infer_health(&snapshot);
    snapshot.can_resume_without_user = matches!(
        snapshot.health,
        ReplayHealth::InProgress | ReplayHealth::Failed
    );
    Ok(snapshot)
}

pub fn replay_jsonl(input: &str) -> Result<SessionReplaySnapshot, EventLogError> {
    let log = EventLog::import_jsonl(input)?;
    replay_event_log(&log)
}

fn infer_health(snapshot: &SessionReplaySnapshot) -> ReplayHealth {
    match snapshot.inferred_state {
        AgentState::Completed => ReplayHealth::Completed,
        AgentState::Cancelled => ReplayHealth::Cancelled,
        AgentState::Failed => ReplayHealth::Failed,
        AgentState::WaitingForToolApproval if !snapshot.pending_permission_ids.is_empty() => {
            ReplayHealth::BlockedForPermission
        }
        AgentState::WaitingForPlanApproval if !snapshot.pending_plan_approval_ids.is_empty() => {
            ReplayHealth::BlockedForPlanApproval
        }
        AgentState::WaitingForToolApproval => ReplayHealth::Invalid(
            "waiting for tool approval without pending permission".to_string(),
        ),
        AgentState::WaitingForPlanApproval => ReplayHealth::Invalid(
            "waiting for plan approval without pending plan approval".to_string(),
        ),
        _ => ReplayHealth::InProgress,
    }
}

fn push_unique(items: &mut Vec<String>, value: String) {
    if !items.iter().any(|item| item == &value) {
        items.push(value);
    }
}

fn extract_json_string(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let start = input.find(&marker)? + marker.len();
    let tail = &input[start..];
    let end = tail.find('"')?;
    Some(tail[..end].replace("\\\"", "\"").replace("\\\\", "\\"))
}

fn parse_agent_state(value: &str) -> Option<AgentState> {
    match value {
        "Created" => Some(AgentState::Created),
        "Planning" => Some(AgentState::Planning),
        "WaitingForPlanApproval" => Some(AgentState::WaitingForPlanApproval),
        "RetrievingContext" => Some(AgentState::RetrievingContext),
        "Executing" => Some(AgentState::Executing),
        "WaitingForToolApproval" => Some(AgentState::WaitingForToolApproval),
        "ApplyingPatch" => Some(AgentState::ApplyingPatch),
        "RunningCommand" => Some(AgentState::RunningCommand),
        "DiagnosingFailure" => Some(AgentState::DiagnosingFailure),
        "Reviewing" => Some(AgentState::Reviewing),
        "WaitingForUser" => Some(AgentState::WaitingForUser),
        "Completed" => Some(AgentState::Completed),
        "Failed" => Some(AgentState::Failed),
        "Cancelled" => Some(AgentState::Cancelled),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{run_no_model_coding_fixture, NoModelCodingFixtureConfig};
    use crate::native_agent_loop::run_scripted_native_agent_loop_external_block_fixture;

    #[test]
    fn replay_completed_fixture_snapshot() {
        let result = run_no_model_coding_fixture(&NoModelCodingFixtureConfig::default()).unwrap();
        let snapshot = replay_jsonl(&result.event_jsonl).unwrap();
        assert_eq!(snapshot.inferred_state, AgentState::Completed);
        assert_eq!(snapshot.health, ReplayHealth::Completed);
        assert!(snapshot.tool_calls_requested >= 1);
        assert!(snapshot.permissions_requested >= 1);
        assert!(snapshot.pending_permission_ids.is_empty());
        assert!(!snapshot.can_resume_without_user);
    }

    #[test]
    fn replay_blocked_permission_snapshot() {
        let result = run_scripted_native_agent_loop_external_block_fixture().unwrap();
        let snapshot = replay_jsonl(&result.event_jsonl).unwrap();
        assert_eq!(snapshot.inferred_state, AgentState::WaitingForToolApproval);
        assert_eq!(snapshot.health, ReplayHealth::BlockedForPermission);
        assert_eq!(snapshot.pending_permission_ids.len(), 1);
        assert!(!snapshot.can_resume_without_user);
    }
}
