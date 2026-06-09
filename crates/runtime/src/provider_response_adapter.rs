//! Native provider response adapter boundary.
//!
//! This module is the single runtime path that turns approved DeepSeek/Qwen
//! provider stream fragments into Product Kernel model events and transcript
//! artifacts. It intentionally does not perform network I/O.

use crate::artifact::{ArtifactRecord, ArtifactStore};
use crate::model_adapter::{ModelRole, PlannedModelCall};
use crate::model_transcript::{
    sanitize_transcript_text, write_model_transcript_artifact, ModelTranscript,
};
use crate::native_profile::deepseek::stream::{
    assemble_deepseek_sse_lines, parse_deepseek_sse_line_all, DeepSeekStreamDelta,
};
use crate::native_profile::deepseek::stream_processor::StreamProcessor;
use crate::qwen_stream::{assemble_qwen_sse_lines, parse_qwen_sse_line_all, QwenStreamDelta};
use crate::session::{AgentSession, SessionError};
use crate::tcml::visible_text_without_tool_calls;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeProviderStreamKind {
    DeepSeek,
    Qwen,
}

#[derive(Debug, Clone)]
pub struct NativeProviderStreamInput<'a> {
    pub provider: NativeProviderStreamKind,
    pub call_id: &'a str,
    pub stream_id: &'a str,
    pub role: ModelRole,
    pub plan: &'a PlannedModelCall,
    pub request_preview: &'a str,
    pub transcript_id: &'a str,
    pub live: bool,
    pub lines: &'a [&'a str],
    pub record_content_deltas: bool,
}

#[derive(Debug, Clone)]
pub struct NativeProviderResponseInput<'a> {
    pub provider: NativeProviderStreamKind,
    pub call_id: &'a str,
    pub stream_id: &'a str,
    pub role: ModelRole,
    pub plan: &'a PlannedModelCall,
    pub request_preview: &'a str,
    pub transcript_id: &'a str,
    pub live: bool,
    pub visible_content: &'a str,
    pub hidden_reasoning_sanitized: Option<&'a str>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    pub prompt_cache_hit_tokens: u64,
    pub prompt_cache_miss_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeProviderStreamResult {
    pub provider: String,
    pub call_id: String,
    pub stream_id: String,
    pub artifact_id: String,
    pub content_hash: String,
    pub visible_content_preview: String,
    pub hidden_reasoning_persisted: bool,
    /// Raw provider reasoning for same-turn continuation only.
    ///
    /// This field is intentionally not recorded in AgentEvent, transcript
    /// artifacts, JSONL exports, or visible UI. DeepSeek thinking-mode tool
    /// continuations need it, but it must remain in process memory.
    pub volatile_reasoning_content: Option<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    pub prompt_cache_hit_tokens: u64,
    pub prompt_cache_miss_tokens: u64,
    pub stop_reason: Option<String>,
}

pub type NativeProviderResponseResult = NativeProviderStreamResult;

pub fn record_native_provider_stream(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    input: NativeProviderStreamInput<'_>,
) -> Result<NativeProviderStreamResult, String> {
    record_native_provider_stream_inner(session, artifact_store, input, true)
}

pub fn record_native_provider_stream_after_started(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    input: NativeProviderStreamInput<'_>,
) -> Result<NativeProviderStreamResult, String> {
    record_native_provider_stream_inner(session, artifact_store, input, false)
}

