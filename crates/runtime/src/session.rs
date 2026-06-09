//! AgentSession core: state transitions plus append-only events.

use crate::context_budget::allocate_native_context_budget;
use crate::event_log::{EventLog, EventLogError};
use crate::model_adapter::ModelRole;
use crate::patch::PatchValidation;
use crate::payload::RuntimeEventPayload;
use crate::state::{can_transition, AgentState};
use researchcode_kernel::model::NativeModelFamily;
use researchcode_kernel::{
    tool::find_tool_spec, Actor, KernelEvent, PermissionDecisionKind, PermissionRequestType,
    PlanApprovalDecisionKind,
};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    InvalidTransition { from: AgentState, to: AgentState },
    EventLog(EventLogError),
    NoPendingPlanApproval,
    NoPendingPermission,
    UnknownTool(String),
}

impl From<EventLogError> for SessionError {
    fn from(value: EventLogError) -> Self {
        Self::EventLog(value)
    }
}

#[derive(Debug)]
pub struct AgentSession {
    project_id: String,
    session_id: String,
    task_id: String,
    state: AgentState,
    event_log: EventLog,
    pending_plan_approval: Option<String>,
    pending_permission: Option<(String, PermissionRequestType)>,
    current_turn_id: Option<String>,
    active_assistant_text_blocks: HashSet<String>,
}

impl AgentSession {
    pub fn new(
        project_id: impl Into<String>,
        session_id: impl Into<String>,
        task_id: impl Into<String>,
    ) -> Result<Self, SessionError> {
        let mut session = Self {
            project_id: project_id.into(),
            session_id: session_id.into(),
            task_id: task_id.into(),
            state: AgentState::Created,
            event_log: EventLog::default(),
            pending_plan_approval: None,
            pending_permission: None,
            current_turn_id: None,
            active_assistant_text_blocks: HashSet::new(),
        };
        session.append_event(
            "session.created",
            Actor::Runtime,
            RuntimeEventPayload::Empty,
        )?;
        Ok(session)
    }

