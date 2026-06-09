//! Native loop turn ledger and DeepSeek context preflight guards.
//!
//! The controller is intentionally small: it records replayable turn boundaries
//! and blocks DeepSeek requests before they exceed the empirically safe 256K
//! window. Native loop orchestration remains in `native_agent_loop`.

use crate::agent_kernel::compactor::Compactor;
use crate::agent_kernel::context_spine::ContextSpineState;
use crate::compaction::CompactionSummary;
use crate::context_budget::{ContextBudget, DEEPSEEK_TARGET_MODEL_CALL_TOKENS};
use crate::live_model_request::PreparedModelHttpRequest;
use crate::session::AgentSession;
use researchcode_kernel::model::NativeModelFamily;
use researchcode_kernel::Actor;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeContextGuardAction {
    Send,
    CompactionRequired,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeContextGuardReport {
    pub action: NativeContextGuardAction,
    pub call_id: String,
    pub stage: String,
    pub estimated_request_tokens: u64,
    pub estimated_total_tokens: u64,
    pub hard_limit_tokens: u64,
    pub target_limit_tokens: u64,
    pub compaction_threshold_tokens: u64,
    pub compaction_summary: Option<CompactionSummary>,
    pub compaction_spine: Option<ContextSpineState>,
    pub compaction_marker: Option<String>,
    pub compaction_preserved_messages: Vec<String>,
}

impl NativeContextGuardReport {
    pub fn should_send(&self) -> bool {
        self.action == NativeContextGuardAction::Send
    }
}

#[derive(Debug, Clone)]
pub struct NativeTurnController {
    turn_id: String,
    pending_tools: BTreeSet<String>,
    completed_tools: BTreeSet<String>,
    pending_permissions: BTreeSet<String>,
    recovery_count: usize,
}

impl NativeTurnController {
    pub fn new(turn_id: &str) -> Self {
        Self::new_with_turn_id(turn_id.to_string())
    }

    pub fn new_for_session(session: &AgentSession, session_id: &str) -> Result<Self, String> {
        let turn_id = session.current_turn_id().ok_or_else(|| {
            format!("NativeTurnController requires current_turn_id for session {session_id}; call begin_interactive_turn first")
        })?;
        Ok(Self::new_with_turn_id(turn_id.to_string()))
    }

    fn new_with_turn_id(turn_id: String) -> Self {
        Self {
            turn_id,
            pending_tools: BTreeSet::new(),
            completed_tools: BTreeSet::new(),
            pending_permissions: BTreeSet::new(),
            recovery_count: 0,
        }
    }

    pub fn record_turn_started(
        &self,
        session: &mut AgentSession,
        model_family: &NativeModelFamily,
        max_iterations: usize,
        max_tool_calls: usize,
    ) -> Result<(), String> {
        session
            .record_runtime_event(
                "agent.turn.started",
                Actor::Runtime,
                format!(
                    "{{\"turn_id\":{},\"model_family\":{},\"max_iterations\":{},\"max_tool_calls\":{}}}",
                    json_string(&self.turn_id),
                    json_string(model_family_label(model_family)),
                    max_iterations,
                    max_tool_calls
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn record_ledger_update(
        &self,
        session: &mut AgentSession,
        phase: &str,
    ) -> Result<(), String> {
        session
            .record_runtime_event(
                "agent.turn.ledger_updated",
                Actor::Runtime,
                format!(
                    "{{\"turn_id\":{},\"phase\":{},\"pending_tools\":{},\"completed_tools\":{},\"pending_permissions\":{},\"recoveries\":{}}}",
                    json_string(&self.turn_id),
                    json_string(phase),
                    self.pending_tools.len(),
                    self.completed_tools.len(),
                    self.pending_permissions.len(),
                    self.recovery_count
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn record_tool_pending(
        &mut self,
        session: &mut AgentSession,
        tool_call_id: &str,
        tool_id: &str,
        iteration: usize,
    ) -> Result<(), String> {
        if self.completed_tools.contains(tool_call_id) || self.pending_tools.contains(tool_call_id)
        {
            self.recovery_count += 1;
            session
                .record_runtime_event(
                    "agent.recovery.started",
                    Actor::Runtime,
                    format!(
                        "{{\"turn_id\":{},\"reason\":\"duplicate_tool_call_id\",\"tool_call_id\":{},\"tool_id\":{},\"iteration\":{},\"recoveries\":{}}}",
                        json_string(&self.turn_id),
                        json_string(tool_call_id),
                        json_string(tool_id),
                        iteration,
                        self.recovery_count
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
            return Err(format!(
                "duplicate tool_call_id in native turn ledger: {tool_call_id}"
            ));
        }
        self.pending_tools.insert(tool_call_id.to_string());
        session
            .record_runtime_event(
                "agent.tool.pending",
                Actor::Runtime,
                format!(
                    "{{\"turn_id\":{},\"tool_call_id\":{},\"tool_id\":{},\"iteration\":{}}}",
                    json_string(&self.turn_id),
                    json_string(tool_call_id),
                    json_string(tool_id),
                    iteration
                ),
            )
            .and_then(|_| {
                session.record_runtime_event(
                    "agent.turn.ledger_updated",
                    Actor::Runtime,
                    format!(
                        "{{\"turn_id\":{},\"phase\":\"tool_pending\",\"pending_tools\":{},\"completed_tools\":{},\"pending_permissions\":{},\"recoveries\":{}}}",
                        json_string(&self.turn_id),
                        self.pending_tools.len(),
                        self.completed_tools.len(),
                        self.pending_permissions.len(),
                        self.recovery_count
                    ),
                )
            })
            .map_err(|error| format!("{error:?}"))
    }

    pub fn record_tool_completed(
        &mut self,
        session: &mut AgentSession,
        tool_call_id: &str,
        tool_id: &str,
        ok: bool,
    ) -> Result<(), String> {
        self.pending_tools.remove(tool_call_id);
        self.completed_tools.insert(tool_call_id.to_string());
        session
            .record_runtime_event(
                "agent.tool.completed",
                Actor::Runtime,
                format!(
                    "{{\"turn_id\":{},\"tool_call_id\":{},\"tool_id\":{},\"ok\":{},\"pending_tools\":{},\"completed_tools\":{}}}",
                    json_string(&self.turn_id),
                    json_string(tool_call_id),
                    json_string(tool_id),
                    ok,
                    self.pending_tools.len(),
                    self.completed_tools.len()
                ),
            )
            .and_then(|_| {
                session.record_runtime_event(
                    "agent.turn.ledger_updated",
                    Actor::Runtime,
                    format!(
                        "{{\"turn_id\":{},\"phase\":\"tool_completed\",\"pending_tools\":{},\"completed_tools\":{},\"pending_permissions\":{},\"recoveries\":{}}}",
                        json_string(&self.turn_id),
                        self.pending_tools.len(),
                        self.completed_tools.len(),
                        self.pending_permissions.len(),
                        self.recovery_count
                    ),
                )
            })
            .map_err(|error| format!("{error:?}"))
    }

    pub fn record_permission_pending(
        &mut self,
        session: &mut AgentSession,
        permission_id: &str,
        request_type: &str,
    ) -> Result<(), String> {
        self.pending_permissions.insert(permission_id.to_string());
        session
            .record_runtime_event(
                "agent.permission.pending",
                Actor::Runtime,
                format!(
                    "{{\"turn_id\":{},\"permission_id\":{},\"request_type\":{},\"pending_permissions\":{}}}",
                    json_string(&self.turn_id),
                    json_string(permission_id),
                    json_string(request_type),
                    self.pending_permissions.len()
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn record_recovery_started(
        &mut self,
        session: &mut AgentSession,
        reason: &str,
        iteration: usize,
    ) -> Result<(), String> {
        self.recovery_count += 1;
        session
            .record_runtime_event(
                "agent.recovery.started",
                Actor::Runtime,
                format!(
                    "{{\"turn_id\":{},\"reason\":{},\"iteration\":{},\"recoveries\":{}}}",
                    json_string(&self.turn_id),
                    json_string(reason),
                    iteration,
                    self.recovery_count
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn ensure_can_complete(
        &self,
        session: &mut AgentSession,
        reason: &str,
    ) -> Result<(), String> {
        if self.pending_tools.is_empty() && self.pending_permissions.is_empty() {
            return Ok(());
        }
        session
            .record_runtime_event(
                "agent.recovery.started",
                Actor::Runtime,
                format!(
                    "{{\"turn_id\":{},\"reason\":{},\"pending_tools\":{},\"pending_permissions\":{}}}",
                    json_string(&self.turn_id),
                    json_string(reason),
                    self.pending_tools.len(),
                    self.pending_permissions.len()
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        Err(format!(
            "native turn cannot complete with pending ledger entries: tools={} permissions={}",
            self.pending_tools.len(),
            self.pending_permissions.len()
        ))
    }
}

pub fn evaluate_native_context_guard(
    session: &mut AgentSession,
    family: &NativeModelFamily,
    budget: &ContextBudget,
    call_id: &str,
    stage: &str,
    prepared: &PreparedModelHttpRequest,
) -> Result<NativeContextGuardReport, String> {
    let estimated_request_tokens = estimate_tokens(&prepared.body_json);
    let requested_output_tokens = extract_max_tokens(&prepared.body_json)
        .unwrap_or(budget.output_reserve_tokens)
        .min(budget.output_reserve_tokens);
    let estimated_total_tokens = estimated_request_tokens + requested_output_tokens;
    let target_limit_tokens = if *family == NativeModelFamily::DeepSeek {
        DEEPSEEK_TARGET_MODEL_CALL_TOKENS.min(budget.max_context_tokens)
    } else {
        budget.max_context_tokens
    };
    let hard_limit_tokens = budget.max_context_tokens;
    session
        .record_runtime_event(
            "model.context_budget",
            Actor::Runtime,
            format!(
                "{{\"call_id\":{},\"stage\":{},\"model_family\":{},\"estimated_request_tokens\":{},\"estimated_total_tokens\":{},\"target_limit_tokens\":{},\"hard_limit_tokens\":{},\"compaction_threshold_tokens\":{}}}",
                json_string(call_id),
                json_string(stage),
                json_string(model_family_label(family)),
                estimated_request_tokens,
                estimated_total_tokens,
                target_limit_tokens,
                hard_limit_tokens,
                budget.compaction_threshold
            ),
        )
        .map_err(|error| format!("{error:?}"))?;

    let blocked =
        *family == NativeModelFamily::DeepSeek && estimated_total_tokens > target_limit_tokens;
    if blocked {
        session
            .record_runtime_event(
                "context.compaction.blocked",
                Actor::Runtime,
                format!(
                    "{{\"call_id\":{},\"stage\":{},\"reason\":\"deepseek_context_budget_exceeded\",\"token_estimate\":{},\"target_limit_tokens\":{},\"hard_limit_tokens\":{}}}",
                    json_string(call_id),
                    json_string(stage),
                    estimated_total_tokens,
                    target_limit_tokens,
                    hard_limit_tokens
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
    }
    let mut compaction_summary = None;
    let mut compaction_spine = None;
    let mut compaction_marker = None;
    let mut compaction_preserved_messages = Vec::new();
    if !blocked
        && *family == NativeModelFamily::DeepSeek
        && estimated_total_tokens > budget.compaction_threshold
    {
        session
            .record_runtime_event(
                "context.compaction.started",
                Actor::Runtime,
                format!(
                    "{{\"call_id\":{},\"stage\":{},\"reason\":\"deepseek_preflight_threshold\",\"token_estimate_before\":{},\"threshold\":{}}}",
                    json_string(call_id),
                    json_string(stage),
                    estimated_total_tokens,
                    budget.compaction_threshold
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        session
            .record_runtime_event(
                "context.compaction.projected",
                Actor::Runtime,
                format!(
                    "{{\"call_id\":{},\"stage\":{},\"strategy\":\"deterministic_projection\",\"model_family\":\"deepseek\",\"reason\":\"deepseek_preflight_threshold\",\"token_estimate_before\":{}}}",
                    json_string(call_id),
                    json_string(stage),
                    estimated_total_tokens
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        let compactor = Compactor::default();
        let compaction = compactor.compact(
            session.event_log(),
            estimated_total_tokens,
            "deepseek_preflight",
        );
        compaction_marker = Some(compaction.marker.to_string());
        compaction_preserved_messages = compaction.projection.preserved_messages.clone();
        compaction_spine = Some(compaction.spine);
        compaction_summary = Some(compaction.summary);
    } else if !blocked
        && *family == NativeModelFamily::DeepSeek
        && estimated_total_tokens <= budget.compaction_threshold
        && !stage.starts_with("compacted_")
    {
        session
            .record_runtime_event(
                "context.compaction.skipped",
                Actor::Runtime,
                format!(
                    "{{\"call_id\":{},\"stage\":{},\"token_estimate_before\":{},\"compaction_threshold_tokens\":{},\"reason\":\"below_threshold\"}}",
                    json_string(call_id),
                    json_string(stage),
                    estimated_total_tokens,
                    budget.compaction_threshold
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
    }

    Ok(NativeContextGuardReport {
        action: if blocked {
            NativeContextGuardAction::Blocked
        } else if compaction_summary.is_some() {
            NativeContextGuardAction::CompactionRequired
        } else {
            NativeContextGuardAction::Send
        },
        call_id: call_id.to_string(),
        stage: stage.to_string(),
        estimated_request_tokens,
        estimated_total_tokens,
        hard_limit_tokens,
        target_limit_tokens,
        compaction_threshold_tokens: budget.compaction_threshold,
        compaction_summary,
        compaction_spine,
        compaction_marker,
        compaction_preserved_messages,
    })
}

pub fn estimate_tokens(value: &str) -> u64 {
    let char_tokens = value.chars().count() as u64 / 4;
    let whitespace_tokens = value.split_whitespace().count() as u64;
    char_tokens.max(whitespace_tokens).max(1)
}

fn extract_max_tokens(body_json: &str) -> Option<u64> {
    let marker = "\"max_tokens\":";
    let start = body_json.find(marker)? + marker.len();
    let digits = body_json[start..]
        .chars()
        .skip_while(|ch| ch.is_ascii_whitespace())
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse::<u64>().ok()
}

fn model_family_label(family: &NativeModelFamily) -> &'static str {
    match family {
        NativeModelFamily::DeepSeek => "deepseek",
        NativeModelFamily::Qwen => "qwen",
    }
}

fn json_string(value: &str) -> String {
    format!("\"{}\"", json_escape(value))
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_budget::allocate_native_context_budget;
    use crate::model_adapter::ModelRole;

    #[test]
    fn deepseek_preflight_blocks_over_240k_target() {
        let mut session = AgentSession::new("p", "s", "t").unwrap();
        let budget = allocate_native_context_budget(
            NativeModelFamily::DeepSeek,
            ModelRole::Executor,
            Some(1_000_000),
        );
        let request = PreparedModelHttpRequest {
            method: "POST".to_string(),
            url: "https://api.deepseek.com".to_string(),
            authorization_env: "DEEPSEEK_API_KEY".to_string(),
            body_json: format!(
                "{{\"max_tokens\":1024,\"messages\":[{{\"role\":\"user\",\"content\":\"{}\"}}]}}",
                "x".repeat(980_000)
            ),
            stream: true,
        };
        let report = evaluate_native_context_guard(
            &mut session,
            &NativeModelFamily::DeepSeek,
            &budget,
            "call_1",
            "initial",
            &request,
        )
        .unwrap();
        assert_eq!(report.action, NativeContextGuardAction::Blocked);
        assert_eq!(report.hard_limit_tokens, 256_000);
        assert!(session
            .export_events_jsonl()
            .contains("context.compaction.blocked"));
    }

    #[test]
    fn deepseek_preflight_requires_rebuild_after_threshold_compaction() {
        let mut session = AgentSession::new("p", "s", "t").unwrap();
        let mut controller = NativeTurnController::new("turn_preflight_compaction");
        controller
            .record_turn_started(&mut session, &NativeModelFamily::DeepSeek, 4, 8)
            .unwrap();
        controller
            .record_tool_pending(&mut session, "tool_1", "file.read", 0)
            .unwrap();
        controller
            .record_tool_completed(&mut session, "tool_1", "file.read", true)
            .unwrap();
        let budget = allocate_native_context_budget(
            NativeModelFamily::DeepSeek,
            ModelRole::Executor,
            Some(1_000_000),
        );
        let request = PreparedModelHttpRequest {
            method: "POST".to_string(),
            url: "https://api.deepseek.com".to_string(),
            authorization_env: "DEEPSEEK_API_KEY".to_string(),
            body_json: format!(
                "{{\"max_tokens\":1024,\"messages\":[{{\"role\":\"user\",\"content\":\"{}\"}}]}}",
                "x".repeat(780_000)
            ),
            stream: true,
        };

        let report = evaluate_native_context_guard(
            &mut session,
            &NativeModelFamily::DeepSeek,
            &budget,
            "call_2",
            "initial",
            &request,
        )
        .unwrap();

        assert_eq!(report.action, NativeContextGuardAction::CompactionRequired);
        assert_eq!(report.compaction_threshold_tokens, 192_000);
        assert!(report.estimated_total_tokens > report.compaction_threshold_tokens);
        assert!(report.estimated_total_tokens <= report.target_limit_tokens);
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("context.compaction.started"));
        assert!(jsonl.contains("context.compaction.projected"));
        assert!(jsonl.contains("file.read"));
        assert!(!jsonl.contains("preflight_under_target_after_builder_compaction"));
        assert!(!jsonl.contains("context.compaction.blocked"));
        assert!(report.compaction_marker.as_deref() == Some("[compacted-context]"));
        assert!(!report.compaction_preserved_messages.is_empty());
    }

    #[test]
    fn deepseek_preflight_records_skipped_below_threshold() {
        let mut session = AgentSession::new("p", "s", "t").unwrap();
        let budget = allocate_native_context_budget(
            NativeModelFamily::DeepSeek,
            ModelRole::Executor,
            Some(1_000_000),
        );
        let request = PreparedModelHttpRequest {
            method: "POST".to_string(),
            url: "https://api.deepseek.com".to_string(),
            authorization_env: "DEEPSEEK_API_KEY".to_string(),
            body_json: r#"{"max_tokens":1024,"messages":[{"role":"user","content":"small"}]}"#
                .to_string(),
            stream: true,
        };

        let report = evaluate_native_context_guard(
            &mut session,
            &NativeModelFamily::DeepSeek,
            &budget,
            "call_small",
            "initial",
            &request,
        )
        .unwrap();

        assert_eq!(report.action, NativeContextGuardAction::Send);
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("context.compaction.skipped"));
        assert!(jsonl.contains("\"reason\":\"below_threshold\""));
        assert!(jsonl.contains("\"compaction_threshold_tokens\":192000"));
    }

    #[test]
    fn turn_controller_records_tool_ledger() {
        let mut session = AgentSession::new("p", "s", "t").unwrap();
        let mut controller = NativeTurnController::new("turn_tool_ledger");
        controller
            .record_turn_started(&mut session, &NativeModelFamily::DeepSeek, 4, 8)
            .unwrap();
        controller
            .record_tool_pending(&mut session, "tool_1", "file.read", 0)
            .unwrap();
        controller
            .record_tool_completed(&mut session, "tool_1", "file.read", true)
            .unwrap();
        controller
            .ensure_can_complete(&mut session, "final")
            .unwrap();
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("agent.turn.started"));
        assert!(jsonl.contains("agent.tool.pending"));
        assert!(jsonl.contains("agent.tool.completed"));
    }

    #[test]
    fn turn_controller_uses_session_turn_id_when_available() {
        let mut session = AgentSession::new("p", "s", "t").unwrap();
        session
            .begin_interactive_turn("runtime_live_turn_123", "test")
            .unwrap();
        let controller = NativeTurnController::new_for_session(&session, "s").unwrap();
        controller
            .record_turn_started(&mut session, &NativeModelFamily::DeepSeek, 4, 8)
            .unwrap();
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"turn_id\":\"runtime_live_turn_123\""));
        assert!(!jsonl.contains("native_turn_0"));
    }

    #[test]
    fn turn_controller_requires_session_turn_id() {
        let session = AgentSession::new("p", "s", "t").unwrap();
        let error = NativeTurnController::new_for_session(&session, "s").unwrap_err();
        assert!(error.contains("current_turn_id"));
    }
}
