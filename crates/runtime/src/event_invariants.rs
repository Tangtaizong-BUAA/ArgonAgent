//! Event-log invariant checks beyond JSONL parsing.
//!
//! `EventLog` already guarantees monotonic sequence numbers and hash chaining.
//! This module validates runtime semantics that matter for resuming sessions and
//! auditing agent safety boundaries.

use crate::event_log::EventLog;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventInvariantReport {
    pub ok: bool,
    pub checked_events: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl EventInvariantReport {
    pub fn to_summary_line(&self) -> String {
        format!(
            "event invariants ok={} events={} errors={} warnings={}",
            self.ok,
            self.checked_events,
            self.errors.len(),
            self.warnings.len()
        )
    }
}

#[derive(Debug, Default)]
struct PatchState {
    validated_pass: bool,
    applied: bool,
}

pub fn validate_event_invariants(log: &EventLog) -> EventInvariantReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut session_created = false;
    let mut tool_requested = HashSet::<String>::new();
    let mut tool_completed = HashSet::<String>::new();
    let mut tool_result_recorded = HashSet::<String>::new();
    let mut model_started = HashSet::<String>::new();
    let mut model_completed = HashSet::<String>::new();
    let mut streams_with_delta = HashSet::<String>::new();
    let mut streams_completed = HashSet::<String>::new();
    let mut pending_permissions = HashMap::<String, String>::new();
    let mut allowed_file_write_seen = false;
    let mut patches = HashMap::<String, PatchState>::new();
    let mut loop_terminal_by_id = HashMap::<String, u64>::new();
    let mut active_native_loop_id: Option<String> = None;
    let mut terminal_required_by_id = HashMap::<String, u64>::new();

    for event in log.iter() {
        if event.event_type != "session.created" && !session_created {
            errors.push(format!(
                "event {} appears before session.created: {}",
                event.sequence, event.event_type
            ));
        }
        match event.event_type.as_str() {
            "session.created" => {
                if session_created {
                    errors.push(format!(
                        "duplicate session.created at event {}",
                        event.sequence
                    ));
                }
                session_created = true;
            }
            "tool.call_requested" => {
                let Some(tool_call_id) = extract_json_string(&event.payload_json, "tool_call_id")
                else {
                    errors.push(format!(
                        "tool.call_requested missing tool_call_id at {}",
                        event.sequence
                    ));
                    continue;
                };
                if !tool_requested.insert(tool_call_id.clone()) {
                    errors.push(format!("duplicate tool.call_requested id {tool_call_id}"));
                }
            }
            "tool.call_completed" => {
                let Some(tool_call_id) = extract_json_string(&event.payload_json, "tool_call_id")
                else {
                    errors.push(format!(
                        "tool.call_completed missing tool_call_id at {}",
                        event.sequence
                    ));
                    continue;
                };
                if !tool_requested.contains(&tool_call_id) {
                    errors.push(format!(
                        "tool.call_completed without request: {tool_call_id}"
                    ));
                }
                if !tool_completed.insert(tool_call_id.clone()) {
                    errors.push(format!("duplicate tool.call_completed id {tool_call_id}"));
                }
            }
            "tool.result_recorded" => {
                let Some(tool_call_id) = extract_json_string(&event.payload_json, "tool_call_id")
                else {
                    errors.push(format!(
                        "tool.result_recorded missing tool_call_id at {}",
                        event.sequence
                    ));
                    continue;
                };
                if !tool_completed.contains(&tool_call_id) {
                    errors.push(format!(
                        "tool.result_recorded before completion: {tool_call_id}"
                    ));
                }
                if !tool_result_recorded.insert(tool_call_id.clone()) {
                    errors.push(format!("duplicate tool.result_recorded id {tool_call_id}"));
                }
            }
            "model.call_started" => {
                let Some(call_id) = extract_json_string(&event.payload_json, "call_id") else {
                    errors.push(format!(
                        "model.call_started missing call_id at {}",
                        event.sequence
                    ));
                    continue;
                };
                if !model_started.insert(call_id.clone()) {
                    errors.push(format!("duplicate model.call_started id {call_id}"));
                }
            }
            "model.stream_delta" => {
                let Some(stream_id) = extract_json_string(&event.payload_json, "stream_id") else {
                    errors.push(format!(
                        "model.stream_delta missing stream_id at {}",
                        event.sequence
                    ));
                    continue;
                };
                streams_with_delta.insert(stream_id);
                if event.payload_json.contains("sk-") {
                    errors.push(format!(
                        "model.stream_delta contains raw API-key-like token at {}",
                        event.sequence
                    ));
                }
            }
            "model.stream_completed" => {
                let Some(stream_id) = extract_json_string(&event.payload_json, "stream_id") else {
                    errors.push(format!(
                        "model.stream_completed missing stream_id at {}",
                        event.sequence
                    ));
                    continue;
                };
                if !streams_with_delta.contains(&stream_id) {
                    warnings.push(format!("stream completed without delta: {stream_id}"));
                }
                if !streams_completed.insert(stream_id.clone()) {
                    errors.push(format!("duplicate model.stream_completed id {stream_id}"));
                }
            }
            "model.call_completed" => {
                let Some(call_id) = extract_json_string(&event.payload_json, "call_id") else {
                    errors.push(format!(
                        "model.call_completed missing call_id at {}",
                        event.sequence
                    ));
                    continue;
                };
                if !model_started.contains(&call_id) {
                    errors.push(format!("model.call_completed without start: {call_id}"));
                }
                if !model_completed.insert(call_id.clone()) {
                    errors.push(format!("duplicate model.call_completed id {call_id}"));
                }
            }
            "permission.requested" => {
                if event.payload_json.contains("\"request_type\":\"plan\"") {
                    errors.push("plan approval represented as PermissionRequest".to_string());
                }
                let Some(permission_id) = extract_json_string(&event.payload_json, "permission_id")
                else {
                    errors.push(format!(
                        "permission.requested missing permission_id at {}",
                        event.sequence
                    ));
                    continue;
                };
                let request_type = extract_json_string(&event.payload_json, "request_type")
                    .unwrap_or_else(|| "unknown".to_string());
                if pending_permissions
                    .insert(permission_id.clone(), request_type)
                    .is_some()
                {
                    errors.push(format!("duplicate pending permission id {permission_id}"));
                }
            }
            "permission.decided" => {
                let Some(permission_id) = extract_json_string(&event.payload_json, "permission_id")
                else {
                    errors.push(format!(
                        "permission.decided missing permission_id at {}",
                        event.sequence
                    ));
                    continue;
                };
                let Some(request_type) = pending_permissions.remove(&permission_id) else {
                    errors.push(format!(
                        "permission.decided without request: {permission_id}"
                    ));
                    continue;
                };
                let decision = extract_json_string(&event.payload_json, "decision")
                    .unwrap_or_else(|| "unknown".to_string());
                if request_type == "file_write"
                    && matches!(
                        decision.as_str(),
                        "allow_once" | "allow_session" | "allow_project_rule"
                    )
                {
                    allowed_file_write_seen = true;
                }
            }
            "plan.approval_requested" => {
                if event.payload_json.contains("\"request_type\"") {
                    errors.push(
                        "PlanApprovalRequest must not include permission request_type".to_string(),
                    );
                }
            }
            "turn.route.classified" => {
                let turn_id = extract_json_string(&event.payload_json, "turn_id")
                    .unwrap_or_else(|| "legacy-global".to_string());
                active_native_loop_id = Some(turn_id);
            }
            "session.state_changed" => {
                let to_state =
                    extract_json_string(&event.payload_json, "to_state").unwrap_or_default();
                if matches!(
                    to_state.as_str(),
                    "Completed"
                        | "Failed"
                        | "Cancelled"
                        | "WaitingForUser"
                        | "WaitingForToolApproval"
                        | "WaitingForPlanApproval"
                ) {
                    if let Some(loop_id) = active_native_loop_id.as_ref() {
                        terminal_required_by_id
                            .entry(loop_id.clone())
                            .or_insert(event.sequence);
                    }
                }
            }
            "agent.loop_stopped" | "agent.loop_incomplete" => {
                let loop_id = extract_json_string(&event.payload_json, "loop_id")
                    .or_else(|| active_native_loop_id.clone())
                    .unwrap_or_else(|| "legacy-global".to_string());
                terminal_required_by_id
                    .entry(loop_id)
                    .or_insert(event.sequence);
            }
            "agent.loop_state.terminal" => {
                let loop_id = extract_json_string(&event.payload_json, "loop_id")
                    .unwrap_or_else(|| "legacy-global".to_string());
                if let Some(previous_sequence) =
                    loop_terminal_by_id.insert(loop_id.clone(), event.sequence)
                {
                    errors.push(format!(
                        "duplicate agent.loop_state.terminal for loop_id {loop_id} at {} after {}",
                        event.sequence, previous_sequence
                    ));
                }
            }
            "patch.proposal_created" => {
                let Some(patch_id) = extract_json_string(&event.payload_json, "patch_id") else {
                    errors.push(format!(
                        "patch.proposal_created missing patch_id at {}",
                        event.sequence
                    ));
                    continue;
                };
                patches.entry(patch_id).or_default();
            }
            "patch.proposal_validated" => {
                let Some(patch_id) = extract_json_string(&event.payload_json, "patch_id") else {
                    errors.push(format!(
                        "patch.proposal_validated missing patch_id at {}",
                        event.sequence
                    ));
                    continue;
                };
                let validation = extract_json_string(&event.payload_json, "validation")
                    .unwrap_or_else(|| "unknown".to_string());
                let state = patches.entry(patch_id).or_default();
                state.validated_pass = matches!(validation.as_str(), "pass" | "pass_create");
            }
            "patch.applied" => {
                let Some(patch_id) = extract_json_string(&event.payload_json, "patch_id") else {
                    errors.push(format!(
                        "patch.applied missing patch_id at {}",
                        event.sequence
                    ));
                    continue;
                };
                let state = patches.entry(patch_id.clone()).or_default();
                if !state.validated_pass {
                    errors.push(format!("patch.applied before pass validation: {patch_id}"));
                }
                if !allowed_file_write_seen {
                    errors.push(format!(
                        "patch.applied before allowed file_write permission: {patch_id}"
                    ));
                }
                if state.applied {
                    errors.push(format!("duplicate patch.applied id {patch_id}"));
                }
                state.applied = true;
            }
            _ => {}
        }
    }

    for tool_call_id in tool_completed.difference(&tool_result_recorded) {
        warnings.push(format!(
            "tool completed without recorded result: {tool_call_id}"
        ));
    }
    for call_id in model_started.difference(&model_completed) {
        warnings.push(format!("model call started without completion: {call_id}"));
    }
    for (loop_id, sequence) in terminal_required_by_id {
        if !loop_terminal_by_id.contains_key(&loop_id) {
            errors.push(format!(
                "native loop terminal state missing agent.loop_state.terminal for loop_id {loop_id} at {sequence}"
            ));
        }
    }

    EventInvariantReport {
        ok: errors.is_empty(),
        checked_events: log.len(),
        errors,
        warnings,
    }
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
    use crate::executor::{run_no_model_coding_fixture, NoModelCodingFixtureConfig};
    use crate::native_agent_loop::run_scripted_native_agent_loop_external_block_fixture;
    use researchcode_kernel::{Actor, KernelEvent};

    #[test]
    fn validates_completed_coding_fixture() {
        let result = run_no_model_coding_fixture(&NoModelCodingFixtureConfig::default()).unwrap();
        let log = EventLog::import_jsonl(&result.event_jsonl).unwrap();
        let report = validate_event_invariants(&log);
        assert_eq!(report.errors, Vec::<String>::new());
        assert!(report.ok, "{:?}", report);
    }

    #[test]
    fn validates_blocked_permission_fixture_without_patch_apply() {
        let result = run_scripted_native_agent_loop_external_block_fixture().unwrap();
        let log = EventLog::import_jsonl(&result.event_jsonl).unwrap();
        let report = validate_event_invariants(&log);
        assert!(report.ok, "{:?}", report);
    }

    #[test]
    fn rejects_multiple_authoritative_loop_terminal_events() {
        let mut log = EventLog::default();
        append_test_event(&mut log, 1, "session.created", "{\"session_id\":\"sess\"}");
        append_test_event(
            &mut log,
            2,
            "agent.loop_state.terminal",
            "{\"loop_id\":\"turn_1\",\"status\":\"blocked\",\"reason\":\"max_iterations\",\"category\":\"turn_budget\"}",
        );
        append_test_event(
            &mut log,
            3,
            "agent.loop_state.terminal",
            "{\"loop_id\":\"turn_1\",\"status\":\"failed\",\"reason\":\"late_failure\",\"category\":\"provider_failure\"}",
        );

        let report = validate_event_invariants(&log);
        assert!(!report.ok);
        assert!(report.errors.iter().any(|error| error
            .contains("duplicate agent.loop_state.terminal for loop_id turn_1 at 3 after 2")));
    }

    #[test]
    fn allows_one_authoritative_loop_terminal_per_loop_id() {
        let mut log = EventLog::default();
        append_test_event(&mut log, 1, "session.created", "{\"session_id\":\"sess\"}");
        append_test_event(
            &mut log,
            2,
            "agent.loop_state.terminal",
            "{\"loop_id\":\"turn_1\",\"status\":\"blocked\",\"reason\":\"max_iterations\",\"category\":\"turn_budget\"}",
        );
        append_test_event(
            &mut log,
            3,
            "agent.loop_state.terminal",
            "{\"loop_id\":\"turn_2\",\"status\":\"completed\",\"reason\":\"model_visible_answer\",\"category\":\"model_answer\"}",
        );

        let report = validate_event_invariants(&log);
        assert!(report.ok, "{:?}", report);
    }

    #[test]
    fn allows_terminal_duplicate_suppressed_diagnostic() {
        let mut log = EventLog::default();
        append_test_event(&mut log, 1, "session.created", "{\"session_id\":\"sess\"}");
        append_test_event(
            &mut log,
            2,
            "agent.loop_state.terminal",
            "{\"loop_id\":\"turn_1\",\"status\":\"blocked\",\"reason\":\"max_iterations\",\"category\":\"turn_budget\"}",
        );
        append_test_event(
            &mut log,
            3,
            "agent.loop_state.terminal_duplicate_suppressed",
            "{\"loop_id\":\"turn_1\",\"existing_reason\":\"max_iterations\",\"requested_reason\":\"late_failure\"}",
        );

        let report = validate_event_invariants(&log);
        assert!(report.ok, "{:?}", report);
    }

    #[test]
    fn rejects_native_loop_terminal_state_without_terminal_event() {
        let mut log = EventLog::default();
        append_test_event(&mut log, 1, "session.created", "{\"session_id\":\"sess\"}");
        append_test_event(
            &mut log,
            2,
            "turn.route.classified",
            "{\"turn_id\":\"turn_1\",\"route\":\"ReadOnlyExplore\",\"strategy\":\"deterministic_rules\"}",
        );
        append_test_event(
            &mut log,
            3,
            "session.state_changed",
            "{\"from_state\":\"Executing\",\"to_state\":\"Completed\"}",
        );

        let report = validate_event_invariants(&log);
        assert!(!report.ok);
        assert!(report.errors.iter().any(|error| error.contains(
            "native loop terminal state missing agent.loop_state.terminal for loop_id turn_1"
        )));
    }

    #[test]
    fn allows_in_progress_native_loop_without_terminal_event() {
        let mut log = EventLog::default();
        append_test_event(&mut log, 1, "session.created", "{\"session_id\":\"sess\"}");
        append_test_event(
            &mut log,
            2,
            "turn.route.classified",
            "{\"turn_id\":\"turn_1\",\"route\":\"ReadOnlyExplore\",\"strategy\":\"deterministic_rules\"}",
        );
        append_test_event(
            &mut log,
            3,
            "session.state_changed",
            "{\"from_state\":\"RetrievingContext\",\"to_state\":\"Executing\"}",
        );

        let report = validate_event_invariants(&log);
        assert!(report.ok, "{:?}", report);
    }

    #[test]
    fn rejects_second_native_turn_missing_terminal_even_when_first_turn_has_terminal() {
        let mut log = EventLog::default();
        append_test_event(&mut log, 1, "session.created", "{\"session_id\":\"sess\"}");
        append_test_event(
            &mut log,
            2,
            "turn.route.classified",
            "{\"turn_id\":\"turn_1\",\"route\":\"ReadOnlyExplore\",\"strategy\":\"deterministic_rules\"}",
        );
        append_test_event(
            &mut log,
            3,
            "session.state_changed",
            "{\"from_state\":\"Executing\",\"to_state\":\"Completed\"}",
        );
        append_test_event(
            &mut log,
            4,
            "agent.loop_state.terminal",
            "{\"loop_id\":\"turn_1\",\"status\":\"completed\",\"reason\":\"model_visible_answer\",\"category\":\"model_answer\"}",
        );
        append_test_event(
            &mut log,
            5,
            "turn.route.classified",
            "{\"turn_id\":\"turn_2\",\"route\":\"CodeEdit\",\"strategy\":\"deterministic_rules\"}",
        );
        append_test_event(
            &mut log,
            6,
            "session.state_changed",
            "{\"from_state\":\"Executing\",\"to_state\":\"WaitingForToolApproval\"}",
        );

        let report = validate_event_invariants(&log);
        assert!(!report.ok);
        assert!(report.errors.iter().any(|error| error.contains(
            "native loop terminal state missing agent.loop_state.terminal for loop_id turn_2"
        )));
    }

    #[test]
    fn allows_later_native_turn_still_executing_after_previous_terminal_turn() {
        let mut log = EventLog::default();
        append_test_event(&mut log, 1, "session.created", "{\"session_id\":\"sess\"}");
        append_test_event(
            &mut log,
            2,
            "turn.route.classified",
            "{\"turn_id\":\"turn_1\",\"route\":\"ReadOnlyExplore\",\"strategy\":\"deterministic_rules\"}",
        );
        append_test_event(
            &mut log,
            3,
            "session.state_changed",
            "{\"from_state\":\"Executing\",\"to_state\":\"Completed\"}",
        );
        append_test_event(
            &mut log,
            4,
            "agent.loop_state.terminal",
            "{\"loop_id\":\"turn_1\",\"status\":\"completed\",\"reason\":\"model_visible_answer\",\"category\":\"model_answer\"}",
        );
        append_test_event(
            &mut log,
            5,
            "turn.route.classified",
            "{\"turn_id\":\"turn_2\",\"route\":\"CodeEdit\",\"strategy\":\"deterministic_rules\"}",
        );
        append_test_event(
            &mut log,
            6,
            "session.state_changed",
            "{\"from_state\":\"RetrievingContext\",\"to_state\":\"Executing\"}",
        );

        let report = validate_event_invariants(&log);
        assert!(report.ok, "{:?}", report);
    }

    fn append_test_event(log: &mut EventLog, sequence: u64, event_type: &str, payload_json: &str) {
        let prev_hash = if sequence > 1 {
            Some(format!("h{}", sequence - 1))
        } else {
            None
        };
        log.append(KernelEvent {
            event_id: format!("evt_{sequence}"),
            schema_version: "1".to_string(),
            project_id: "proj".to_string(),
            session_id: Some("sess".to_string()),
            task_id: Some("task".to_string()),
            sequence,
            event_type: event_type.to_string(),
            actor: Actor::Runtime,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            payload_json: payload_json.to_string(),
            prev_hash,
            hash: format!("h{sequence}"),
        })
        .unwrap();
    }
}
