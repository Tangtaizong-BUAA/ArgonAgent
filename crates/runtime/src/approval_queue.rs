//! Approval queue extraction from EventLog.
//!
//! This is the runtime/API boundary for future TUI/GUI approval drawers. It
//! keeps task governance approvals (PlanApproval) separate from safety approvals
//! (PermissionRequest).

use crate::event_log::EventLog;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalQueueItemKind {
    PlanApproval,
    Permission,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalQueueItem {
    pub id: String,
    pub kind: ApprovalQueueItemKind,
    pub request_type: Option<String>,
    pub created_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApprovalQueue {
    pub plan_approvals: Vec<ApprovalQueueItem>,
    pub permissions: Vec<ApprovalQueueItem>,
}

impl ApprovalQueue {
    pub fn is_empty(&self) -> bool {
        self.plan_approvals.is_empty() && self.permissions.is_empty()
    }

    pub fn total_pending(&self) -> usize {
        self.plan_approvals.len() + self.permissions.len()
    }

    pub fn to_summary_line(&self) -> String {
        format!(
            "approval queue pending={} plan={} permission={}",
            self.total_pending(),
            self.plan_approvals.len(),
            self.permissions.len()
        )
    }
}

pub fn extract_approval_queue(log: &EventLog) -> ApprovalQueue {
    let mut queue = ApprovalQueue::default();
    for event in log.iter() {
        match event.event_type.as_str() {
            "plan.approval_requested" => {
                if let Some(plan_approval_id) =
                    extract_json_string(&event.payload_json, "plan_approval_id")
                {
                    upsert_item(
                        &mut queue.plan_approvals,
                        ApprovalQueueItem {
                            id: plan_approval_id,
                            kind: ApprovalQueueItemKind::PlanApproval,
                            request_type: None,
                            created_sequence: event.sequence,
                        },
                    );
                }
            }
            "plan.approval_decided" => {
                if let Some(plan_approval_id) =
                    extract_json_string(&event.payload_json, "plan_approval_id")
                {
                    queue
                        .plan_approvals
                        .retain(|item| item.id != plan_approval_id);
                }
            }
            "permission.requested" => {
                if let Some(permission_id) =
                    extract_json_string(&event.payload_json, "permission_id")
                {
                    upsert_item(
                        &mut queue.permissions,
                        ApprovalQueueItem {
                            id: permission_id,
                            kind: ApprovalQueueItemKind::Permission,
                            request_type: extract_json_string(&event.payload_json, "request_type"),
                            created_sequence: event.sequence,
                        },
                    );
                }
            }
            "permission.decided" => {
                if let Some(permission_id) =
                    extract_json_string(&event.payload_json, "permission_id")
                {
                    queue.permissions.retain(|item| item.id != permission_id);
                }
            }
            _ => {}
        }
    }
    queue
}

fn upsert_item(items: &mut Vec<ApprovalQueueItem>, item: ApprovalQueueItem) {
    items.retain(|candidate| candidate.id != item.id);
    items.push(item);
}

fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let start = json.find(&marker)? + marker.len();
    let rest = &json[start..];
    let end = rest.find('"')?;
    Some(rest[..end].replace("\\\"", "\"").replace("\\\\", "\\"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_log::EventLog;
    use crate::native_agent_loop::run_scripted_native_agent_loop_external_block_fixture;
    use crate::session::AgentSession;
    use crate::state::AgentState;

    #[test]
    fn extracts_pending_permission_from_blocked_loop() {
        let result = run_scripted_native_agent_loop_external_block_fixture().unwrap();
        let log = EventLog::import_jsonl(&result.event_jsonl).unwrap();
        let queue = extract_approval_queue(&log);
        assert_eq!(queue.plan_approvals.len(), 0);
        assert_eq!(queue.permissions.len(), 1);
        assert_eq!(
            queue.permissions[0].request_type.as_deref(),
            Some("file_write")
        );
    }

    #[test]
    fn keeps_plan_approval_separate_from_permission() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session.request_plan_approval("plan_1", None).unwrap();
        let log = EventLog::import_jsonl(&session.export_events_jsonl()).unwrap();
        let queue = extract_approval_queue(&log);
        assert_eq!(queue.plan_approvals.len(), 1);
        assert_eq!(queue.permissions.len(), 0);
        assert_eq!(
            queue.plan_approvals[0].kind,
            ApprovalQueueItemKind::PlanApproval
        );
    }
}