fn record_native_provider_stream_inner(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    input: NativeProviderStreamInput<'_>,
    record_call_started: bool,
) -> Result<NativeProviderStreamResult, String> {
    let provider = provider_to_str(input.provider);
    if record_call_started {
        session
            .record_model_call_started(
                input.call_id,
                provider,
                &input.plan.adapter_id,
                &input.plan.actual_model_name,
                role_to_str(&input.role),
                input.live,
            )
            .map_err(session_error)?;
    }

    match input.provider {
        NativeProviderStreamKind::DeepSeek => {
            let assembly = assemble_deepseek_sse_lines(input.lines)?;
            let visible_content =
                stream_visible_content(&assembly.content, &assembly.tool_call_pairs());
            let recorded_deltas = record_deepseek_stream_deltas(
                session,
                input.stream_id,
                provider,
                input.lines,
                input.record_content_deltas,
            )?;
            if recorded_deltas == 0 && input.record_content_deltas && !visible_content.is_empty() {
                session
                    .record_model_stream_delta(
                        input.stream_id,
                        provider,
                        "content",
                        &visible_content,
                    )
                    .map_err(session_error)?;
            }
            let transcript = ModelTranscript::from_deepseek_stream_assembly(
                input.transcript_id,
                input.role.clone(),
                input.plan,
                input.request_preview,
                &assembly,
            );
            let record = write_model_transcript_artifact(artifact_store, &transcript)
                .map_err(|error| error.to_string())?;
            let result = NativeProviderStreamResult::from_record(
                input,
                record,
                visible_content,
                !assembly.reasoning_sanitized.is_empty(),
                Some(assembly.reasoning_raw_volatile.clone())
                    .filter(|value| !value.trim().is_empty()),
                assembly.telemetry.prompt_tokens.unwrap_or(0),
                assembly.telemetry.completion_tokens.unwrap_or(0),
                assembly.telemetry.reasoning_tokens.unwrap_or(0),
                assembly.telemetry.prompt_cache_hit_tokens.unwrap_or(0),
                assembly.telemetry.prompt_cache_miss_tokens.unwrap_or(0),
                assembly.stop_reason.clone(),
            );
            record_completion_events(session, &result)?;
            Ok(result)
        }
        NativeProviderStreamKind::Qwen => {
            let assembly = assemble_qwen_sse_lines(input.lines)?;
            let visible_content =
                stream_visible_content(&assembly.content, &assembly.tool_call_pairs());
            let recorded_deltas = record_qwen_stream_deltas(
                session,
                input.stream_id,
                provider,
                input.lines,
                input.record_content_deltas,
            )?;
            if recorded_deltas == 0 && input.record_content_deltas && !visible_content.is_empty() {
                session
                    .record_model_stream_delta(
                        input.stream_id,
                        provider,
                        "content",
                        &visible_content,
                    )
                    .map_err(session_error)?;
            }
            let transcript = ModelTranscript::from_qwen_stream_assembly(
                input.transcript_id,
                input.role.clone(),
                input.plan,
                input.request_preview,
                &assembly,
            );
            let record = write_model_transcript_artifact(artifact_store, &transcript)
                .map_err(|error| error.to_string())?;
            let result = NativeProviderStreamResult::from_record(
                input,
                record,
                visible_content,
                !assembly.thinking_sanitized.is_empty(),
                None,
                assembly.telemetry.prompt_tokens.unwrap_or(0),
                assembly.telemetry.completion_tokens.unwrap_or(0),
                0,
                0,
                0,
                assembly.stop_reason.clone(),
            );
            record_completion_events(session, &result)?;
            Ok(result)
        }
    }
}

pub fn record_native_provider_response(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    input: NativeProviderResponseInput<'_>,
) -> Result<NativeProviderResponseResult, String> {
    record_native_provider_response_inner(session, artifact_store, input, true)
}

pub fn record_native_provider_response_after_started(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    input: NativeProviderResponseInput<'_>,
) -> Result<NativeProviderResponseResult, String> {
    record_native_provider_response_inner(session, artifact_store, input, false)
}

