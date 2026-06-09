//! Live model executor preflight.
//!
//! V0 does not perform network I/O. It records model call boundaries, evaluates
//! native provider gates, returns an auditable prepared request, and provides a
//! response-recording boundary for provider bodies supplied by a separate HTTP
//! transport.

use crate::artifact::ArtifactStore;
use crate::context_budget::allocate_native_context_budget;
use crate::live_model_request::{
    build_deepseek_anthropic_request, build_deepseek_anthropic_request_with_tools,
    build_deepseek_openai_request, build_deepseek_openai_request_with_tools,
    build_qwen_openai_request, build_qwen_openai_request_with_tools, ModelRequestMessage,
    PreparedModelHttpRequest,
};
use crate::model_adapter::{ModelRole, PlannedModelCall};
use crate::native_provider::{
    evaluate_native_live_call_gate, NativeLiveCallGate, NativeProviderEndpoint,
};
use crate::native_response_normalizer::{
    normalize_deepseek_anthropic_response, normalize_deepseek_openai_response,
    normalize_qwen_openai_response,
};
use crate::patch::stable_text_hash;
use crate::provider_response_adapter::{
    record_native_provider_response_after_started, record_native_provider_stream_after_started,
    NativeProviderResponseInput, NativeProviderResponseResult, NativeProviderStreamInput,
    NativeProviderStreamKind,
};
use crate::secret_scan::contains_high_severity_secret;
use crate::session::{AgentSession, SessionError};
use researchcode_kernel::context::estimate_tokens;
use researchcode_kernel::model::NativeModelFamily;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveModelExecutionRequest {
    pub call_id: String,
    pub role: String,
    pub endpoint: NativeProviderEndpoint,
    pub messages: Vec<ModelRequestMessage>,
    pub max_tokens: u64,
    pub stream: bool,
    pub tools_json: Option<String>,
    pub live_calls_enabled: bool,
    pub network_approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveModelExecutionStatus {
    Blocked,
    Prepared,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveModelExecutionResult {
    pub status: LiveModelExecutionStatus,
    pub gate: Option<NativeLiveCallGate>,
    pub prepared_request: Option<PreparedModelHttpRequest>,
}

#[derive(Debug, Clone)]
pub struct LiveModelResponseRecordRequest<'a> {
    pub call_id: &'a str,
    pub stream_id: &'a str,
    pub endpoint: &'a NativeProviderEndpoint,
    pub role: ModelRole,
    pub plan: &'a PlannedModelCall,
    pub request_preview: &'a str,
    pub transcript_id: &'a str,
    pub response_body: &'a str,
}

#[derive(Debug, Clone)]
pub struct LiveModelStreamRecordRequest<'a> {
    pub call_id: &'a str,
    pub stream_id: &'a str,
    pub endpoint: &'a NativeProviderEndpoint,
    pub role: ModelRole,
    pub plan: &'a PlannedModelCall,
    pub request_preview: &'a str,
    pub transcript_id: &'a str,
    pub response_sse_body: &'a str,
    pub record_content_deltas: bool,
}

