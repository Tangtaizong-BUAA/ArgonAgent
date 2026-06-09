//! Typed event payload helpers.

use crate::state::AgentState;
use researchcode_kernel::{
    PermissionDecisionKind, PermissionRequestType, PlanApprovalDecisionKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEventPayload {
    Empty,
    Generic {
        json: String,
    },
    StateChanged {
        from: AgentState,
        to: AgentState,
    },
    PlanApprovalRequested {
        plan_approval_id: String,
        goal: Option<String>,
    },
    PlanApprovalDecided {
        plan_approval_id: String,
        decision: PlanApprovalDecisionKind,
    },
    PermissionRequested {
        permission_id: String,
        request_type: PermissionRequestType,
        tool_id: Option<String>,
    },
    PermissionDecided {
        permission_id: String,
        request_type: PermissionRequestType,
        decision: PermissionDecisionKind,
    },
    ToolCallAssembled {
        tool_call_id: String,
        tool_id: String,
        arguments_json: String,
        arguments_replayable: bool,
        provider_tool_call_id: Option<String>,
    },
    ToolCallRequested {
        tool_call_id: String,
        tool_id: String,
        provider_tool_call_id: Option<String>,
    },
    ToolCallCompleted {
        tool_call_id: String,
        tool_id: String,
        ok: bool,
        provider_tool_call_id: Option<String>,
    },
    ToolResultRecorded {
        tool_call_id: String,
        tool_id: String,
        artifact_id: String,
        content_hash: String,
        preview: String,
        provider_tool_call_id: Option<String>,
    },
    ModelCallStarted {
        call_id: String,
        provider: String,
        adapter_id: String,
        actual_model_name: String,
        role: String,
        live: bool,
        scaffold_level: String,
        prompt_tokens_estimate: u64,
        prompt_hash: String,
        tool_catalog_hash: String,
        max_context_tokens: u64,
        prompt_scaffold_budget: u64,
        dynamic_context_budget: u64,
        protected_reserve_tokens: u64,
        budget_warning_count: u64,
    },
    ModelCallCompleted {
        call_id: String,
        provider: String,
        ok: bool,
        artifact_id: String,
        content_hash: String,
    },
    ModelCallBlocked {
        call_id: String,
        provider: String,
        gate: String,
    },
    ModelStreamDelta {
        stream_id: String,
        provider: String,
        delta_kind: String,
        preview: String,
        runtime_sanitized: bool,
    },
    ModelStreamCompleted {
        stream_id: String,
        provider: String,
        artifact_id: String,
        content_hash: String,
        prompt_tokens: u64,
        completion_tokens: u64,
        reasoning_tokens: u64,
        prompt_cache_hit_tokens: u64,
        prompt_cache_miss_tokens: u64,
        stop_reason: Option<String>,
    },
    PatchProposalCreated {
        patch_id: String,
        path: String,
    },
    PatchProposalValidated {
        patch_id: String,
        validation: String,
    },
    PatchApplied {
        patch_id: String,
        path: String,
    },
}

impl RuntimeEventPayload {
    pub fn to_json(&self) -> String {
        match self {
            Self::Empty => "{}".to_string(),
            Self::Generic { json } => json.clone(),
            Self::StateChanged { from, to } => {
                format!(
                    "{{\"from_state\":\"{}\",\"to_state\":\"{}\"}}",
                    state_to_str(*from),
                    state_to_str(*to)
                )
            }
            Self::PlanApprovalRequested {
                plan_approval_id,
                goal,
            } => {
                let goal_part = goal
                    .as_ref()
                    .map(|g| format!("\"goal\":\"{}\",", escape(g)))
                    .unwrap_or_default();
                format!(
                    "{{{}\"plan_approval_id\":\"{}\"}}",
                    goal_part,
                    escape(plan_approval_id)
                )
            }
            Self::PlanApprovalDecided {
                plan_approval_id,
                decision,
            } => format!(
                "{{\"plan_approval_id\":\"{}\",\"decision\":\"{}\"}}",
                escape(plan_approval_id),
                plan_decision_to_str(decision)
            ),
            Self::PermissionRequested {
                permission_id,
                request_type,
                tool_id,
            } => {
                let tool_part = tool_id
                    .as_ref()
                    .map(|tid| format!("\"tool_id\":\"{}\",", escape(tid)))
                    .unwrap_or_default();
                format!(
                    "{{{}\"permission_id\":\"{}\",\"request_type\":\"{}\"}}",
                    tool_part,
                    escape(permission_id),
                    permission_request_type_to_str(request_type)
                )
            }
            Self::PermissionDecided {
                permission_id,
                request_type,
                decision,
            } => format!(
                "{{\"permission_id\":\"{}\",\"request_type\":\"{}\",\"decision\":\"{}\"}}",
                escape(permission_id),
                permission_request_type_to_str(request_type),
                permission_decision_to_str(decision)
            ),
            Self::ToolCallAssembled {
                tool_call_id,
                tool_id,
                arguments_json,
                arguments_replayable,
                provider_tool_call_id,
            } => {
                // Validate arguments_json is legal JSON before interpolating it
                // into the outer JSON object. Fall back to "{}" on parse failure.
                let safe_arguments = if serde_json::from_str::<serde_json::Value>(arguments_json).is_ok() {
                    arguments_json.as_str()
                } else {
                    "{}"
                };
                let provider = provider_tool_call_id
                    .as_deref()
                    .map(|v| format!("\"provider_tool_call_id\":\"{}\",", escape(v)))
                    .unwrap_or_default();
                format!(
                    "{{{}\"tool_call_id\":\"{}\",\"tool_id\":\"{}\",\"arguments\":{},\"arguments_replayable\":{}}}",
                    provider,
                    escape(tool_call_id),
                    escape(tool_id),
                    safe_arguments,
                    arguments_replayable
                )
            }
            Self::ToolCallRequested {
                tool_call_id,
                tool_id,
                provider_tool_call_id,
            } => {
                let provider = provider_tool_call_id
                    .as_deref()
                    .map(|v| format!("\"provider_tool_call_id\":\"{}\",", escape(v)))
                    .unwrap_or_default();
                format!(
                    "{{{}\"tool_call_id\":\"{}\",\"tool_id\":\"{}\"}}",
                    provider,
                    escape(tool_call_id),
                    escape(tool_id)
                )
            }
            Self::ToolCallCompleted {
                tool_call_id,
                tool_id,
                ok,
                provider_tool_call_id,
            } => {
                let provider = provider_tool_call_id
                    .as_deref()
                    .map(|v| format!("\"provider_tool_call_id\":\"{}\",", escape(v)))
                    .unwrap_or_default();
                format!(
                    "{{{}\"tool_call_id\":\"{}\",\"tool_id\":\"{}\",\"ok\":{}}}",
                    provider,
                    escape(tool_call_id),
                    escape(tool_id),
                    ok
                )
            }
            Self::ToolResultRecorded {
                tool_call_id,
                tool_id,
                artifact_id,
                content_hash,
                preview,
                provider_tool_call_id,
            } => {
                let provider = provider_tool_call_id
                    .as_deref()
                    .map(|v| format!("\"provider_tool_call_id\":\"{}\",", escape(v)))
                    .unwrap_or_default();
                format!(
                    "{{{}\"tool_call_id\":\"{}\",\"tool_id\":\"{}\",\"artifact_id\":\"{}\",\"content_hash\":\"{}\",\"preview\":\"{}\"}}",
                    provider,
                    escape(tool_call_id),
                    escape(tool_id),
                    escape(artifact_id),
                    escape(content_hash),
                    escape(preview)
                )
            }
            Self::ModelCallStarted {
                call_id,
                provider,
                adapter_id,
                actual_model_name,
                role,
                live,
                scaffold_level,
                prompt_tokens_estimate,
                prompt_hash,
                tool_catalog_hash,
                max_context_tokens,
                prompt_scaffold_budget,
                dynamic_context_budget,
                protected_reserve_tokens,
                budget_warning_count,
            } => format!(
                "{{\"call_id\":\"{}\",\"provider\":\"{}\",\"adapter_id\":\"{}\",\"actual_model_name\":\"{}\",\"role\":\"{}\",\"live\":{},\"scaffold_level\":\"{}\",\"prompt_tokens_estimate\":{},\"prompt_hash\":\"{}\",\"tool_catalog_hash\":\"{}\",\"max_context_tokens\":{},\"prompt_scaffold_budget\":{},\"dynamic_context_budget\":{},\"protected_reserve_tokens\":{},\"budget_warning_count\":{}}}",
                escape(call_id),
                escape(provider),
                escape(adapter_id),
                escape(actual_model_name),
                escape(role),
                live,
                escape(scaffold_level),
                prompt_tokens_estimate,
                escape(prompt_hash),
                escape(tool_catalog_hash),
                max_context_tokens,
                prompt_scaffold_budget,
                dynamic_context_budget,
                protected_reserve_tokens,
                budget_warning_count
            ),
            Self::ModelCallCompleted {
                call_id,
                provider,
                ok,
                artifact_id,
                content_hash,
            } => format!(
                "{{\"call_id\":\"{}\",\"provider\":\"{}\",\"ok\":{},\"artifact_id\":\"{}\",\"content_hash\":\"{}\"}}",
                escape(call_id),
                escape(provider),
                ok,
                escape(artifact_id),
                escape(content_hash)
            ),
            Self::ModelCallBlocked {
                call_id,
                provider,
                gate,
            } => format!(
                "{{\"call_id\":\"{}\",\"provider\":\"{}\",\"gate\":\"{}\"}}",
                escape(call_id),
                escape(provider),
                escape(gate)
            ),
            Self::ModelStreamDelta {
                stream_id,
                provider,
                delta_kind,
                preview,
                runtime_sanitized,
            } => format!(
                "{{\"stream_id\":\"{}\",\"provider\":\"{}\",\"delta_kind\":\"{}\",\"preview\":\"{}\",\"runtime_sanitized\":{}}}",
                escape(stream_id),
                escape(provider),
                escape(delta_kind),
                escape(preview),
                runtime_sanitized
            ),
            Self::ModelStreamCompleted {
                stream_id,
                provider,
                artifact_id,
                content_hash,
                prompt_tokens,
                completion_tokens,
                reasoning_tokens,
                prompt_cache_hit_tokens,
                prompt_cache_miss_tokens,
                stop_reason,
            } => format!(
                "{{\"stream_id\":\"{}\",\"provider\":\"{}\",\"artifact_id\":\"{}\",\"content_hash\":\"{}\",\"prompt_tokens\":{},\"completion_tokens\":{},\"reasoning_tokens\":{},\"prompt_cache_hit_tokens\":{},\"prompt_cache_miss_tokens\":{}{}}}",
                escape(stream_id),
                escape(provider),
                escape(artifact_id),
                escape(content_hash),
                prompt_tokens,
                completion_tokens,
                reasoning_tokens,
                prompt_cache_hit_tokens,
                prompt_cache_miss_tokens,
                stop_reason
                    .as_deref()
                    .map(|reason| format!(",\"stop_reason\":\"{}\"", escape(reason)))
                    .unwrap_or_default()
            ),
            Self::PatchProposalCreated { patch_id, path } => format!(
                "{{\"patch_id\":\"{}\",\"path\":\"{}\"}}",
                escape(patch_id),
                escape(path)
            ),
            Self::PatchProposalValidated {
                patch_id,
                validation,
            } => format!(
                "{{\"patch_id\":\"{}\",\"validation\":\"{}\"}}",
                escape(patch_id),
                escape(validation)
            ),
            Self::PatchApplied { patch_id, path } => format!(
                "{{\"patch_id\":\"{}\",\"path\":\"{}\"}}",
                escape(patch_id),
                escape(path)
            ),
        }
    }
}

pub fn state_to_str(state: AgentState) -> &'static str {
    match state {
        AgentState::Created => "Created",
        AgentState::Planning => "Planning",
        AgentState::WaitingForPlanApproval => "WaitingForPlanApproval",
        AgentState::RetrievingContext => "RetrievingContext",
        AgentState::Executing => "Executing",
        AgentState::WaitingForToolApproval => "WaitingForToolApproval",
        AgentState::ApplyingPatch => "ApplyingPatch",
        AgentState::RunningCommand => "RunningCommand",
        AgentState::DiagnosingFailure => "DiagnosingFailure",
        AgentState::Reviewing => "Reviewing",
        AgentState::WaitingForUser => "WaitingForUser",
        AgentState::Completed => "Completed",
        AgentState::Failed => "Failed",
        AgentState::Cancelled => "Cancelled",
    }
}