fn record_native_provider_response_inner(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    input: NativeProviderResponseInput<'_>,
    record_call_started: bool,
) -> Result<NativeProviderResponseResult, String> {
    let provider = provider_to_str(input.provider);
    if record_call_started {
        session
            .record_model_call_started(
                input.call_id,
                provider,
                &input.plan.adapter_id,
                &input.plan.actual_model_name,
                role_to_str(&input.role),
                input.live,
            )
            .map_err(session_error)?;
    }

    let visible_content = sanitize_transcript_text(input.visible_content);
    let visible_content_for_user = visible_text_without_tool_calls(&visible_content);
    let hidden_reasoning = sanitize_transcript_text(input.hidden_reasoning_sanitized.unwrap_or(""));
    if !hidden_reasoning.is_empty() {
        let delta_kind = match input.provider {
            NativeProviderStreamKind::DeepSeek => "reasoning_sanitized",
            NativeProviderStreamKind::Qwen => "thinking_sanitized",
        };
        session
            .record_model_stream_delta(input.stream_id, provider, delta_kind, &hidden_reasoning)
            .map_err(session_error)?;
    }
    if !visible_content_for_user.is_empty() {
        session
            .record_model_stream_delta(
                input.stream_id,
                provider,
                "content",
                &visible_content_for_user,
            )
            .map_err(session_error)?;
    }

    let mut transcript = ModelTranscript::from_planned_call(
        input.transcript_id,
        input.role.clone(),
        input.plan,
        input.request_preview,
        &visible_content_for_user,
    );
    transcript.prompt_tokens_estimate = input.prompt_tokens;
    transcript.response_tokens_estimate = input.completion_tokens;
    transcript.reasoning_persisted = !hidden_reasoning.is_empty();
    let record = write_model_transcript_artifact(artifact_store, &transcript)
        .map_err(|error| error.to_string())?;
    let result = NativeProviderStreamResult {
        provider: provider.to_string(),
        call_id: input.call_id.to_string(),
        stream_id: input.stream_id.to_string(),
        artifact_id: record.artifact_id,
        content_hash: record.content_hash,
        visible_content_preview: visible_content,
        hidden_reasoning_persisted: transcript.reasoning_persisted,
        volatile_reasoning_content: None,
        prompt_tokens: input.prompt_tokens,
        completion_tokens: input.completion_tokens,
        reasoning_tokens: input.reasoning_tokens,
        prompt_cache_hit_tokens: input.prompt_cache_hit_tokens,
        prompt_cache_miss_tokens: input.prompt_cache_miss_tokens,
        stop_reason: None,
    };
    record_completion_events(session, &result)?;
    Ok(result)
}

impl NativeProviderStreamResult {
    fn from_record(
        input: NativeProviderStreamInput<'_>,
        record: ArtifactRecord,
        visible_content_preview: String,
        hidden_reasoning_persisted: bool,
        volatile_reasoning_content: Option<String>,
        prompt_tokens: u64,
        completion_tokens: u64,
        reasoning_tokens: u64,
        prompt_cache_hit_tokens: u64,
        prompt_cache_miss_tokens: u64,
        stop_reason: Option<String>,
    ) -> Self {
        Self {
            provider: provider_to_str(input.provider).to_string(),
            call_id: input.call_id.to_string(),
            stream_id: input.stream_id.to_string(),
            artifact_id: record.artifact_id,
            content_hash: record.content_hash,
            visible_content_preview,
            hidden_reasoning_persisted,
            volatile_reasoning_content,
            prompt_tokens,
            completion_tokens,
            reasoning_tokens,
            prompt_cache_hit_tokens,
            prompt_cache_miss_tokens,
            stop_reason,
        }
    }
}

fn record_completion_events(
    session: &mut AgentSession,
    result: &NativeProviderStreamResult,
) -> Result<(), String> {
    session
        .record_model_stream_completed(
            &result.stream_id,
            &result.provider,
            &result.artifact_id,
            &result.content_hash,
            result.prompt_tokens,
            result.completion_tokens,
            result.reasoning_tokens,
            result.prompt_cache_hit_tokens,
            result.prompt_cache_miss_tokens,
            result.stop_reason.as_deref(),
        )
        .map_err(session_error)?;
    session
        .record_model_call_completed(
            &result.call_id,
            &result.provider,
            true,
            &result.artifact_id,
            &result.content_hash,
        )
        .map_err(session_error)
}

fn record_deepseek_stream_deltas(
    session: &mut AgentSession,
    stream_id: &str,
    provider: &str,
    lines: &[&str],
    record_content_deltas: bool,
) -> Result<usize, String> {
    let mut recorded = 0usize;
    let mut stream_processor = StreamProcessor::default();
    for line in lines {
        for delta in parse_deepseek_sse_line_all(line)? {
            match delta {
                DeepSeekStreamDelta::Reasoning {
                    sanitized_delta, ..
                } => {
                    if !sanitized_delta.is_empty() {
                        session
                            .record_model_stream_delta(
                                stream_id,
                                provider,
                                "reasoning_sanitized",
                                &sanitized_delta,
                            )
                            .map_err(session_error)?;
                        recorded += 1;
                    }
                }
                DeepSeekStreamDelta::Content { delta } => {
                    if !record_content_deltas {
                        continue;
                    }
                    let user_visible =
                        deepseek_stream_delta_user_visible(&delta, &mut stream_processor);
                    if !user_visible.is_empty() {
                        session
                            .record_model_stream_delta(
                                stream_id,
                                provider,
                                "content",
                                &user_visible,
                            )
                            .map_err(session_error)?;
                        recorded += 1;
                    }
                }
                _ => {}
            }
        }
    }
    Ok(recorded)
}