pub fn prepare_live_model_execution(
    session: &mut AgentSession,
    request: &LiveModelExecutionRequest,
) -> Result<LiveModelExecutionResult, SessionError> {
    let provider = provider_label(&request.endpoint.family);
    let metadata = prompt_metadata(request);
    session.record_model_call_started_with_metadata(
        &request.call_id,
        provider,
        &request.endpoint.provider_id,
        &request.endpoint.actual_model_name,
        &request.role,
        true,
        metadata.scaffold_level,
        metadata.prompt_tokens_estimate,
        metadata.prompt_hash,
        metadata.tool_catalog_hash,
        metadata.max_context_tokens,
        metadata.prompt_scaffold_budget,
        metadata.dynamic_context_budget,
        metadata.protected_reserve_tokens,
        metadata.budget_warning_count,
    )?;
    if request
        .messages
        .iter()
        .any(|message| contains_high_severity_secret(&message.content))
    {
        session.record_model_call_blocked(&request.call_id, provider, "secret_detected")?;
        return Ok(LiveModelExecutionResult {
            status: LiveModelExecutionStatus::Blocked,
            gate: Some(NativeLiveCallGate::SecretDetected),
            prepared_request: None,
        });
    }
    let gate = evaluate_native_live_call_gate(
        &request.endpoint,
        request.live_calls_enabled,
        request.network_approved,
    );
    if gate != NativeLiveCallGate::Allowed {
        session.record_model_call_blocked(&request.call_id, provider, gate_to_str(&gate))?;
        return Ok(LiveModelExecutionResult {
            status: LiveModelExecutionStatus::Blocked,
            gate: Some(gate),
            prepared_request: None,
        });
    }
    let prepared = match request.endpoint.family {
        NativeModelFamily::DeepSeek => match (
            request.endpoint.protocol.as_str(),
            request.tools_json.as_deref(),
        ) {
            ("openai_compatible", Some(tools_json)) => build_deepseek_openai_request_with_tools(
                &request.endpoint,
                &request.messages,
                request.max_tokens,
                request.stream,
                tools_json,
            ),
            ("openai_compatible", None) => build_deepseek_openai_request(
                &request.endpoint,
                &request.messages,
                request.max_tokens,
                request.stream,
            ),
            ("anthropic_compatible", Some(tools_json)) => {
                build_deepseek_anthropic_request_with_tools(
                    &request.endpoint,
                    &request.messages,
                    request.max_tokens,
                    request.stream,
                    tools_json,
                )
            }
            ("anthropic_compatible", None) => build_deepseek_anthropic_request(
                &request.endpoint,
                &request.messages,
                request.max_tokens,
                request.stream,
            ),
            (other, _) => Err(format!(
                "DeepSeek family does not support protocol '{}'; expected 'openai_compatible' or 'anthropic_compatible'",
                other
            )),
        },
        NativeModelFamily::Qwen => match request.tools_json.as_deref() {
            Some(tools_json) => build_qwen_openai_request_with_tools(
                &request.endpoint,
                &request.messages,
                request.max_tokens,
                request.stream,
                tools_json,
            ),
            None => build_qwen_openai_request(
                &request.endpoint,
                &request.messages,
                request.max_tokens,
                request.stream,
            ),
        },
    };
    match prepared {
        Ok(prepared_request) => Ok(LiveModelExecutionResult {
            status: LiveModelExecutionStatus::Prepared,
            gate: Some(NativeLiveCallGate::Allowed),
            prepared_request: Some(prepared_request),
        }),
        Err(error) => {
            session.record_model_call_blocked(
                &request.call_id,
                provider,
                "invalid_request_shape",
            )?;
            Ok(LiveModelExecutionResult {
                status: LiveModelExecutionStatus::Blocked,
                gate: Some(NativeLiveCallGate::InvalidEndpoint(error)),
                prepared_request: None,
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptTelemetryMetadata {
    scaffold_level: String,
    prompt_tokens_estimate: u64,
    prompt_hash: String,
    tool_catalog_hash: String,
    max_context_tokens: u64,
    prompt_scaffold_budget: u64,
    dynamic_context_budget: u64,
    protected_reserve_tokens: u64,
    budget_warning_count: u64,
}

fn prompt_metadata(request: &LiveModelExecutionRequest) -> PromptTelemetryMetadata {
    let role = model_role_from_str(&request.role).unwrap_or(ModelRole::Executor);
    let budget = allocate_native_context_budget(request.endpoint.family.clone(), role, None);
    let prompt_text = request
        .messages
        .iter()
        .map(|message| format!("{}:{}", message.role, message.content))
        .collect::<Vec<_>>()
        .join("\n");
    let tool_catalog = request
        .messages
        .iter()
        .filter_map(|message| {
            extract_between(&message.content, "<tool_catalog>", "</tool_catalog>")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let budget_warning_count = u64::from(
        estimate_tokens(&prompt_text)
            > budget.max_context_tokens - budget.protected_reserve_tokens(),
    );
    PromptTelemetryMetadata {
        scaffold_level: format!("{:?}", budget.scaffold_level),
        prompt_tokens_estimate: estimate_tokens(&prompt_text),
        prompt_hash: stable_text_hash(&prompt_text),
        tool_catalog_hash: if tool_catalog.is_empty() {
            "none".to_string()
        } else {
            stable_text_hash(&tool_catalog)
        },
        max_context_tokens: budget.max_context_tokens,
        prompt_scaffold_budget: budget.prompt_scaffold_tokens(),
        dynamic_context_budget: budget.dynamic_context_tokens(),
        protected_reserve_tokens: budget.protected_reserve_tokens(),
        budget_warning_count,
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

fn extract_between(text: &str, start: &str, end: &str) -> Option<String> {
    let left = text.find(start)?;
    let right = text[left + start.len()..].find(end)? + left + start.len();
    Some(text[left + start.len()..right].to_string())
}

pub fn record_live_model_response(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    request: LiveModelResponseRecordRequest<'_>,
) -> Result<NativeProviderResponseResult, String> {
    let normalized = match request.endpoint.family {
        NativeModelFamily::DeepSeek if request.endpoint.protocol == "openai_compatible" => {
            normalize_deepseek_openai_response(request.response_body)?
        }
        NativeModelFamily::DeepSeek => {
            normalize_deepseek_anthropic_response(request.response_body)?
        }
        NativeModelFamily::Qwen => normalize_qwen_openai_response(request.response_body)?,
    };
    let visible_content = if normalized.tool_calls_dsml.is_empty() {
        normalized.visible_content.clone()
    } else if normalized.visible_content.trim().is_empty() {
        normalized.tool_calls_dsml.clone()
    } else {
        format!(
            "{}\n{}",
            normalized.visible_content, normalized.tool_calls_dsml
        )
    };
    record_native_provider_response_after_started(
        session,
        artifact_store,
        NativeProviderResponseInput {
            provider: normalized.provider,
            call_id: request.call_id,
            stream_id: request.stream_id,
            role: request.role,
            plan: request.plan,
            request_preview: request.request_preview,
            transcript_id: request.transcript_id,
            live: true,
            visible_content: &visible_content,
            hidden_reasoning_sanitized: normalized.hidden_reasoning_sanitized.as_deref(),
            prompt_tokens: normalized.prompt_tokens,
            completion_tokens: normalized.completion_tokens,
            reasoning_tokens: normalized.reasoning_tokens,
            prompt_cache_hit_tokens: normalized.prompt_cache_hit_tokens,
            prompt_cache_miss_tokens: normalized.prompt_cache_miss_tokens,
        },
    )
}

pub fn record_live_model_stream_response(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    request: LiveModelStreamRecordRequest<'_>,
) -> Result<NativeProviderResponseResult, String> {
    let lines = request.response_sse_body.lines().collect::<Vec<_>>();
    record_native_provider_stream_after_started(
        session,
        artifact_store,
        NativeProviderStreamInput {
            provider: stream_kind_for_family(&request.endpoint.family),
            call_id: request.call_id,
            stream_id: request.stream_id,
            role: request.role,
            plan: request.plan,
            request_preview: request.request_preview,
            transcript_id: request.transcript_id,
            live: true,
            lines: &lines,
            record_content_deltas: request.record_content_deltas,
        },
    )
}

pub fn gate_to_str(gate: &NativeLiveCallGate) -> &'static str {
    match gate {
        NativeLiveCallGate::Allowed => "allowed",
        NativeLiveCallGate::DisabledByDefault => "disabled_by_default",
        NativeLiveCallGate::MissingApiKeyEnv => "missing_api_key_env",
        NativeLiveCallGate::MissingApiKeyValue => "missing_api_key_value",
        NativeLiveCallGate::NetworkApprovalRequired => "network_approval_required",
        NativeLiveCallGate::SecretDetected => "secret_detected",
        NativeLiveCallGate::InvalidEndpoint(_) => "invalid_endpoint",
    }
}

fn provider_label(family: &NativeModelFamily) -> &'static str {
    match family {
        NativeModelFamily::DeepSeek => "deepseek",
        NativeModelFamily::Qwen => "qwen",
    }
}

fn stream_kind_for_family(family: &NativeModelFamily) -> NativeProviderStreamKind {
    match family {
        NativeModelFamily::DeepSeek => NativeProviderStreamKind::DeepSeek,
        NativeModelFamily::Qwen => NativeProviderStreamKind::Qwen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::ArtifactStore;
    use crate::model_adapter::{DeepSeekNativeAdapter, ModelAdapter, ModelAdapterRequest};
    use crate::state::AgentState;
    use researchcode_kernel::model::{NativeModelProfile, OptimizationLevel};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn message() -> Vec<ModelRequestMessage> {
        vec![ModelRequestMessage {
            role: "user".to_string(),
            content: "Plan the task".to_string(),
            cache_control_ttl: None,
        }]
    }

    #[test]
    fn preflight_blocks_deepseek_by_default_and_records_event() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        let result = prepare_live_model_execution(
            &mut session,
            &LiveModelExecutionRequest {
                call_id: "call_1".to_string(),
                role: "planner".to_string(),
                endpoint: NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
                messages: message(),
                max_tokens: 1024,
                stream: true,
                tools_json: None,
                live_calls_enabled: true,
                network_approved: true,
            },
        )
        .unwrap();
        assert_eq!(result.status, LiveModelExecutionStatus::Blocked);
        assert_eq!(result.gate, Some(NativeLiveCallGate::DisabledByDefault));
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"model.call_started\""));
        assert!(jsonl.contains("\"event_type\":\"model.call_blocked\""));
        assert!(jsonl.contains("\"gate\":\"disabled_by_default\""));
        assert!(!jsonl.contains("sk-"));
    }

    #[test]
    fn preflight_blocks_before_network_approval() {
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.live_calls_enabled_by_default = true;
        endpoint.api_key_env = "PATH".to_string();
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        let result = prepare_live_model_execution(
            &mut session,
            &LiveModelExecutionRequest {
                call_id: "call_1".to_string(),
                role: "planner".to_string(),
                endpoint,
                messages: message(),
                max_tokens: 1024,
                stream: true,
                tools_json: None,
                live_calls_enabled: true,
                network_approved: false,
            },
        )
        .unwrap();
        assert_eq!(
            result.gate,
            Some(NativeLiveCallGate::NetworkApprovalRequired)
        );
    }

    #[test]
    fn preflight_blocks_qwen_unresolved_endpoint_shape() {
        let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        endpoint.live_calls_enabled_by_default = true;
        endpoint.api_key_env = "PATH".to_string();
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        let result = prepare_live_model_execution(
            &mut session,
            &LiveModelExecutionRequest {
                call_id: "call_1".to_string(),
                role: "executor".to_string(),
                endpoint,
                messages: message(),
                max_tokens: 1024,
                stream: true,
                tools_json: None,
                live_calls_enabled: true,
                network_approved: true,
            },
        )
        .unwrap();
        assert_eq!(result.status, LiveModelExecutionStatus::Blocked);
        assert!(matches!(
            result.gate,
            Some(NativeLiveCallGate::InvalidEndpoint(_))
        ));
    }

    #[test]
    fn preflight_blocks_secret_before_request_build() {
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.live_calls_enabled_by_default = true;
        endpoint.api_key_env = "PATH".to_string();
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        let result = prepare_live_model_execution(
            &mut session,
            &LiveModelExecutionRequest {
                call_id: "call_1".to_string(),
                role: "planner".to_string(),
                endpoint,
                messages: vec![ModelRequestMessage {
                    role: "user".to_string(),
                    content: "do not send sk-testsecret123456789".to_string(),
                    cache_control_ttl: None,
                }],
                max_tokens: 1024,
                stream: true,
                tools_json: None,
                live_calls_enabled: true,
                network_approved: true,
            },
        )
        .unwrap();
        assert_eq!(result.gate, Some(NativeLiveCallGate::SecretDetected));
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"gate\":\"secret_detected\""));
        assert!(!jsonl.contains("sk-testsecret"));
    }

    #[test]
    fn prepared_live_response_records_through_native_adapter_once() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-live-response-{nonce}"));
        let store = ArtifactStore::new(root.join("artifacts"));
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.live_calls_enabled_by_default = true;
        endpoint.api_key_env = "PATH".to_string();
        let adapter = DeepSeekNativeAdapter::new(
            NativeModelProfile {
                profile_id: "deepseek-v4-native".to_string(),
                family: NativeModelFamily::DeepSeek,
                optimization_level: OptimizationLevel::Native,
            },
            "deepseek-v4-flash",
        )
        .unwrap();
        let plan = adapter
            .plan_call(&ModelAdapterRequest {
                role: ModelRole::Planner,
                task_summary: "live response".to_string(),
                requires_tools: false,
                context_tokens_estimate: 2_000,
            })
            .unwrap();
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        let prepared = prepare_live_model_execution(
            &mut session,
            &LiveModelExecutionRequest {
                call_id: "live_call_1".to_string(),
                role: "planner".to_string(),
                endpoint: endpoint.clone(),
                messages: message(),
                max_tokens: 1024,
                stream: false,
                tools_json: None,
                live_calls_enabled: true,
                network_approved: true,
            },
        )
        .unwrap();
        assert_eq!(prepared.status, LiveModelExecutionStatus::Prepared);
        let result = record_live_model_response(
            &mut session,
            &store,
            LiveModelResponseRecordRequest {
                call_id: "live_call_1",
                stream_id: "live_stream_1",
                endpoint: &endpoint,
                role: ModelRole::Planner,
                plan: &plan,
                request_preview: "request preview",
                transcript_id: "live_transcript_1",
                response_body: r#"{"model":"deepseek-v4-flash","content":[{"type":"text","text":"Visible live response with sk-responsesecret and .env"}],"reasoning_content":"Need sk-reasoningsecret from .env","usage":{"input_tokens":10,"output_tokens":5,"reasoning_tokens":3,"cache_read_input_tokens":7,"cache_creation_input_tokens":3}}"#,
            },
        )
        .unwrap();
        assert_eq!(result.prompt_tokens, 10);
        assert_eq!(result.prompt_cache_hit_tokens, 7);
        let jsonl = session.export_events_jsonl();
        assert_eq!(
            jsonl
                .matches("\"event_type\":\"model.call_started\"")
                .count(),
            1
        );
        assert!(jsonl.contains("\"event_type\":\"model.call_completed\""));
        assert!(!jsonl.contains("sk-responsesecret"));
        assert!(!jsonl.contains("sk-reasoningsecret"));
        assert!(!jsonl.contains(".env"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn deepseek_openai_non_stream_response_preserves_tool_calls() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-live-openai-response-{nonce}"));
        let store = ArtifactStore::new(root.join("artifacts"));
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
        endpoint.live_calls_enabled_by_default = true;
        endpoint.api_key_env = "PATH".to_string();
        let adapter = DeepSeekNativeAdapter::new(
            NativeModelProfile {
                profile_id: "deepseek-v4-native".to_string(),
                family: NativeModelFamily::DeepSeek,
                optimization_level: OptimizationLevel::Native,
            },
            "deepseek-v4-flash",
        )
        .unwrap();
        let plan = adapter
            .plan_call(&ModelAdapterRequest {
                role: ModelRole::Executor,
                task_summary: "live response".to_string(),
                requires_tools: true,
                context_tokens_estimate: 2_000,
            })
            .unwrap();
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        prepare_live_model_execution(
            &mut session,
            &LiveModelExecutionRequest {
                call_id: "live_call_openai_1".to_string(),
                role: "executor".to_string(),
                endpoint: endpoint.clone(),
                messages: message(),
                max_tokens: 1024,
                stream: false,
                tools_json: None,
                live_calls_enabled: true,
                network_approved: true,
            },
        )
        .unwrap();
        let result = record_live_model_response(
            &mut session,
            &store,
            LiveModelResponseRecordRequest {
                call_id: "live_call_openai_1",
                stream_id: "live_stream_openai_1",
                endpoint: &endpoint,
                role: ModelRole::Executor,
                plan: &plan,
                request_preview: "request preview",
                transcript_id: "live_transcript_openai_1",
                response_body: r#"{"model":"deepseek-v4-flash","choices":[{"message":{"content":"","tool_calls":[{"id":"call_read","type":"function","function":{"name":"file_read","arguments":"{\"path\":\"README.md\"}"}}]}}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#,
            },
        )
        .unwrap();
        assert!(
            result.visible_content_preview.contains("tool_calls"),
            "{}",
            result.visible_content_preview
        );
        assert!(result
            .visible_content_preview
            .contains("<｜｜DSML｜｜invoke name=\"file_read\" id=\"call_read\">"));
        assert!(result
            .visible_content_preview
            .contains("<｜｜DSML｜｜parameter name=\"path\" string=\"true\">README.md"));
        let _ = fs::remove_dir_all(root);
    }
}