fn permission_request_type_to_str(value: &PermissionRequestType) -> &'static str {
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

fn permission_decision_to_str(value: &PermissionDecisionKind) -> &'static str {
    match value {
        PermissionDecisionKind::AllowOnce => "allow_once",
        PermissionDecisionKind::AllowSession => "allow_session",
        PermissionDecisionKind::AllowProjectRule => "allow_project_rule",
        PermissionDecisionKind::Deny => "deny",
        PermissionDecisionKind::Modify => "modify",
    }
}

fn plan_decision_to_str(value: &PlanApprovalDecisionKind) -> &'static str {
    match value {
        PlanApprovalDecisionKind::Approve => "approve",
        PlanApprovalDecisionKind::Reject => "reject",
        PlanApprovalDecisionKind::RequestRevision => "request_revision",
    }
}

fn escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            other if other.is_control() => format!("\\u{:04x}", other as u32).chars().collect(),
            other => vec![other],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_payload_keeps_security_type() {
        let payload = RuntimeEventPayload::PermissionRequested {
            permission_id: "perm_1".to_string(),
            request_type: PermissionRequestType::CloudModel,
            tool_id: None,
        };
        assert_eq!(
            payload.to_json(),
            "{\"permission_id\":\"perm_1\",\"request_type\":\"cloud_model\"}"
        );
    }
}