    pub fn resume_from_event_log(event_log: EventLog) -> Result<Self, SessionError> {
        let Some(last) = event_log.last() else {
            return Err(SessionError::EventLog(EventLogError::Parse(
                "cannot resume from empty event log".to_string(),
            )));
        };
        let project_id = last.project_id.clone();
        let session_id = last.session_id.clone().ok_or_else(|| {
            SessionError::EventLog(EventLogError::Parse(
                "cannot resume event log without session_id".to_string(),
            ))
        })?;
        let task_id = last.task_id.clone().ok_or_else(|| {
            SessionError::EventLog(EventLogError::Parse(
                "cannot resume event log without task_id".to_string(),
            ))
        })?;
        let mut state = AgentState::Created;
        let mut pending_plan_approval: Option<String> = None;
        let mut pending_permission: Option<(String, PermissionRequestType)> = None;
        let mut current_turn_id: Option<String> = None;
        for event in event_log.iter() {
            match event.event_type.as_str() {
                "session.turn_started" => {
                    current_turn_id = extract_json_string(&event.payload_json, "turn_id");
                }
                "session.state_changed" => {
                    if let Some(next) = extract_json_string(&event.payload_json, "to_state")
                        .and_then(|value| agent_state_from_str(&value))
                    {
                        state = next;
                    }
                }
                "plan.approval_requested" => {
                    pending_plan_approval =
                        extract_json_string(&event.payload_json, "plan_approval_id");
                }
                "plan.approval_decided" => {
                    if let Some(plan_approval_id) =
                        extract_json_string(&event.payload_json, "plan_approval_id")
                    {
                        if pending_plan_approval.as_deref() == Some(plan_approval_id.as_str()) {
                            pending_plan_approval = None;
                        }
                    }
                }
                "plan.approval_cleared" => {
                    if let Some(plan_approval_id) =
                        extract_json_string(&event.payload_json, "plan_approval_id")
                    {
                        if pending_plan_approval.as_deref() == Some(plan_approval_id.as_str()) {
                            pending_plan_approval = None;
                        }
                    }
                }
                "permission.requested" => {
                    if let (Some(permission_id), Some(request_type)) = (
                        extract_json_string(&event.payload_json, "permission_id"),
                        extract_json_string(&event.payload_json, "request_type")
                            .and_then(|value| PermissionRequestType::parse(&value)),
                    ) {
                        pending_permission = Some((permission_id, request_type));
                    }
                }
                "permission.decided" => {
                    if let Some(permission_id) =
                        extract_json_string(&event.payload_json, "permission_id")
                    {
                        if pending_permission
                            .as_ref()
                            .is_some_and(|(pending_id, _)| pending_id == &permission_id)
                        {
                            pending_permission = None;
                        }
                    }
                }
                "permission.cleared" => {
                    if let Some(permission_id) =
                        extract_json_string(&event.payload_json, "permission_id")
                    {
                        if pending_permission
                            .as_ref()
                            .is_some_and(|(pending_id, _)| pending_id == &permission_id)
                        {
                            pending_permission = None;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(Self {
            project_id,
            session_id,
            task_id,
            state,
            event_log,
            pending_plan_approval,
            pending_permission,
            current_turn_id,
            active_assistant_text_blocks: HashSet::new(),
        })
    }

    pub fn state(&self) -> AgentState {
        self.state
    }

    pub fn event_count(&self) -> usize {
        self.event_log.len()
    }

    pub fn event_log(&self) -> &EventLog {
        &self.event_log
    }

    pub fn pending_permission_id(&self) -> Option<&str> {
        self.pending_permission.as_ref().map(|(id, _)| id.as_str())
    }

    pub fn pending_plan_approval_id(&self) -> Option<&str> {
        self.pending_plan_approval.as_deref()
    }

    pub fn current_turn_id(&self) -> Option<&str> {
        self.current_turn_id.as_deref()
    }

    pub fn export_events_jsonl(&self) -> String {
        self.event_log.export_jsonl()
    }

    /// Directly set the session state from a trusted source (e.g., loop result).
    /// Skips transition validation since the state is authoritative.
    pub fn set_state(&mut self, next: AgentState) -> Result<(), SessionError> {
        let from = self.state;
        self.state = next;
        self.append_event(
            "session.state_changed",
            Actor::Runtime,
            RuntimeEventPayload::StateChanged { from, to: next },
        )
    }

    pub fn transition_to(&mut self, next: AgentState) -> Result<(), SessionError> {
        if !can_transition(self.state, next) {
            return Err(SessionError::InvalidTransition {
                from: self.state,
                to: next,
            });
        }
        let from = self.state;
        self.state = next;
        self.append_event(
            "session.state_changed",
            Actor::Runtime,
            RuntimeEventPayload::StateChanged { from, to: next },
        )
    }

    pub fn begin_interactive_turn(
        &mut self,
        turn_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<(), SessionError> {
        let turn_id = turn_id.into();
        let reason = reason.into();
        self.current_turn_id = Some(turn_id.clone());
        match self.state {
            AgentState::Completed => {
                self.append_event(
                    "session.turn_started",
                    Actor::Runtime,
                    RuntimeEventPayload::Generic {
                        json: format!(
                            "{{\"turn_id\":\"{}\",\"reason\":\"{}\",\"from_state\":\"Completed\"}}",
                            json_escape(&turn_id),
                            json_escape(&reason)
                        ),
                    },
                )?;
                let from = self.state;
                self.append_event(
                    "session.forced_transition",
                    Actor::Runtime,
                    RuntimeEventPayload::Generic {
                        json: format!(
                            "{{\"reason\":\"bypass_can_transition\",\"from\":\"Completed\",\"to\":\"Executing\"}}"
                        ),
                    },
                )?;
                self.state = AgentState::Executing;
                self.append_event(
                    "session.state_changed",
                    Actor::Runtime,
                    RuntimeEventPayload::StateChanged {
                        from,
                        to: AgentState::Executing,
                    },
                )
            }
            AgentState::Failed | AgentState::Cancelled | AgentState::WaitingForUser => {
                self.append_event(
                    "session.turn_started",
                    Actor::Runtime,
                    RuntimeEventPayload::Generic {
                        json: format!(
                            "{{\"turn_id\":\"{}\",\"reason\":\"{}\",\"from_state\":\"{:?}\"}}",
                            json_escape(&turn_id),
                            json_escape(&reason),
                            self.state
                        ),
                    },
                )?;
                let from = self.state;
                self.append_event(
                    "session.forced_transition",
                    Actor::Runtime,
                    RuntimeEventPayload::Generic {
                        json: format!(
                            "{{\"reason\":\"bypass_can_transition\",\"from\":\"{:?}\",\"to\":\"Executing\"}}",
                            from
                        ),
                    },
                )?;
                self.state = AgentState::Executing;
                self.append_event(
                    "session.state_changed",
                    Actor::Runtime,
                    RuntimeEventPayload::StateChanged {
                        from,
                        to: AgentState::Executing,
                    },
                )
            }
            _ => Ok(()),
        }
    }

    fn ensure_execution_activity(
        &mut self,
        activity: &str,
        allow_waiting_for_plan: bool,
    ) -> Result<(), SessionError> {
        if self.state == AgentState::Completed {
            self.begin_interactive_turn(activity, "new_interactive_turn")?;
        }
        if self.state != AgentState::Executing
            && !(allow_waiting_for_plan && self.state == AgentState::WaitingForPlanApproval)
        {
            self.transition_to(AgentState::Executing)?;
        }
        Ok(())
    }

    pub fn request_plan_approval(
        &mut self,
        plan_approval_id: impl Into<String>,
        goal: Option<String>,
    ) -> Result<(), SessionError> {
        if self.state == AgentState::Completed {
            self.begin_interactive_turn("plan.approval_requested", "new_interactive_turn")?;
        }
        let plan_approval_id = plan_approval_id.into();
        self.transition_to(AgentState::WaitingForPlanApproval)?;
        self.pending_plan_approval = Some(plan_approval_id.clone());
        self.append_event(
            "plan.approval_requested",
            Actor::Runtime,
            RuntimeEventPayload::PlanApprovalRequested {
                plan_approval_id,
                goal,
            },
        )
    }

    pub fn decide_plan(&mut self, decision: PlanApprovalDecisionKind) -> Result<(), SessionError> {
        let plan_approval_id = self
            .pending_plan_approval
            .clone()
            .ok_or(SessionError::NoPendingPlanApproval)?;
        self.append_event(
            "plan.approval_decided",
            Actor::User,
            RuntimeEventPayload::PlanApprovalDecided {
                plan_approval_id: plan_approval_id.clone(),
                decision: decision.clone(),
            },
        )?;
        // Clear pending only after successful event append
        self.pending_plan_approval = None;
        match decision {
            PlanApprovalDecisionKind::Approve => self.transition_to(AgentState::RetrievingContext),
            PlanApprovalDecisionKind::Reject | PlanApprovalDecisionKind::RequestRevision => {
                self.transition_to(AgentState::WaitingForUser)
            }
        }
    }

    pub fn request_permission(
        &mut self,
        permission_id: impl Into<String>,
        request_type: PermissionRequestType,
        tool_id: Option<String>,
    ) -> Result<(), SessionError> {
        if self.state == AgentState::Completed {
            self.begin_interactive_turn("permission.requested", "new_interactive_turn")?;
        }
        if self.state != AgentState::Executing {
            self.transition_to(AgentState::Executing)?;
        }
        let permission_id = permission_id.into();
        self.transition_to(AgentState::WaitingForToolApproval)?;
        self.pending_permission = Some((permission_id.clone(), request_type.clone()));
        self.append_event(
            "permission.requested",
            Actor::Runtime,
            RuntimeEventPayload::PermissionRequested {
                permission_id,
                request_type,
                tool_id,
            },
        )
    }

    pub fn decide_permission(
        &mut self,
        decision: PermissionDecisionKind,
    ) -> Result<(), SessionError> {
        let (permission_id, request_type) = self
            .pending_permission
            .clone()
            .ok_or(SessionError::NoPendingPermission)?;
        self.append_event(
            "permission.decided",
            Actor::User,
            RuntimeEventPayload::PermissionDecided {
                permission_id: permission_id.clone(),
                request_type: request_type.clone(),
                decision: decision.clone(),
            },
        )?;
        // Clear pending only after successful event append
        self.pending_permission = None;
        match decision {
            PermissionDecisionKind::Deny => self.transition_to(AgentState::Executing),
            PermissionDecisionKind::AllowOnce
            | PermissionDecisionKind::AllowSession
            | PermissionDecisionKind::AllowProjectRule => match request_type {
                PermissionRequestType::Command => self.transition_to(AgentState::RunningCommand),
                PermissionRequestType::FileWrite => self.transition_to(AgentState::ApplyingPatch),
                _ => self.transition_to(AgentState::Executing),
            },
            PermissionDecisionKind::Modify => self.transition_to(AgentState::WaitingForUser),
        }
    }

    pub fn clear_pending_runtime_decisions(&mut self, reason: &str) -> Result<(), SessionError> {
        if let Some(plan_approval_id) = self.pending_plan_approval.clone() {
            self.append_event(
                "plan.approval_cleared",
                Actor::Runtime,
                RuntimeEventPayload::Generic {
                    json: format!(
                        "{{\"plan_approval_id\":\"{}\",\"reason\":\"{}\"}}",
                        json_escape(&plan_approval_id),
                        json_escape(reason)
                    ),
                },
            )?;
            self.pending_plan_approval = None;
        }
        if let Some((permission_id, request_type)) = self.pending_permission.clone() {
            self.append_event(
                "permission.cleared",
                Actor::Runtime,
                RuntimeEventPayload::Generic {
                    json: format!(
                        "{{\"permission_id\":\"{}\",\"request_type\":\"{}\",\"reason\":\"{}\"}}",
                        json_escape(&permission_id),
                        json_escape(permission_request_type_to_wire(&request_type)),
                        json_escape(reason)
                    ),
                },
            )?;
            self.pending_permission = None;
        }
        Ok(())
    }

    pub fn record_tool_call_requested(
        &mut self,
        tool_call_id: impl Into<String>,
        tool_id: impl Into<String>,
    ) -> Result<(), SessionError> {
        let tool_call_id = tool_call_id.into();
        let tool_id = tool_id.into();
        if find_tool_spec(&tool_id).is_none() {
            return Err(SessionError::UnknownTool(tool_id));
        }
        self.ensure_execution_activity("tool.call_requested", true)?;
        self.append_event(
            "tool.call_requested",
            Actor::Agent,
            RuntimeEventPayload::ToolCallRequested {
                tool_call_id,
                tool_id,
                provider_tool_call_id: None,
            },
        )
    }

    pub fn record_tool_call_completed(
        &mut self,
        tool_call_id: impl Into<String>,
        tool_id: impl Into<String>,
        ok: bool,
    ) -> Result<(), SessionError> {
        let tool_call_id = tool_call_id.into();
        let tool_id = tool_id.into();
        if find_tool_spec(&tool_id).is_none() {
            return Err(SessionError::UnknownTool(tool_id));
        }
        self.append_event(
            "tool.call_completed",
            Actor::Tool,
            RuntimeEventPayload::ToolCallCompleted {
                tool_call_id,
                tool_id,
                ok,
                provider_tool_call_id: None,
            },
        )
    }

    pub fn record_tool_result_artifact(
        &mut self,
        tool_call_id: impl Into<String>,
        tool_id: impl Into<String>,
        artifact_id: impl Into<String>,
        content_hash: impl Into<String>,
        preview: impl Into<String>,
    ) -> Result<(), SessionError> {
        let tool_call_id = tool_call_id.into();
        let tool_id = tool_id.into();
        if find_tool_spec(&tool_id).is_none() {
            return Err(SessionError::UnknownTool(tool_id));
        }
        self.append_event(
            "tool.result_recorded",
            Actor::Runtime,
            RuntimeEventPayload::ToolResultRecorded {
                tool_call_id,
                tool_id,
                artifact_id: artifact_id.into(),
                content_hash: content_hash.into(),
                preview: preview.into(),
                provider_tool_call_id: None,
            },
        )
    }

    pub fn record_tool_call_assembled(
        &mut self,
        tool_call_id: impl Into<String>,
        tool_id: impl Into<String>,
        arguments_json: impl Into<String>,
        arguments_replayable: bool,
    ) -> Result<(), SessionError> {
        let tool_call_id = tool_call_id.into();
        let tool_id = tool_id.into();
        let arguments_json = arguments_json.into();
        if find_tool_spec(&tool_id).is_none() {
            return Err(SessionError::UnknownTool(tool_id));
        }
        self.record_assistant_tool_call_block(&tool_call_id, None, &tool_id, &arguments_json)?;
        self.append_event(
            "tool.call.assembled",
            Actor::Agent,
            RuntimeEventPayload::ToolCallAssembled {
                tool_call_id,
                tool_id,
                arguments_json,
                arguments_replayable,
                provider_tool_call_id: None,
            },
        )
    }

    pub fn record_tool_call_assembled_with_provider_id(
        &mut self,
        tool_call_id: impl Into<String>,
        provider_tool_call_id: Option<impl Into<String>>,
        tool_id: impl Into<String>,
        arguments_json: impl Into<String>,
        arguments_replayable: bool,
    ) -> Result<(), SessionError> {
        let tool_call_id = tool_call_id.into();
        let tool_id = tool_id.into();
        let provider_tool_call_id = provider_tool_call_id.map(|v| v.into());
        let arguments_json = arguments_json.into();
        if find_tool_spec(&tool_id).is_none() {
            return Err(SessionError::UnknownTool(tool_id));
        }
        self.record_assistant_tool_call_block(
            &tool_call_id,
            provider_tool_call_id.as_deref(),
            &tool_id,
            &arguments_json,
        )?;
        self.append_event(
            "tool.call.assembled",
            Actor::Agent,
            RuntimeEventPayload::ToolCallAssembled {
                tool_call_id,
                tool_id,
                arguments_json,
                arguments_replayable,
                provider_tool_call_id,
            },
        )
    }

    pub fn record_tool_call_requested_with_provider_id(
        &mut self,
        tool_call_id: impl Into<String>,
        provider_tool_call_id: Option<impl Into<String>>,
        tool_id: impl Into<String>,
    ) -> Result<(), SessionError> {
        let tool_call_id = tool_call_id.into();
        let tool_id = tool_id.into();
        if find_tool_spec(&tool_id).is_none() {
            return Err(SessionError::UnknownTool(tool_id));
        }
        self.ensure_execution_activity("tool.call_requested", true)?;
        self.append_event(
            "tool.call_requested",
            Actor::Agent,
            RuntimeEventPayload::ToolCallRequested {
                tool_call_id,
                tool_id,
                provider_tool_call_id: provider_tool_call_id.map(|v| v.into()),
            },
        )
    }

    pub fn record_tool_call_completed_with_provider_id(
        &mut self,
        tool_call_id: impl Into<String>,
        provider_tool_call_id: Option<impl Into<String>>,
        tool_id: impl Into<String>,
        ok: bool,
    ) -> Result<(), SessionError> {
        let tool_call_id = tool_call_id.into();
        let tool_id = tool_id.into();
        if find_tool_spec(&tool_id).is_none() {
            return Err(SessionError::UnknownTool(tool_id));
        }
        self.append_event(
            "tool.call_completed",
            Actor::Tool,
            RuntimeEventPayload::ToolCallCompleted {
                tool_call_id,
                tool_id,
                ok,
                provider_tool_call_id: provider_tool_call_id.map(|v| v.into()),
            },
        )
    }

    pub fn record_tool_result_artifact_with_provider_id(
        &mut self,
        tool_call_id: impl Into<String>,
        provider_tool_call_id: Option<impl Into<String>>,
        tool_id: impl Into<String>,
        artifact_id: impl Into<String>,
        content_hash: impl Into<String>,
        preview: impl Into<String>,
    ) -> Result<(), SessionError> {
        let tool_call_id = tool_call_id.into();
        let tool_id = tool_id.into();
        if find_tool_spec(&tool_id).is_none() {
            return Err(SessionError::UnknownTool(tool_id));
        }
        self.append_event(
            "tool.result_recorded",
            Actor::Runtime,
            RuntimeEventPayload::ToolResultRecorded {
                tool_call_id,
                tool_id,
                artifact_id: artifact_id.into(),
                content_hash: content_hash.into(),
                preview: preview.into(),
                provider_tool_call_id: provider_tool_call_id.map(|v| v.into()),
            },
        )
    }

    pub fn record_model_stream_delta(
        &mut self,
        stream_id: impl Into<String>,
        provider: impl Into<String>,
        delta_kind: impl Into<String>,
        preview: impl Into<String>,
    ) -> Result<(), SessionError> {
        let stream_id = stream_id.into();
        let provider = provider.into();
        let delta_kind = delta_kind.into();
        let preview = preview.into();
        self.ensure_execution_activity("model.stream_delta", false)?;
        if delta_kind == "content" {
            self.record_assistant_text_delta(&stream_id, &provider, &preview, true)?;
        }
        self.append_event(
            "model.stream_delta",
            Actor::Model,
            RuntimeEventPayload::ModelStreamDelta {
                stream_id,
                provider,
                delta_kind,
                preview,
                runtime_sanitized: true,
            },
        )
    }

    pub fn record_model_call_started(
        &mut self,
        call_id: impl Into<String>,
        provider: impl Into<String>,
        adapter_id: impl Into<String>,
        actual_model_name: impl Into<String>,
        role: impl Into<String>,
        live: bool,
    ) -> Result<(), SessionError> {
        let scaffold_level = infer_scaffold_level(&provider.into(), &role.into());
        self.record_model_call_started_with_metadata(
            call_id,
            scaffold_level.provider,
            adapter_id,
            actual_model_name,
            scaffold_level.role,
            live,
            scaffold_level.level,
            0,
            "unknown",
            "unknown",
            scaffold_level.max_context_tokens,
            scaffold_level.prompt_scaffold_budget,
            scaffold_level.dynamic_context_budget,
            scaffold_level.protected_reserve_tokens,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_model_call_started_with_metadata(
        &mut self,
        call_id: impl Into<String>,
        provider: impl Into<String>,
        adapter_id: impl Into<String>,
        actual_model_name: impl Into<String>,
        role: impl Into<String>,
        live: bool,
        scaffold_level: impl Into<String>,
        prompt_tokens_estimate: u64,
        prompt_hash: impl Into<String>,
        tool_catalog_hash: impl Into<String>,
        max_context_tokens: u64,
        prompt_scaffold_budget: u64,
        dynamic_context_budget: u64,
        protected_reserve_tokens: u64,
        budget_warning_count: u64,
    ) -> Result<(), SessionError> {
        self.ensure_execution_activity("model.call_started", false)?;
        self.append_event(
            "model.call_started",
            Actor::Runtime,
            RuntimeEventPayload::ModelCallStarted {
                call_id: call_id.into(),
                provider: provider.into(),
                adapter_id: adapter_id.into(),
                actual_model_name: actual_model_name.into(),
                role: role.into(),
                live,
                scaffold_level: scaffold_level.into(),
                prompt_tokens_estimate,
                prompt_hash: prompt_hash.into(),
                tool_catalog_hash: tool_catalog_hash.into(),
                max_context_tokens,
                prompt_scaffold_budget,
                dynamic_context_budget,
                protected_reserve_tokens,
                budget_warning_count,
            },
        )
    }

    pub fn record_model_call_completed(
        &mut self,
        call_id: impl Into<String>,
        provider: impl Into<String>,
        ok: bool,
        artifact_id: impl Into<String>,
        content_hash: impl Into<String>,
    ) -> Result<(), SessionError> {
        self.append_event(
            "model.call_completed",
            Actor::Runtime,
            RuntimeEventPayload::ModelCallCompleted {
                call_id: call_id.into(),
                provider: provider.into(),
                ok,
                artifact_id: artifact_id.into(),
                content_hash: content_hash.into(),
            },
        )
    }

    pub fn record_model_call_blocked(
        &mut self,
        call_id: impl Into<String>,
        provider: impl Into<String>,
        gate: impl Into<String>,
    ) -> Result<(), SessionError> {
        self.append_event(
            "model.call_blocked",
            Actor::Policy,
            RuntimeEventPayload::ModelCallBlocked {
                call_id: call_id.into(),
                provider: provider.into(),
                gate: gate.into(),
            },
        )
    }

    pub fn record_model_stream_completed(
        &mut self,
        stream_id: impl Into<String>,
        provider: impl Into<String>,
        artifact_id: impl Into<String>,
        content_hash: impl Into<String>,
        prompt_tokens: u64,
        completion_tokens: u64,
        reasoning_tokens: u64,
        prompt_cache_hit_tokens: u64,
        prompt_cache_miss_tokens: u64,
        stop_reason: Option<&str>,
    ) -> Result<(), SessionError> {
        let stream_id = stream_id.into();
        let provider = provider.into();
        self.close_assistant_text_block_if_open(&stream_id, &provider)?;
        self.append_event(
            "model.stream_completed",
            Actor::Runtime,
            RuntimeEventPayload::ModelStreamCompleted {
                stream_id,
                provider,
                artifact_id: artifact_id.into(),
                content_hash: content_hash.into(),
                prompt_tokens,
                completion_tokens,
                reasoning_tokens,
                prompt_cache_hit_tokens,
                prompt_cache_miss_tokens,
                stop_reason: stop_reason.map(|value| value.to_string()),
            },
        )
    }

    fn record_assistant_text_delta(
        &mut self,
        stream_id: &str,
        provider: &str,
        text: &str,
        runtime_sanitized: bool,
    ) -> Result<(), SessionError> {
        let block_id = assistant_text_block_id(stream_id);
        if self.active_assistant_text_blocks.insert(block_id.clone()) {
            self.append_event(
                "assistant.block_started",
                Actor::Agent,
                RuntimeEventPayload::Generic {
                    json: format!(
                        "{{\"stream_id\":{},\"block_id\":{},\"block_kind\":\"text\",\"provider\":{}}}",
                        json_string_local(stream_id),
                        json_string_local(&block_id),
                        json_string_local(provider)
                    ),
                },
            )?;
        }
        self.append_event(
            "assistant.text_delta",
            Actor::Agent,
            RuntimeEventPayload::Generic {
                json: format!(
                    "{{\"stream_id\":{},\"block_id\":{},\"text\":{},\"provider\":{},\"runtime_sanitized\":{}}}",
                    json_string_local(stream_id),
                    json_string_local(&block_id),
                    json_string_local(text),
                    json_string_local(provider),
                    runtime_sanitized
                ),
            },
        )
    }

    fn close_assistant_text_block_if_open(
        &mut self,
        stream_id: &str,
        provider: &str,
    ) -> Result<(), SessionError> {
        let block_id = assistant_text_block_id(stream_id);
        if !self.active_assistant_text_blocks.remove(&block_id) {
            return Ok(());
        }
        self.append_event(
            "assistant.block_completed",
            Actor::Agent,
            RuntimeEventPayload::Generic {
                json: format!(
                    "{{\"stream_id\":{},\"block_id\":{},\"block_kind\":\"text\",\"provider\":{}}}",
                    json_string_local(stream_id),
                    json_string_local(&block_id),
                    json_string_local(provider)
                ),
            },
        )
    }

    fn record_assistant_tool_call_block(
        &mut self,
        tool_call_id: &str,
        provider_tool_call_id: Option<&str>,
        tool_id: &str,
        arguments_json: &str,
    ) -> Result<(), SessionError> {
        let block_id = format!("tool_call:{tool_call_id}");
        let provider_part = provider_tool_call_id
            .map(|id| format!(",\"provider_tool_call_id\":{}", json_string_local(id)))
            .unwrap_or_default();
        self.append_event(
            "assistant.block_started",
            Actor::Agent,
            RuntimeEventPayload::Generic {
                json: format!(
                    "{{\"block_id\":{},\"block_kind\":\"tool_call\",\"tool_call_id\":{},\"tool_id\":{}{}}}",
                    json_string_local(&block_id),
                    json_string_local(tool_call_id),
                    json_string_local(tool_id),
                    provider_part.as_str()
                ),
            },
        )?;
        self.append_event(
            "assistant.tool_call_delta",
            Actor::Agent,
            RuntimeEventPayload::Generic {
                json: format!(
                    "{{\"block_id\":{},\"tool_call_id\":{},\"tool_id\":{},\"arguments_json\":{}{}}}",
                    json_string_local(&block_id),
                    json_string_local(tool_call_id),
                    json_string_local(tool_id),
                    json_string_local(arguments_json),
                    provider_part.as_str()
                ),
            },
        )?;
        self.append_event(
            "assistant.block_completed",
            Actor::Agent,
            RuntimeEventPayload::Generic {
                json: format!(
                    "{{\"block_id\":{},\"block_kind\":\"tool_call\",\"tool_call_id\":{},\"tool_id\":{}{}}}",
                    json_string_local(&block_id),
                    json_string_local(tool_call_id),
                    json_string_local(tool_id),
                    provider_part.as_str()
                ),
            },
        )
    }

    pub fn record_patch_proposal_created(
        &mut self,
        patch_id: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<(), SessionError> {
        let patch_id = patch_id.into();
        let path = path.into();
        self.ensure_execution_activity("patch.proposal_created", false)?;
        self.append_event(
            "patch.proposal_created",
            Actor::Agent,
            RuntimeEventPayload::PatchProposalCreated { patch_id, path },
        )
    }

    pub fn record_patch_proposal_validated(
        &mut self,
        patch_id: impl Into<String>,
        validation: PatchValidation,
    ) -> Result<(), SessionError> {
        let patch_id = patch_id.into();
        self.append_event(
            "patch.proposal_validated",
            Actor::Runtime,
            RuntimeEventPayload::PatchProposalValidated {
                patch_id,
                validation: patch_validation_to_str(&validation).to_string(),
            },
        )
    }

    pub fn record_patch_applied(
        &mut self,
        patch_id: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<(), SessionError> {
        let patch_id = patch_id.into();
        let path = path.into();
        if self.state != AgentState::ApplyingPatch {
            self.transition_to(AgentState::ApplyingPatch)?;
        }
        self.append_event(
            "patch.applied",
            Actor::Runtime,
            RuntimeEventPayload::PatchApplied { patch_id, path },
        )
    }

    pub fn record_runtime_event(
        &mut self,
        event_type: impl Into<String>,
        actor: Actor,
        payload_json: impl Into<String>,
    ) -> Result<(), SessionError> {
        let event_type = event_type.into();
        self.append_event(
            &event_type,
            actor,
            RuntimeEventPayload::Generic {
                json: payload_json.into(),
            },
        )
    }

    pub fn start_review(&mut self) -> Result<(), SessionError> {
        self.transition_to(AgentState::Reviewing)
    }

    pub fn complete_after_review(&mut self) -> Result<(), SessionError> {
        self.transition_to(AgentState::Completed)
    }

    pub fn request_rework_from_review(&mut self) -> Result<(), SessionError> {
        self.transition_to(AgentState::Executing)
    }

    pub fn diagnose_failure(&mut self) -> Result<(), SessionError> {
        self.transition_to(AgentState::DiagnosingFailure)
    }

    pub fn resume_after_diagnosis(&mut self) -> Result<(), SessionError> {
        self.transition_to(AgentState::Executing)
    }

    /// Merge events from another event log (e.g., from a completed agent loop)
    /// into this session's event log. Re-sequences events to continue from
    /// the last sequence number in the existing log. Skips `session.created`
    /// events (already present) and rewrites duplicate call IDs in payload_json
    /// to avoid collision with prior loop invocations.
    pub fn merge_events(&mut self, events: &[KernelEvent]) {
        let suffix_seq = self.event_log.last().map(|e| e.sequence + 1).unwrap_or(1);
        self.merge_events_with_id_suffix(events, suffix_seq);
    }

    /// Merge a batch of events from another event log while using one stable
    /// ID suffix for the entire batch. This keeps related `call_id`,
    /// `stream_id`, and `tool_call_id` values correlated after merge.
    pub fn merge_events_with_id_suffix(&mut self, events: &[KernelEvent], suffix_seq: u64) {
        let mut next_seq = self.event_log.last().map(|e| e.sequence + 1).unwrap_or(1);
        for event in events {
            if event.event_type == "session.created" {
                continue;
            }
            let mut renumbered = event.clone();
            renumbered.sequence = next_seq;
            renumbered.event_id = format!("evt_{next_seq:04}");
            renumbered.hash = format!("{}:{}:{}", self.session_id, next_seq, renumbered.event_type);
            renumbered.prev_hash = self.event_log.last().map(|e| e.hash.clone());
            // Rewrite ID fields at all nesting levels to avoid duplicates across
            // loop invocations. Recursive: covers top-level and nested objects
            // (tool_use, tool_result, permission blocks, provider metadata, etc.).
            if let Some(ref mut map) = serde_json::from_str::<
                serde_json::Map<String, serde_json::Value>,
            >(&renumbered.payload_json)
            .ok()
            {
                let mut updated = serde_json::Value::Object(map.clone());
                rewrite_ids_recursive(&mut updated, suffix_seq);
                if let serde_json::Value::Object(updated_map) = updated {
                    if let Ok(new_payload) = serde_json::to_string(&updated_map) {
                        renumbered.payload_json = new_payload;
                    }
                }
            }
            if self.event_log.append(renumbered).is_ok() {
                if let Some(merged) = self.event_log.last().cloned() {
                    self.apply_merged_event_side_effect(&merged);
                }
                next_seq += 1;
            }
        }
    }

    fn apply_merged_event_side_effect(&mut self, event: &KernelEvent) {
        match event.event_type.as_str() {
            "session.turn_started" => {
                self.current_turn_id = extract_json_string(&event.payload_json, "turn_id");
            }
            "session.state_changed" => {
                if let Some(next) = extract_json_string(&event.payload_json, "to_state")
                    .and_then(|value| agent_state_from_str(&value))
                {
                    self.state = next;
                }
            }
            "plan.approval_requested" => {
                self.pending_plan_approval =
                    extract_json_string(&event.payload_json, "plan_approval_id");
            }
            "plan.approval_decided" => {
                if let Some(plan_approval_id) =
                    extract_json_string(&event.payload_json, "plan_approval_id")
                {
                    if self.pending_plan_approval.as_deref() == Some(plan_approval_id.as_str()) {
                        self.pending_plan_approval = None;
                    }
                }
            }
            "plan.approval_cleared" => {
                if let Some(plan_approval_id) =
                    extract_json_string(&event.payload_json, "plan_approval_id")
                {
                    if self.pending_plan_approval.as_deref() == Some(plan_approval_id.as_str()) {
                        self.pending_plan_approval = None;
                    }
                }
            }
            "permission.requested" => {
                if let (Some(permission_id), Some(request_type)) = (
                    extract_json_string(&event.payload_json, "permission_id"),
                    extract_json_string(&event.payload_json, "request_type")
                        .and_then(|value| PermissionRequestType::parse(&value)),
                ) {
                    self.pending_permission = Some((permission_id, request_type));
                }
            }
            "permission.decided" => {
                if let Some(permission_id) =
                    extract_json_string(&event.payload_json, "permission_id")
                {
                    if self
                        .pending_permission
                        .as_ref()
                        .is_some_and(|(pending_id, _)| pending_id == &permission_id)
                    {
                        self.pending_permission = None;
                    }
                }
            }
            "permission.cleared" => {
                if let Some(permission_id) =
                    extract_json_string(&event.payload_json, "permission_id")
                {
                    if self
                        .pending_permission
                        .as_ref()
                        .is_some_and(|(pending_id, _)| pending_id == &permission_id)
                    {
                        self.pending_permission = None;
                    }
                }
            }
            _ => {}
        }
    }

    fn append_event(
        &mut self,
        event_type: &str,
        actor: Actor,
        payload: RuntimeEventPayload,
    ) -> Result<(), SessionError> {
        let sequence = self
            .event_log
            .last()
            .map(|event| event.sequence + 1)
            .unwrap_or(1);
        let prev_hash = self.event_log.last().map(|event| event.hash.clone());
        // Hash commits to payload content for tamper evidence, not just identity fields
        let payload_json = payload.to_json();
        let hash_source = format!(
            "{}:{}:{}:{}:{}",
            self.session_id,
            sequence,
            event_type,
            prev_hash.as_deref().unwrap_or("genesis"),
            simple_hash(&payload_json)
        );
        let hash = simple_hash(&hash_source);
        self.event_log.append(KernelEvent {
            event_id: format!("evt_{sequence:04}"),
            schema_version: "v0".to_string(),
            project_id: self.project_id.clone(),
            session_id: Some(self.session_id.clone()),
            task_id: Some(self.task_id.clone()),
            sequence,
            event_type: event_type.to_string(),
            actor,
            created_at: chrono_now(),
            payload_json,
            prev_hash,
            hash,
        })?;
        Ok(())
    }
}

/// ID fields that may collide across loop invocations when merging events.
const REWRITABLE_ID_KEYS: &[&str] = &[
    "id",
    "call_id",
    "stream_id",
    "tool_call_id",
    "provider_tool_use_id",
];

/// Recursively walk the JSON value, appending `_loop_{seq}` to any
/// string-valued field whose key is in `REWRITABLE_ID_KEYS`.
fn rewrite_ids_recursive(value: &mut serde_json::Value, seq: u64) {
    match value {
        serde_json::Value::Object(map) => {
            let mut key_rewrites: Vec<(String, String)> = Vec::new();
            for (key, val) in map.iter() {
                if REWRITABLE_ID_KEYS.contains(&key.as_str()) {
                    if let serde_json::Value::String(s) = val {
                        key_rewrites.push((key.clone(), format!("{s}_loop_{seq}")));
                    }
                }
            }
            for (key, new_val) in key_rewrites {
                map.insert(key, serde_json::Value::String(new_val));
            }
            // Recurse into all values
            for val in map.values_mut() {
                rewrite_ids_recursive(val, seq);
            }
        }
        serde_json::Value::Array(arr) => {
            for val in arr.iter_mut() {
                rewrite_ids_recursive(val, seq);
            }
        }
        _ => {}
    }
}

fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let start = json.find(&marker)? + marker.len();
    let rest = &json[start..];
    let end = rest.find('"')?;
    Some(
        rest[..end]
            .replace("\\\\", "\x00") // temp placeholder
            .replace("\\\"", "\"")
            .replace("\x00", "\\"), // restore real backslashes
    )
}

/// Non-cryptographic hash for event chain integrity.
/// Uses FNV-1a 64-bit for speed; sufficient for tamper detection.
fn simple_hash(input: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv64_{hash:016x}")
}

/// Current ISO 8601 timestamp. Returns "now" if clock is broken.
fn chrono_now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| {
            let secs = d.as_secs();
            // ISO 8601 basic format
            let nanos = d.subsec_nanos();
            format!("{secs}.{nanos:09}")
        })
        .unwrap_or_else(|_| "now".to_string())
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            other => vec![other],
        })
        .collect()
}

fn permission_request_type_to_wire(value: &PermissionRequestType) -> &'static str {
    match value {
        PermissionRequestType::Command => "command",
        PermissionRequestType::FileWrite => "file_write",
        PermissionRequestType::Network => "network",
        PermissionRequestType::PackageInstall => "package_install",
        PermissionRequestType::CloudModel => "cloud_model",
        PermissionRequestType::ProtectedPath => "protected_path",
        PermissionRequestType::ArtifactExport => "artifact_export",
    }
}

struct InferredModelStart {
    provider: String,
    role: String,
    level: String,
    max_context_tokens: u64,
    prompt_scaffold_budget: u64,
    dynamic_context_budget: u64,
    protected_reserve_tokens: u64,
}

fn infer_scaffold_level(provider: &str, role: &str) -> InferredModelStart {
    let role_enum = model_role_from_str(role).unwrap_or(ModelRole::Executor);
    let budget = match provider {
        "deepseek" => Some(allocate_native_context_budget(
            NativeModelFamily::DeepSeek,
            role_enum,
            None,
        )),
        "qwen" => Some(allocate_native_context_budget(
            NativeModelFamily::Qwen,
            role_enum,
            None,
        )),
        _ => None,
    };
    let level = budget
        .as_ref()
        .map(|budget| format!("{:?}", budget.scaffold_level))
        .unwrap_or_else(|| "unknown".to_string());
    InferredModelStart {
        provider: provider.to_string(),
        role: role.to_string(),
        level,
        max_context_tokens: budget
            .as_ref()
            .map_or(0, |budget| budget.max_context_tokens),
        prompt_scaffold_budget: budget
            .as_ref()
            .map_or(0, |budget| budget.prompt_scaffold_tokens()),
        dynamic_context_budget: budget
            .as_ref()
            .map_or(0, |budget| budget.dynamic_context_tokens()),
        protected_reserve_tokens: budget
            .as_ref()
            .map_or(0, |budget| budget.protected_reserve_tokens()),
    }
}

fn model_role_from_str(value: &str) -> Option<ModelRole> {
    match value {
        "planner" => Some(ModelRole::Planner),
        "executor" => Some(ModelRole::Executor),
        "reviewer" => Some(ModelRole::Reviewer),
        "researcher" => Some(ModelRole::Researcher),
        "summarizer" => Some(ModelRole::Summarizer),
        _ => None,
    }
}

fn agent_state_from_str(value: &str) -> Option<AgentState> {
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

fn patch_validation_to_str(validation: &PatchValidation) -> &'static str {
    match validation {
        PatchValidation::Pass => "pass",
        PatchValidation::PassCreate => "pass_create",
        PatchValidation::FailProtected => "fail_protected",
        PatchValidation::FailCreateExists => "fail_create_exists",
        PatchValidation::FailMissing => "fail_missing",
        PatchValidation::FailStale => "fail_stale",
        PatchValidation::FailMissingOldString => "fail_missing_old_string",
        PatchValidation::FailAmbiguous => "fail_ambiguous",
        PatchValidation::FailMissingBaseHash => "fail_missing_base_hash",
    }
}

fn assistant_text_block_id(stream_id: &str) -> String {
    format!("text:{stream_id}")
}

fn json_string_local(value: &str) -> String {
    format!("\"{}\"", escape_json_local(value))
}

fn escape_json_local(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_approval_is_governance_path() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .request_plan_approval("pa_1", Some("test plan goal".to_string()))
            .unwrap();
        session
            .decide_plan(PlanApprovalDecisionKind::Approve)
            .unwrap();
        assert_eq!(session.state(), AgentState::RetrievingContext);
        assert_eq!(session.event_count(), 6);
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"plan.approval_requested\""));
        assert!(jsonl.contains("\"plan_approval_id\":\"pa_1\""));
        assert!(!jsonl.contains("\"request_type\":\"plan\""));
    }

    #[test]
    fn permission_file_write_routes_to_patch_state() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session
            .request_permission("perm_1", PermissionRequestType::FileWrite, None)
            .unwrap();
        session
            .decide_permission(PermissionDecisionKind::AllowOnce)
            .unwrap();
        assert_eq!(session.state(), AgentState::ApplyingPatch);
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"permission_id\":\"perm_1\""));
        assert!(jsonl.contains("\"request_type\":\"file_write\""));
        assert!(jsonl.contains("\"decision\":\"allow_once\""));
    }

    #[test]
    fn permission_command_routes_to_running_command() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session
            .request_permission("perm_1", PermissionRequestType::Command, None)
            .unwrap();
        session
            .decide_permission(PermissionDecisionKind::AllowOnce)
            .unwrap();
        assert_eq!(session.state(), AgentState::RunningCommand);
    }

    #[test]
    fn cannot_decide_permission_without_request() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        assert_eq!(
            session.decide_permission(PermissionDecisionKind::AllowOnce),
            Err(SessionError::NoPendingPermission)
        );
    }

    #[test]
    fn resumes_pending_permission_from_event_log() {
        let mut session = AgentSession::new("proj", "sess_resume", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session
            .request_permission("perm_resume", PermissionRequestType::FileWrite, None)
            .unwrap();
        let log = EventLog::import_jsonl(&session.export_events_jsonl()).unwrap();
        let mut resumed = AgentSession::resume_from_event_log(log).unwrap();
        assert_eq!(resumed.state(), AgentState::WaitingForToolApproval);
        resumed
            .decide_permission(PermissionDecisionKind::AllowOnce)
            .unwrap();
        assert_eq!(resumed.state(), AgentState::ApplyingPatch);
        assert!(resumed
            .export_events_jsonl()
            .contains("\"permission_id\":\"perm_resume\""));
    }

    #[test]
    fn cleared_pending_decisions_are_replayed_from_event_log() {
        let mut session = AgentSession::new("proj", "sess_cleared", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .request_plan_approval("plan_clear", Some("clear plan".to_string()))
            .unwrap();
        session
            .clear_pending_runtime_decisions("test_clear_plan")
            .unwrap();
        assert_eq!(session.pending_plan_approval_id(), None);
        session
            .transition_to(AgentState::Executing)
            .unwrap_or_else(|_| session.set_state(AgentState::Executing).unwrap());
        session
            .request_permission("perm_clear", PermissionRequestType::Command, None)
            .unwrap();
        session
            .clear_pending_runtime_decisions("test_clear_permission")
            .unwrap();
        assert_eq!(session.pending_permission_id(), None);

        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("plan.approval_cleared"));
        assert!(jsonl.contains("permission.cleared"));
        let log = EventLog::import_jsonl(&jsonl).unwrap();
        let resumed = AgentSession::resume_from_event_log(log).unwrap();
        assert_eq!(resumed.pending_plan_approval_id(), None);
        assert_eq!(resumed.pending_permission_id(), None);
    }

    #[test]
    fn merge_events_preserves_pending_permission_identity_and_state() {
        let mut source = AgentSession::new("proj", "source_sess", "task").unwrap();
        source.transition_to(AgentState::Planning).unwrap();
        source.transition_to(AgentState::RetrievingContext).unwrap();
        source.transition_to(AgentState::Executing).unwrap();
        source
            .request_permission(
                "native_loop_v2_write_perm_120",
                PermissionRequestType::FileWrite,
                Some("file.write".to_string()),
            )
            .unwrap();
        let source_events = EventLog::import_jsonl(&source.export_events_jsonl()).unwrap();

        let mut target = AgentSession::new("proj", "target_sess", "task").unwrap();
        target.merge_events_with_id_suffix(&source_events.iter().cloned().collect::<Vec<_>>(), 99);

        assert_eq!(target.state(), AgentState::WaitingForToolApproval);
        assert_eq!(
            target.pending_permission_id(),
            Some("native_loop_v2_write_perm_120")
        );
        let merged_jsonl = target.export_events_jsonl();
        assert!(merged_jsonl.contains("\"permission_id\":\"native_loop_v2_write_perm_120\""));
        assert!(!merged_jsonl.contains("native_loop_v2_write_perm_120_loop_99"));
    }

    #[test]
    fn records_tool_call_lifecycle_with_tool_spec_validation() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session
            .record_tool_call_requested("tool_call_1", "file.read")
            .unwrap();
        session
            .record_tool_call_completed("tool_call_1", "file.read", true)
            .unwrap();
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"tool.call_requested\""));
        assert!(jsonl.contains("\"tool_id\":\"file.read\""));
        assert!(jsonl.contains("\"ok\":true"));
    }

    #[test]
    fn records_tool_result_artifact_link() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session
            .record_tool_call_requested("tool_call_1", "file.read")
            .unwrap();
        session
            .record_tool_call_completed("tool_call_1", "file.read", true)
            .unwrap();
        session
            .record_tool_result_artifact(
                "tool_call_1",
                "file.read",
                "artifact_1",
                "fnv64_hash",
                "preview",
            )
            .unwrap();
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"tool.result_recorded\""));
        assert!(jsonl.contains("\"artifact_id\":\"artifact_1\""));
        assert!(jsonl.contains("\"content_hash\":\"fnv64_hash\""));
        assert_eq!(
            session.record_tool_call_requested("tool_call_2", "unknown.tool"),
            Err(SessionError::UnknownTool("unknown.tool".to_string()))
        );
    }

    #[test]
    fn records_model_stream_without_raw_reasoning_replay() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session
            .record_model_stream_delta(
                "stream_1",
                "deepseek",
                "reasoning_sanitized",
                "Need [REDACTED_SECRET] from [REDACTED_PATH]",
            )
            .unwrap();
        session
            .record_model_stream_delta("stream_1", "deepseek", "content", "Visible answer")
            .unwrap();
        session
            .record_model_stream_completed(
                "stream_1",
                "deepseek",
                "artifact_stream_1",
                "fnv64_hash",
                100,
                20,
                15,
                80,
                20,
                Some("stop"),
            )
            .unwrap();
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"model.stream_delta\""));
        assert!(jsonl.contains("\"delta_kind\":\"reasoning_sanitized\""));
        assert!(jsonl.contains("\"runtime_sanitized\":true"));
        assert!(jsonl.contains("\"event_type\":\"model.stream_completed\""));
        assert!(jsonl.contains("\"prompt_cache_hit_tokens\":80"));
        assert!(!jsonl.contains("sk-testsecret"));
        assert!(!jsonl.contains(".env"));
    }

    #[test]
    fn records_assistant_text_block_around_visible_stream_content() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session
            .record_model_stream_delta("stream_1", "deepseek", "content", "I will read X first.")
            .unwrap();
        session
            .record_model_stream_completed(
                "stream_1",
                "deepseek",
                "artifact_stream_1",
                "fnv64_hash",
                100,
                20,
                0,
                80,
                20,
                Some("stop"),
            )
            .unwrap();

        let jsonl = session.export_events_jsonl();
        let block_started = jsonl
            .find("\"event_type\":\"assistant.block_started\"")
            .unwrap();
        let text_delta = jsonl
            .find("\"event_type\":\"assistant.text_delta\"")
            .unwrap();
        let block_completed = jsonl
            .find("\"event_type\":\"assistant.block_completed\"")
            .unwrap();
        let model_completed = jsonl
            .find("\"event_type\":\"model.stream_completed\"")
            .unwrap();

        assert!(block_started < text_delta);
        assert!(text_delta < block_completed);
        assert!(block_completed < model_completed);
        assert!(jsonl.contains("\"block_kind\":\"text\""));
        assert!(jsonl.contains("\"stream_id\":\"stream_1\""));
        assert!(jsonl.contains("\"text\":\"I will read X first.\""));
    }

    #[test]
    fn records_assistant_tool_call_block_before_assembled_tool_call() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session
            .record_tool_call_assembled("call_1", "file.read", r#"{"path":"README.md"}"#, true)
            .unwrap();

        let jsonl = session.export_events_jsonl();
        let block_started = jsonl
            .find("\"event_type\":\"assistant.block_started\"")
            .unwrap();
        let tool_delta = jsonl
            .find("\"event_type\":\"assistant.tool_call_delta\"")
            .unwrap();
        let block_completed = jsonl
            .find("\"event_type\":\"assistant.block_completed\"")
            .unwrap();
        let assembled = jsonl
            .find("\"event_type\":\"tool.call.assembled\"")
            .unwrap();

        assert!(block_started < tool_delta);
        assert!(tool_delta < block_completed);
        assert!(block_completed < assembled);
        assert!(jsonl.contains("\"block_kind\":\"tool_call\""));
        assert!(jsonl.contains("\"tool_call_id\":\"call_1\""));
        assert!(jsonl.contains("\"tool_id\":\"file.read\""));
        assert!(jsonl.contains("\"arguments_json\":\"{\\\"path\\\":\\\"README.md\\\"}\""));
    }

    #[test]
    fn records_model_call_boundary_without_key_material() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session
            .record_model_call_started(
                "call_1",
                "deepseek",
                "deepseek-v4-native",
                "deepseek-v4-flash",
                "planner",
                false,
            )
            .unwrap();
        session
            .record_model_call_completed(
                "call_1",
                "deepseek",
                true,
                "artifact_model_call_1",
                "fnv64_hash",
            )
            .unwrap();
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"model.call_started\""));
        assert!(jsonl.contains("\"live\":false"));
        assert!(jsonl.contains("\"event_type\":\"model.call_completed\""));
        assert!(!jsonl.contains("api_key"));
        assert!(!jsonl.contains("sk-"));
    }

    #[test]
    fn records_model_call_blocked_reason() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session
            .record_model_call_started(
                "call_1",
                "deepseek",
                "deepseek-v4-native",
                "deepseek-v4-flash",
                "planner",
                true,
            )
            .unwrap();
        session
            .record_model_call_blocked("call_1", "deepseek", "disabled_by_default")
            .unwrap();
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"model.call_blocked\""));
        assert!(jsonl.contains("\"gate\":\"disabled_by_default\""));
        assert!(!jsonl.contains("sk-"));
    }

    #[test]
    fn review_can_complete_or_return_to_execution() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session.start_review().unwrap();
        session.request_rework_from_review().unwrap();
        assert_eq!(session.state(), AgentState::Executing);
        session.start_review().unwrap();
        session.complete_after_review().unwrap();
        assert_eq!(session.state(), AgentState::Completed);
    }

    #[test]
    fn completed_session_can_start_new_interactive_turn() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session.start_review().unwrap();
        session.complete_after_review().unwrap();
        assert_eq!(session.state(), AgentState::Completed);

        session
            .record_tool_call_requested("tool_read_after_done", "file.read")
            .unwrap();
        assert_eq!(session.state(), AgentState::Executing);
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"session.turn_started\""));
        assert!(jsonl.contains("\"from_state\":\"Completed\""));
        assert!(jsonl.contains("\"to_state\":\"Executing\""));
        assert!(jsonl.contains("tool_read_after_done"));
    }

    #[test]
    fn waiting_for_user_session_can_start_new_interactive_turn() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session.transition_to(AgentState::WaitingForUser).unwrap();
        assert_eq!(session.state(), AgentState::WaitingForUser);

        session
            .begin_interactive_turn("turn_after_waiting_user", "followup")
            .unwrap();
        assert_eq!(session.state(), AgentState::Executing);
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"session.turn_started\""));
        assert!(jsonl.contains("\"from_state\":\"WaitingForUser\""));
        assert!(jsonl.contains("\"to_state\":\"Executing\""));
    }

    #[test]
    fn completed_session_can_request_permission_without_invalid_transition() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session.start_review().unwrap();
        session.complete_after_review().unwrap();
        assert_eq!(session.state(), AgentState::Completed);

        session
            .request_permission("perm_after_done", PermissionRequestType::Command, None)
            .unwrap();
        assert_eq!(session.state(), AgentState::WaitingForToolApproval);
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"session.turn_started\""));
        assert!(jsonl.contains("\"permission_id\":\"perm_after_done\""));
    }

    #[test]
    fn completed_session_can_request_plan_approval_without_invalid_transition() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session.start_review().unwrap();
        session.complete_after_review().unwrap();
        assert_eq!(session.state(), AgentState::Completed);

        session
            .request_plan_approval("plan_after_done", None)
            .unwrap();
        assert_eq!(session.state(), AgentState::WaitingForPlanApproval);
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"session.turn_started\""));
        assert!(jsonl.contains("\"plan_approval_id\":\"plan_after_done\""));
    }

    #[test]
    fn failure_diagnosis_can_resume_execution() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session.transition_to(AgentState::RunningCommand).unwrap();
        session.diagnose_failure().unwrap();
        session.resume_after_diagnosis().unwrap();
        assert_eq!(session.state(), AgentState::Executing);
    }

    #[test]
    fn records_patch_proposal_lifecycle() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session
            .record_patch_proposal_created("patch_1", "src/lib.rs")
            .unwrap();
        session
            .record_patch_proposal_validated("patch_1", PatchValidation::Pass)
            .unwrap();
        session
            .request_permission("perm_patch", PermissionRequestType::FileWrite, None)
            .unwrap();
        session
            .decide_permission(PermissionDecisionKind::AllowOnce)
            .unwrap();
        session
            .record_patch_applied("patch_1", "src/lib.rs")
            .unwrap();
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"patch.proposal_created\""));
        assert!(jsonl.contains("\"validation\":\"pass\""));
        assert!(jsonl.contains("\"event_type\":\"patch.applied\""));
    }
}