fn record_qwen_stream_deltas(
    session: &mut AgentSession,
    stream_id: &str,
    provider: &str,
    lines: &[&str],
    record_content_deltas: bool,
) -> Result<usize, String> {
    let mut recorded = 0usize;
    for line in lines {
        for delta in parse_qwen_sse_line_all(line)? {
            match delta {
                QwenStreamDelta::Thinking { sanitized_delta } => {
                    if !sanitized_delta.is_empty() {
                        session
                            .record_model_stream_delta(
                                stream_id,
                                provider,
                                "thinking_sanitized",
                                &sanitized_delta,
                            )
                            .map_err(session_error)?;
                        recorded += 1;
                    }
                }
                QwenStreamDelta::Content { delta } => {
                    if !record_content_deltas {
                        continue;
                    }
                    let user_visible = stream_delta_user_visible(&delta);
                    if !user_visible.is_empty() {
                        session
                            .record_model_stream_delta(
                                stream_id,
                                provider,
                                "content",
                                &user_visible,
                            )
                            .map_err(session_error)?;
                        recorded += 1;
                    }
                }
                _ => {}
            }
        }
    }
    Ok(recorded)
}

fn stream_visible_content(content: &str, tool_calls: &[(String, String)]) -> String {
    let content = suppress_tool_budget_refusal_text(&sanitize_transcript_text(content));
    if tool_calls.is_empty() {
        return content;
    };
    let calls = tool_calls
        .iter()
        .map(|(tool_name, tool_arguments)| {
            let arguments = if tool_arguments.trim().starts_with('{') {
                tool_arguments.trim().to_string()
            } else {
                "{}".to_string()
            };
            format!(
                "{{\"name\":\"{}\",\"arguments\":{}}}",
                escape_json(tool_name),
                arguments
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let tool_payload = sanitize_transcript_text(&format!("{{\"tool_calls\":[{}]}}", calls));
    if content.is_empty() {
        tool_payload
    } else {
        format!("{content}\n{tool_payload}")
    }
}

fn stream_delta_user_visible(delta: &str) -> String {
    let sanitized = sanitize_transcript_text(delta);
    suppress_tool_budget_refusal_text(&visible_text_without_tool_calls(&sanitized))
}

fn deepseek_stream_delta_user_visible(
    delta: &str,
    stream_processor: &mut StreamProcessor,
) -> String {
    let _ = stream_processor.ingest(
        crate::live_http_transport::LiveHttpStreamEvent::VisibleTextDelta(delta.to_string()),
    );
    let (filtered, _) = stream_processor.take_pending_content();
    if filtered.is_empty() {
        return String::new();
    }
    stream_delta_user_visible(&filtered)
}

fn suppress_tool_budget_refusal_text(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lowered = trimmed.to_ascii_lowercase();
    let cn_budget = trimmed.contains("工具调用次数")
        || trimmed.contains("工具调用已达上限")
        || trimmed.contains("工具调用达到上限")
        || trimmed.contains("调用次数已达上限");
    let cn_refusal = trimmed.contains("无法继续")
        || trimmed.contains("不能继续")
        || trimmed.contains("无法读取")
        || trimmed.contains("不能读取");
    let en_budget = lowered.contains("tool-call budget")
        || lowered.contains("tool call budget")
        || lowered.contains("tool limit")
        || lowered.contains("max tool calls")
        || lowered.contains("tool calls are exhausted");
    let en_refusal = lowered.contains("cannot continue")
        || lowered.contains("can't continue")
        || lowered.contains("unable to continue")
        || lowered.contains("cannot read")
        || lowered.contains("unable to read");
    if (cn_budget && cn_refusal) || (en_budget && en_refusal) {
        String::new()
    } else {
        value.to_string()
    }
}

fn escape_json(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn provider_to_str(provider: NativeProviderStreamKind) -> &'static str {
    match provider {
        NativeProviderStreamKind::DeepSeek => "deepseek",
        NativeProviderStreamKind::Qwen => "qwen",
    }
}

fn role_to_str(role: &ModelRole) -> &'static str {
    match role {
        ModelRole::Planner => "planner",
        ModelRole::Executor => "executor",
        ModelRole::Reviewer => "reviewer",
        ModelRole::Researcher => "researcher",
        ModelRole::Summarizer => "summarizer",
    }
}

fn session_error(error: SessionError) -> String {
    format!("{error:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::ArtifactStore;
    use crate::model_adapter::{
        DeepSeekNativeAdapter, ModelAdapter, ModelAdapterRequest, QwenNativeAdapter,
    };
    use crate::state::AgentState;
    use researchcode_kernel::model::{NativeModelFamily, NativeModelProfile, OptimizationLevel};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn records_deepseek_stream_as_sanitized_model_events() {
        let (root, store) = temp_store("deepseek-response-adapter");
        let mut session = ready_session();
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
                task_summary: "plan".to_string(),
                requires_tools: true,
                context_tokens_estimate: 2_000,
            })
            .unwrap();
        let result = record_native_provider_stream(
            &mut session,
            &store,
            NativeProviderStreamInput {
                provider: NativeProviderStreamKind::DeepSeek,
                call_id: "call_1",
                stream_id: "stream_1",
                role: ModelRole::Planner,
                plan: &plan,
                request_preview: "request",
                transcript_id: "transcript_1",
                live: false,
                record_content_deltas: true,
                lines: &[
                    r#"data: {"choices":[{"delta":{"reasoning_content":"Need sk-testsecret from .env"}}]}"#,
                    r#"data: {"choices":[{"delta":{"content":"Visible"}}]}"#,
                    r#"data: {"usage":{"prompt_tokens":100,"completion_tokens":20,"reasoning_tokens":15,"prompt_cache_hit_tokens":80,"prompt_cache_miss_tokens":20}}"#,
                    "data: [DONE]",
                ],
            },
        )
        .unwrap();
        assert_eq!(result.provider, "deepseek");
        assert_eq!(result.visible_content_preview, "Visible");
        assert!(result.hidden_reasoning_persisted);
        assert_eq!(
            result.volatile_reasoning_content.as_deref(),
            Some("Need sk-testsecret from .env")
        );
        assert_eq!(result.reasoning_tokens, 15);
        assert_eq!(result.prompt_cache_hit_tokens, 80);
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"model.call_started\""));
        assert!(jsonl.contains("\"event_type\":\"model.stream_completed\""));
        assert!(!jsonl.contains("sk-testsecret"));
        assert!(!jsonl.contains(".env"));
        assert!(!jsonl.contains("volatile_reasoning_content"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn records_qwen_stream_and_rejects_wrong_deployment() {
        let (root, store) = temp_store("qwen-response-adapter");
        let mut session = ready_session();
        let adapter = QwenNativeAdapter::new(
            NativeModelProfile {
                profile_id: "qwen3-6-27b-native".to_string(),
                family: NativeModelFamily::Qwen,
                optimization_level: OptimizationLevel::Native,
            },
            "Qwen/Qwen3.6-27B",
        )
        .unwrap();
        let plan = adapter
            .plan_call(&ModelAdapterRequest {
                role: ModelRole::Executor,
                task_summary: "execute".to_string(),
                requires_tools: true,
                context_tokens_estimate: 2_000,
            })
            .unwrap();
        let result = record_native_provider_stream(
            &mut session,
            &store,
            NativeProviderStreamInput {
                provider: NativeProviderStreamKind::Qwen,
                call_id: "call_1",
                stream_id: "stream_1",
                role: ModelRole::Executor,
                plan: &plan,
                request_preview: "request",
                transcript_id: "transcript_1",
                live: false,
                record_content_deltas: true,
                lines: &[
                    r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"reasoning_content":"Need sk-testsecret from .env"}}]}"#,
                    r#"data: {"choices":[{"delta":{"content":"Visible Qwen"}}]}"#,
                    r#"data: {"usage":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}}"#,
                    "data: [DONE]",
                ],
            },
        )
        .unwrap();
        assert_eq!(result.provider, "qwen");
        assert_eq!(result.visible_content_preview, "Visible Qwen");
        assert!(result.hidden_reasoning_persisted);
        assert_eq!(result.reasoning_tokens, 0);
        let blocked = record_native_provider_stream(
            &mut ready_session(),
            &store,
            NativeProviderStreamInput {
                provider: NativeProviderStreamKind::Qwen,
                call_id: "call_2",
                stream_id: "stream_2",
                role: ModelRole::Executor,
                plan: &plan,
                request_preview: "request",
                transcript_id: "transcript_2",
                live: false,
                record_content_deltas: true,
                lines: &[
                    r#"data: {"model":"Qwen/Qwen2-7B","choices":[{"delta":{"content":"No"}}]}"#,
                ],
            },
        );
        assert!(blocked.is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preserves_visible_text_and_tool_call_in_same_deepseek_stream() {
        let (root, store) = temp_store("deepseek-text-plus-tool-response-adapter");
        let mut session = ready_session();
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
                task_summary: "inspect project".to_string(),
                requires_tools: true,
                context_tokens_estimate: 2_000,
            })
            .unwrap();
        let result = record_native_provider_stream(
            &mut session,
            &store,
            NativeProviderStreamInput {
                provider: NativeProviderStreamKind::DeepSeek,
                call_id: "call_text_tool",
                stream_id: "stream_text_tool",
                role: ModelRole::Executor,
                plan: &plan,
                request_preview: "request",
                transcript_id: "transcript_text_tool",
                live: false,
                record_content_deltas: true,
                lines: &[
                    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"我先看看关键文件。"}}"#,
                    r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"file_read","input":{"path":"README.md","max_bytes":2000}}}"#,
                    r#"data: {"type":"message_delta","usage":{"input_tokens":100,"output_tokens":20}}"#,
                    r#"data: {"type":"message_stop"}"#,
                ],
            },
        )
        .unwrap();
        assert!(result.visible_content_preview.contains("我先看看关键文件"));
        assert!(result.visible_content_preview.contains("\"tool_calls\""));
        assert!(result
            .visible_content_preview
            .contains("\"name\":\"file_read\""));
        assert!(result
            .visible_content_preview
            .contains("\"path\":\"README.md\""));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn deepseek_content_deltas_filter_dsml_across_chunks() {
        let (root, store) = temp_store("deepseek-cross-chunk-dsml-filter");
        let mut session = ready_session();
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
                task_summary: "answer".to_string(),
                requires_tools: false,
                context_tokens_estimate: 2_000,
            })
            .unwrap();
        let _result = record_native_provider_stream(
            &mut session,
            &store,
            NativeProviderStreamInput {
                provider: NativeProviderStreamKind::DeepSeek,
                call_id: "call_cross_chunk_dsml",
                stream_id: "stream_cross_chunk_dsml",
                role: ModelRole::Executor,
                plan: &plan,
                request_preview: "request",
                transcript_id: "transcript_cross_chunk_dsml",
                live: false,
                record_content_deltas: true,
                lines: &[
                    r#"data: {"choices":[{"delta":{"content":"Visible <too"}}]}"#,
                    r#"data: {"choices":[{"delta":{"content":"l_call>{\"name\":\"file.write\",\"arguments\":{\"content\":\"secret\"}"}}]}"#,
                    r#"data: {"choices":[{"delta":{"content":"}</tool_call> done"}}]}"#,
                    "data: [DONE]",
                ],
            },
        )
        .unwrap();
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("Visible"), "{jsonl}");
        assert!(jsonl.contains("done"), "{jsonl}");
        assert!(!jsonl.contains("file.write"));
        assert!(!jsonl.contains("secret"));
        assert!(!jsonl.contains("<tool_call>"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn records_non_stream_response_with_sanitized_contract() {
        let (root, store) = temp_store("native-non-stream-response-adapter");
        let mut session = ready_session();
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
                role: ModelRole::Reviewer,
                task_summary: "review".to_string(),
                requires_tools: false,
                context_tokens_estimate: 2_000,
            })
            .unwrap();
        let result = record_native_provider_response(
            &mut session,
            &store,
            NativeProviderResponseInput {
                provider: NativeProviderStreamKind::DeepSeek,
                call_id: "call_response_1",
                stream_id: "stream_response_1",
                role: ModelRole::Reviewer,
                plan: &plan,
                request_preview: "request mentions .env sk-requestsecret",
                transcript_id: "transcript_response_1",
                live: false,
                visible_content: "Visible response with sk-responsesecret and .env",
                hidden_reasoning_sanitized: Some("Need [REDACTED_SECRET] from [REDACTED_PATH]"),
                prompt_tokens: 120,
                completion_tokens: 30,
                reasoning_tokens: 9,
                prompt_cache_hit_tokens: 90,
                prompt_cache_miss_tokens: 30,
            },
        )
        .unwrap();
        assert_eq!(result.provider, "deepseek");
        assert!(result.hidden_reasoning_persisted);
        assert_eq!(result.prompt_tokens, 120);
        assert_eq!(result.reasoning_tokens, 9);
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"model.call_started\""));
        assert!(jsonl.contains("\"event_type\":\"model.stream_delta\""));
        assert!(jsonl.contains("\"event_type\":\"model.stream_completed\""));
        assert!(jsonl.contains("\"event_type\":\"model.call_completed\""));
        assert!(!jsonl.contains("sk-requestsecret"));
        assert!(!jsonl.contains("sk-responsesecret"));
        assert!(!jsonl.contains(".env"));
        let artifact_hash = result.content_hash.clone();
        let artifact = store
            .read_bytes(&ArtifactRecord {
                artifact_id: result.artifact_id,
                kind: crate::artifact::ArtifactKind::ModelTranscript,
                content_hash: artifact_hash.clone(),
                size_bytes: 0,
                privacy_class: "internal".to_string(),
                relative_path: std::path::PathBuf::from("sha256")
                    .join(artifact_hash.get(0..2).unwrap_or("00"))
                    .join(&artifact_hash),
            })
            .unwrap();
        let artifact_text = String::from_utf8(artifact).unwrap();
        assert!(!artifact_text.contains("sk-requestsecret"));
        assert!(!artifact_text.contains("sk-responsesecret"));
        assert!(!artifact_text.contains(".env"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn records_response_after_started_without_duplicate_call_start() {
        let (root, store) = temp_store("native-response-after-started-adapter");
        let mut session = ready_session();
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
                task_summary: "plan".to_string(),
                requires_tools: false,
                context_tokens_estimate: 2_000,
            })
            .unwrap();
        session
            .record_model_call_started(
                "call_started_once",
                "deepseek",
                "deepseek-v4-native",
                "deepseek-v4-flash",
                "planner",
                true,
            )
            .unwrap();
        let result = record_native_provider_response_after_started(
            &mut session,
            &store,
            NativeProviderResponseInput {
                provider: NativeProviderStreamKind::DeepSeek,
                call_id: "call_started_once",
                stream_id: "stream_after_started",
                role: ModelRole::Planner,
                plan: &plan,
                request_preview: "request",
                transcript_id: "transcript_after_started",
                live: true,
                visible_content: "Visible",
                hidden_reasoning_sanitized: None,
                prompt_tokens: 1,
                completion_tokens: 1,
                reasoning_tokens: 0,
                prompt_cache_hit_tokens: 0,
                prompt_cache_miss_tokens: 0,
            },
        )
        .unwrap();
        assert_eq!(result.call_id, "call_started_once");
        let jsonl = session.export_events_jsonl();
        assert_eq!(
            jsonl
                .matches("\"event_type\":\"model.call_started\"")
                .count(),
            1
        );
        assert_eq!(
            jsonl
                .matches("\"event_type\":\"model.call_completed\"")
                .count(),
            1
        );
        let _ = fs::remove_dir_all(root);
    }

    fn ready_session() -> AgentSession {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session
    }

    fn temp_store(prefix: &str) -> (std::path::PathBuf, ArtifactStore) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-{prefix}-{nonce}"));
        (root.clone(), ArtifactStore::new(root.join("artifacts")))
    }
}
