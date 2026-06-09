#![allow(unused_imports)]
// === native_agent_loop family imports (per docs/architecture/native_agent_loop_module_api.md §5) ===
use crate::agent_kernel::permission_gate::{classify_command_with_reasons, CommandDecision};
use crate::agent_kernel::permission_gate::{
    DefaultTool, FileEditTool, FileWriteTool, PatchApplyTool, ShellCommandTool,
};
use crate::agent_kernel::{
    conversation_messages_from_tool_result_continuation, requested_line_count_policy,
    tool_inventory_gated_attempt_count, tool_inventory_observation_count,
    validate_file_write_line_count, AgentKernel, ContinuationStrategy, ContinuationView,
    ConversationMessage, ConversationToolCall, EvidenceClass, EvidenceLedger, IterationOutcome,
    LoopStopReason, NativeLoopIterationContext, ObservationCache, PermissionGate, PermissionMode,
    PostToolBatchAction, ToolBatchGuardAction, ToolInventoryRecord, ToolIterationControlAction,
    ToolIterationControlInput, TurnBudget, TurnController, TurnState,
};
use crate::artifact::ArtifactStore;
use crate::compaction::CompactionSummary;
use crate::context_budget::{
    allocate_native_context_budget_for_turn, ContextBudget, DEEPSEEK_REASONING_REPLAY_BUDGET_TOKENS,
};
use crate::error_recovery::ErrorRecoveryState;
use crate::event_log::EventLog;
use crate::hook_dispatcher::HookDispatcher;
use crate::live_http_transport::{
    LiveHttpResponse, LiveHttpStreamEvent, LiveHttpTransport, ScriptedLiveHttpTransport,
};
use crate::live_model_executor::{
    prepare_live_model_execution, record_live_model_stream_response, LiveModelExecutionRequest,
    LiveModelStreamRecordRequest,
};
use crate::live_model_request::{
    apply_role_sampling_to_prepared_request,
    build_deepseek_anthropic_multi_tool_result_request_with_thinking,
    build_deepseek_anthropic_request_with_tools,
    build_deepseek_openai_multi_tool_result_request_with_reasoning,
    build_deepseek_openai_request_with_tools, build_qwen_openai_multi_tool_result_request,
    build_qwen_openai_request_with_tools, DeepSeekAnthropicToolResultBlock,
    DeepSeekAnthropicToolUseBlock, DeepSeekOpenAiToolCallBlock, DeepSeekOpenAiToolResultBlock,
    ModelRequestMessage, PreparedModelHttpRequest, QwenOpenAiToolCallBlock,
    QwenOpenAiToolResultBlock,
};
use crate::model_adapter::{
    DeepSeekNativeAdapter, ModelAdapter, ModelAdapterRequest, ModelRole, PlannedModelCall,
    QwenNativeAdapter,
};
use crate::native_profile::deepseek::adaptation::{
    DeepSeekAdaptationManager, DualProtocolFallback, ProtocolFormat,
};
use crate::native_profile::deepseek::reasoning::ReasoningReplayManager;
use crate::native_profile::deepseek::stream_processor::StreamProcessor;
use crate::native_provider::NativeProviderEndpoint;
use crate::native_turn_controller::{
    estimate_tokens, NativeContextGuardAction, NativeContextGuardReport, NativeTurnController,
};
use crate::patch::{
    stable_text_hash, validate_patch_allowing_protected, PatchCheck, PatchValidation,
};
use crate::permission_policy::{
    PermissionCheck, PermissionRequest, PermissionResolution, PermissionRuleSet,
    PermissionRuleStore,
};
use crate::session::AgentSession;
use crate::state::AgentState;
use crate::tcml::{
    build_tool_manifest_for_context, mediate_tool_call, mediate_tool_call_with_provider_id,
    model_error_to_tool_result, tool_manifest_generated_payload_json, ModelReadableToolError,
    ToolManifestBuildContext, ToolManifestExposure,
};
use crate::tcml::{
    extract_json_bool, extract_json_string, extract_json_value, normalize_tool_id,
    visible_text_without_tool_calls, CompletedStreamingToolCall, ParsedToolArguments,
    ParsedToolCall, PipelineOutcome, ToolCallSyntax,
};
use crate::tool_execution::{
    execute_tool, ToolExecutionArgs, ToolExecutionMode, ToolExecutionRequest,
};
use crate::tool_result::{write_tool_result_artifact, ToolResultRecord};
use researchcode_kernel::hooks::{HookDecision, HookEvent};
use researchcode_kernel::model::{NativeModelFamily, NativeModelProfile, OptimizationLevel};
use researchcode_kernel::tool::{find_tool_spec, provider_tool_name_for_id};
use researchcode_kernel::{PermissionDecisionKind, PermissionRequestType};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

// Layer B: turn completion and structured-stop helpers.

use crate::native_agent_loop::native_agent_loop_util::{
    json_optional_string, json_string, native_loop_provider_label,
};

use researchcode_kernel::Actor;
use std::env;

pub(in crate::native_agent_loop) fn emit_new_session_events(
    session: &AgentSession,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
) {
    let Some(sink) = event_sink.as_deref_mut() else {
        return;
    };
    let jsonl = session.export_events_jsonl();
    let lines = jsonl.lines().collect::<Vec<_>>();
    for line in lines.iter().skip(*emitted_event_count) {
        sink(line);
    }
    *emitted_event_count = lines.len();
}

#[cfg(test)]
pub(in crate::native_agent_loop) fn record_live_visible_stream_event(
    event: LiveHttpStreamEvent,
    _session: &mut AgentSession,
    _stream_id: &str,
    _provider: &str,
    _emitted_event_count: &mut usize,
    _event_sink: &mut Option<&mut dyn FnMut(&str)>,
    stream_processor: &mut StreamProcessor,
) -> Result<bool, String> {
    let _ = stream_processor.ingest(event);
    Ok(false)
}

pub(in crate::native_agent_loop) fn flush_live_content_stream_event(
    session: &mut AgentSession,
    stream_id: &str,
    provider: &str,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
    pending_content: &mut String,
    pending_content_chunks: &mut usize,
) -> Result<(), String> {
    if pending_content.is_empty() {
        return Ok(());
    }
    let content = std::mem::take(pending_content);
    let chunk_count = *pending_content_chunks;
    *pending_content_chunks = 0;
    session
        .record_model_stream_delta(stream_id, provider, "content", &content)
        .map_err(|error| format!("{error:?}"))?;
    if chunk_count > 1 {
        session
            .record_runtime_event(
                "runtime.stream.coalesced",
                researchcode_kernel::Actor::Runtime,
                format!(
                    "{{\"stream_id\":{},\"provider\":{},\"delta_kind\":\"content\",\"chunks\":{},\"chars\":{}}}",
                    json_string(stream_id),
                    json_string(provider),
                    chunk_count,
                    content.chars().count()
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
    }
    emit_new_session_events(session, emitted_event_count, event_sink);
    Ok(())
}

pub(in crate::native_agent_loop) fn record_live_content_suppressed_event(
    session: &mut AgentSession,
    stream_id: &str,
    provider: &str,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
    reason: &str,
    chars: usize,
    chunks: usize,
) -> Result<(), String> {
    if chars == 0 && chunks == 0 {
        return Ok(());
    }
    session
        .record_runtime_event(
            "runtime.stream.preamble_suppressed",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"stream_id\":{},\"provider\":{},\"reason\":{},\"chars\":{},\"chunks\":{}}}",
                json_string(stream_id),
                json_string(provider),
                json_string(reason),
                chars,
                chunks
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    emit_new_session_events(session, emitted_event_count, event_sink);
    Ok(())
}

fn compact_stream_narration(content: &str, max_chars: usize) -> (String, bool) {
    let char_count = content.chars().count();
    if char_count <= max_chars {
        return (content.to_string(), false);
    }
    let mut compacted = content.chars().take(max_chars).collect::<String>();
    compacted.push_str("\n[stream narration truncated]");
    (compacted, true)
}

pub(in crate::native_agent_loop) fn record_live_stream_narration_event(
    session: &mut AgentSession,
    stream_id: &str,
    provider: &str,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
    reason: &str,
    content: &str,
    chunks: usize,
) -> Result<(), String> {
    let content = content.trim();
    if content.is_empty() {
        return Ok(());
    }
    let (content, truncated) = compact_stream_narration(content, 4_000);
    session
        .record_runtime_event(
            "runtime.stream.narration",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"stream_id\":{},\"provider\":{},\"reason\":{},\"content\":{},\"chars\":{},\"chunks\":{},\"truncated\":{}}}",
                json_string(stream_id),
                json_string(provider),
                json_string(reason),
                json_string(&content),
                content.chars().count(),
                chunks,
                truncated
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    emit_new_session_events(session, emitted_event_count, event_sink);
    Ok(())
}

pub(in crate::native_agent_loop) fn flush_live_thinking_stream_event(
    session: &mut AgentSession,
    stream_id: &str,
    provider: &str,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
    pending_thinking_chars: &mut usize,
) -> Result<(), String> {
    if *pending_thinking_chars == 0 {
        return Ok(());
    }
    let chars = *pending_thinking_chars;
    *pending_thinking_chars = 0;
    session
        .record_model_stream_delta(
            stream_id,
            provider,
            "thinking_sanitized",
            &format!("chars={chars}"),
        )
        .map_err(|error| format!("{error:?}"))?;
    emit_new_session_events(session, emitted_event_count, event_sink);
    Ok(())
}

fn should_trace_http_request_bodies() -> bool {
    matches!(
        env::var("RESEARCHCODE_TRACE_HTTP").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

fn record_native_loop_http_request_body_trace(
    session: &mut AgentSession,
    stream_id: &str,
    provider: &str,
    request: &PreparedModelHttpRequest,
    attempt: u32,
    protocol_format: ProtocolFormat,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
) -> Result<(), String> {
    if !should_trace_http_request_bodies() {
        return Ok(());
    }
    session
        .record_runtime_event(
            "model.call.request_body_recorded",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"stream_id\":{},\"provider\":{},\"attempt\":{},\"protocol_format\":{},\"method\":{},\"url\":{},\"stream\":{},\"body_chars\":{},\"body_hash\":{},\"body_json\":{}}}",
                json_string(stream_id),
                json_string(provider),
                attempt,
                json_string(&format!("{protocol_format:?}")),
                json_string(&request.method),
                json_string(&request.url),
                request.stream,
                request.body_json.chars().count(),
                json_string(&stable_text_hash(&request.body_json)),
                json_string(&request.body_json),
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    emit_new_session_events(session, emitted_event_count, event_sink);
    Ok(())
}

pub(in crate::native_agent_loop) fn send_with_live_visible_stream_events<T: LiveHttpTransport>(
    transport: &T,
    request: &PreparedModelHttpRequest,
    session: &mut AgentSession,
    stream_id: &str,
    family: &NativeModelFamily,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
    mut stream_event_handler: Option<
        &mut dyn FnMut(
            &mut AgentSession,
            &LiveHttpStreamEvent,
            &[CompletedStreamingToolCall],
        ) -> Result<(), String>,
    >,
    mut error_recovery: Option<&mut ErrorRecoveryState>,
    mut dual_protocol: Option<&mut DualProtocolFallback>,
    interrupt: &AtomicBool,
) -> Result<(LiveHttpResponse, bool), String> {
    let provider = native_loop_provider_label(family).to_string();
    let mut transient_attempt = 1u32;
    let mut transient_retry_count = 0u32;

    // Non-streaming path — supports dual-protocol retry.
    if !request.stream || (event_sink.is_none() && stream_event_handler.is_none()) {
        let mut current_request = request.clone();
        let mut current_format = dual_protocol
            .as_ref()
            .map(|dp| dp.current_format)
            .unwrap_or(ProtocolFormat::Anthropic);
        loop {
            record_native_loop_http_request_body_trace(
                session,
                stream_id,
                &provider,
                &current_request,
                transient_attempt,
                current_format,
                emitted_event_count,
                event_sink,
            )?;
            match transport.send(&current_request) {
                Ok(response) => {
                    if (200..300).contains(&response.status_code) {
                        record_native_loop_http_retry_completed(
                            session,
                            stream_id,
                            response.status_code,
                            transient_retry_count,
                            emitted_event_count,
                            event_sink,
                        )?;
                        if let Some(er) = error_recovery.as_deref_mut() {
                            er.on_success();
                        }
                        if let Some(dp) = dual_protocol.as_deref_mut() {
                            dp.on_success(current_format);
                        }
                        return Ok((response, true));
                    }
                    if response.status_code == 400 {
                        if let Some(dp) = dual_protocol.as_deref_mut() {
                            if dp.on_400_error().is_some() {
                                let (new_body, new_url) =
                                    DualProtocolFallback::convert_anthropic_body_to_openai(
                                        &current_request.body_json,
                                        &current_request.url,
                                    )?;
                                current_request.body_json = new_body;
                                current_request.url = new_url;
                                current_format = ProtocolFormat::OpenAI;
                                transient_attempt += 1;
                                continue;
                            }
                        }
                    }
                    if should_retry_transient_status(response.status_code, transient_attempt) {
                        record_native_loop_http_retry_scheduled(
                            session,
                            stream_id,
                            response.status_code,
                            transient_attempt,
                            emitted_event_count,
                            event_sink,
                        )?;
                        if let Some(er) = error_recovery.as_deref_mut() {
                            er.max_tokens.record_retry();
                        }
                        transient_retry_count += 1;
                        transient_attempt += 1;
                        continue;
                    }
                    return Ok((response, true));
                }
                Err(e) => {
                    if let Some(er) = error_recovery.as_deref_mut() {
                        er.max_tokens.escalate();
                    }
                    return Err(e);
                }
            }
        }
    }

    // Streaming path — with dual-protocol retry support.
    //
    // IMPORTANT — dirty-data risk on protocol fallback:
    // When the observer above emits text/tool delta events to the UI via
    // `event_sink` and the downstream HTTP response later returns 400, those
    // already-emitted events cannot be retracted. The retry on the alternate
    // protocol will emit a second set of deltas for the same logical turn.
    //
    // The observer buffers visible/thinking deltas and flushes them only after
    // a 2xx response is confirmed, so a 400-triggered protocol retry cannot
    // leak dirty UI events from the failed attempt.
    let mut current_request = request.clone();
    let mut current_format = dual_protocol
        .as_ref()
        .map(|dp| dp.current_format)
        .unwrap_or(ProtocolFormat::Anthropic);
    loop {
        record_native_loop_http_request_body_trace(
            session,
            stream_id,
            &provider,
            &current_request,
            transient_attempt,
            current_format,
            emitted_event_count,
            event_sink,
        )?;
        let mut observer_error: Option<String> = None;
        let mut live_visible_delta_count = 0usize;
        let mut live_visible_char_count = 0usize;
        let mut live_visible_content = String::new();
        let mut live_preamble_retracted = false;
        let mut live_text_block_content = String::new();
        let mut live_text_block_chunks = 0usize;
        let mut live_stream_status_is_success = false;
        let mut stream_processor = StreamProcessor::default();
        let mut observer = |event: LiveHttpStreamEvent| {
            if observer_error.is_some() {
                return;
            }
            if let LiveHttpStreamEvent::HttpStatus { status_code } = &event {
                live_stream_status_is_success = (200..300).contains(status_code);
                return;
            }
            let is_tool_event = matches!(
                event,
                LiveHttpStreamEvent::ToolCallStarted { .. }
                    | LiveHttpStreamEvent::ToolCallArgumentsDelta { .. }
                    | LiveHttpStreamEvent::ToolCallFinished { .. }
            );
            if let LiveHttpStreamEvent::VisibleTextDelta(delta) = &event {
                live_text_block_content.push_str(delta);
                live_text_block_chunks = live_text_block_chunks.saturating_add(1);
            }
            let processor_output = stream_processor.ingest(event.clone());
            if live_stream_status_is_success {
                if is_tool_event
                    && current_format == ProtocolFormat::Anthropic
                    && live_visible_delta_count > 0
                    && !live_preamble_retracted
                {
                    let narration = if live_text_block_content.trim().is_empty() {
                        &live_visible_content
                    } else {
                        &live_text_block_content
                    };
                    let narration_chunks = if live_text_block_chunks == 0 {
                        live_visible_delta_count
                    } else {
                        live_text_block_chunks
                    };
                    if !narration.trim().is_empty() {
                        if let Err(error) = record_live_stream_narration_event(
                            session,
                            stream_id,
                            &provider,
                            emitted_event_count,
                            event_sink,
                            "anthropic_text_before_tool",
                            narration,
                            narration_chunks,
                        ) {
                            observer_error = Some(error);
                            return;
                        }
                    }
                    live_preamble_retracted = true;
                    live_text_block_content.clear();
                    live_text_block_chunks = 0;
                }
            }
            if live_stream_status_is_success {
                if let Some(handler) = stream_event_handler.as_deref_mut() {
                    if let Err(error) =
                        handler(session, &event, &processor_output.completed_tool_calls)
                    {
                        observer_error = Some(error);
                        return;
                    }
                }
            }
            if live_stream_status_is_success
                && is_tool_event
                && live_visible_delta_count > 0
                && !live_preamble_retracted
            {
                if current_format == ProtocolFormat::Anthropic {
                    // Anthropic content blocks are first-class visible narration.
                    // Keep the already-emitted stream text instead of retracting it.
                } else {
                    if let Err(error) = record_live_stream_narration_event(
                        session,
                        stream_id,
                        &provider,
                        emitted_event_count,
                        event_sink,
                        "tool_call_live_preamble_retracted",
                        &live_visible_content,
                        live_visible_delta_count,
                    ) {
                        observer_error = Some(error);
                        return;
                    }
                    if let Err(error) = record_live_content_suppressed_event(
                        session,
                        stream_id,
                        &provider,
                        emitted_event_count,
                        event_sink,
                        "tool_call_live_preamble_retracted",
                        live_visible_char_count,
                        live_visible_delta_count,
                    ) {
                        observer_error = Some(error);
                        return;
                    }
                }
                live_preamble_retracted = true;
            }
            if live_stream_status_is_success && !stream_processor.snapshot().had_tool_call {
                let (mut pending_content, mut pending_content_chunks) =
                    stream_processor.take_pending_content_preserving_raw();
                let pending_chars = pending_content.chars().count();
                let flushed_pending_content = !pending_content.is_empty();
                let pending_content_for_narration = pending_content.clone();
                if let Err(error) = flush_live_content_stream_event(
                    session,
                    stream_id,
                    &provider,
                    emitted_event_count,
                    event_sink,
                    &mut pending_content,
                    &mut pending_content_chunks,
                ) {
                    observer_error = Some(error);
                    return;
                }
                if flushed_pending_content {
                    live_visible_content.push_str(&pending_content_for_narration);
                    live_visible_delta_count += 1;
                    live_visible_char_count = live_visible_char_count.saturating_add(pending_chars);
                }
            }
        };
        let response =
            match transport.send_with_stream_observer(&current_request, &mut observer, interrupt) {
                Ok(resp) => resp,
                Err(e) => {
                    if let Some(er) = error_recovery.as_deref_mut() {
                        er.max_tokens.escalate();
                    }
                    return Err(e);
                }
            };
        if let Some(error) = observer_error {
            if let Some(er) = error_recovery.as_deref_mut() {
                er.max_tokens.escalate();
            }
            return Err(error);
        }
        if (200..300).contains(&response.status_code) {
            record_stream_finish_reason_telemetry(
                session,
                stream_id,
                &provider,
                current_format,
                transient_attempt,
                dual_protocol
                    .as_ref()
                    .map(|dp| dp.fallback_count)
                    .unwrap_or_default(),
                response.status_code,
                &response.body,
                emitted_event_count,
                event_sink,
            )?;
            let content_candidates = stream_processor.complete_stream();
            if !content_candidates.completed_tool_calls.is_empty() {
                let provider_stop_reason = content_candidates
                    .stop_reason
                    .clone()
                    .or_else(|| stream_finish_reason_from_body(&response.body));
                session
                    .record_runtime_event(
                        "deepseek.tool_call.incomplete_flushed",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"stream_id\":{},\"count\":{},\"reason\":\"stream_completed_before_tool_arguments_complete\",\"provider_stop_reason\":{}}}",
                            json_string(stream_id),
                            content_candidates.completed_tool_calls.len(),
                            provider_stop_reason
                                .as_deref()
                                .map(json_string)
                                .unwrap_or_else(|| "null".to_string())
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                if let Some(handler) = stream_event_handler.as_deref_mut() {
                    handler(
                        session,
                        &LiveHttpStreamEvent::ToolCallFinished { index: None },
                        &content_candidates.completed_tool_calls,
                    )?;
                }
                emit_new_session_events(session, emitted_event_count, event_sink);
            }
            for candidate in &content_candidates.content_tool_call_candidates {
                session
                    .record_runtime_event(
                        "agent.content_tool_call.detected",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"stream_id\":{},\"tool_id\":{},\"confidence\":{},\"source_start\":{},\"source_end\":{},\"action\":\"candidate_only_not_auto_executed\"}}",
                            json_string(stream_id),
                            json_string(&candidate.call.tool_id),
                            candidate.confidence,
                            candidate.source_span.0,
                            candidate.source_span.1
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
            }
            if !content_candidates.content_tool_call_candidates.is_empty() {
                emit_new_session_events(session, emitted_event_count, event_sink);
            }
            if stream_processor.snapshot().had_tool_call {
                let (mut pending_content, pending_content_chunks) =
                    stream_processor.take_pending_content();
                let suppression = stream_processor.take_suppression_counters();
                record_live_content_suppressed_event(
                    session,
                    stream_id,
                    &provider,
                    emitted_event_count,
                    event_sink,
                    "tool_call_stream_completion",
                    pending_content.chars().count(),
                    pending_content_chunks,
                )?;
                pending_content.clear();
                record_live_stream_narration_event(
                    session,
                    stream_id,
                    &provider,
                    emitted_event_count,
                    event_sink,
                    "tool_call_stream_preamble",
                    &suppression.suppressed_preamble_content,
                    suppression.suppressed_preamble_content_chunks,
                )?;
                record_live_content_suppressed_event(
                    session,
                    stream_id,
                    &provider,
                    emitted_event_count,
                    event_sink,
                    "tool_call_stream_preamble",
                    suppression.suppressed_preamble_content_chars,
                    suppression.suppressed_preamble_content_chunks,
                )?;
                record_live_stream_narration_event(
                    session,
                    stream_id,
                    &provider,
                    emitted_event_count,
                    event_sink,
                    "post_tool_visible_delta",
                    &suppression.suppressed_post_tool_content,
                    suppression.suppressed_post_tool_content_chunks,
                )?;
                record_live_content_suppressed_event(
                    session,
                    stream_id,
                    &provider,
                    emitted_event_count,
                    event_sink,
                    "post_tool_visible_delta",
                    suppression.suppressed_post_tool_content_chars,
                    suppression.suppressed_post_tool_content_chunks,
                )?;
            } else {
                let (mut pending_content, mut pending_content_chunks) =
                    stream_processor.take_pending_content();
                let flushed_pending_content = !pending_content.is_empty();
                flush_live_content_stream_event(
                    session,
                    stream_id,
                    &provider,
                    emitted_event_count,
                    event_sink,
                    &mut pending_content,
                    &mut pending_content_chunks,
                )?;
                if flushed_pending_content {
                    live_visible_delta_count += 1;
                }
            }
            let mut pending_thinking_chars = stream_processor.take_pending_thinking_chars();
            flush_live_thinking_stream_event(
                session,
                stream_id,
                &provider,
                emitted_event_count,
                event_sink,
                &mut pending_thinking_chars,
            )?;
        }
        // Dual-protocol fallback on 400
        if response.status_code == 400 {
            if let Some(dp) = dual_protocol.as_deref_mut() {
                if dp.on_400_error().is_some() {
                    let (new_body, new_url) =
                        DualProtocolFallback::convert_anthropic_body_to_openai(
                            &current_request.body_json,
                            &current_request.url,
                        )?;
                    current_request.body_json = new_body;
                    current_request.url = new_url;
                    current_format = ProtocolFormat::OpenAI;
                    transient_attempt += 1;
                    continue;
                }
            }
        }
        let retry_stream_without_dirty_events =
            !stream_processor.snapshot().had_tool_call && live_visible_delta_count == 0;
        if should_retry_transient_status(response.status_code, transient_attempt)
            && retry_stream_without_dirty_events
        {
            record_native_loop_http_retry_scheduled(
                session,
                stream_id,
                response.status_code,
                transient_attempt,
                emitted_event_count,
                event_sink,
            )?;
            if let Some(er) = error_recovery.as_deref_mut() {
                er.max_tokens.record_retry();
            }
            transient_retry_count += 1;
            transient_attempt += 1;
            continue;
        }
        if (200..300).contains(&response.status_code) {
            record_native_loop_http_retry_completed(
                session,
                stream_id,
                response.status_code,
                transient_retry_count,
                emitted_event_count,
                event_sink,
            )?;
            if let Some(er) = error_recovery.as_deref_mut() {
                er.on_success();
            }
            if let Some(dp) = dual_protocol.as_deref_mut() {
                dp.on_success(current_format);
            }
        }
        return Ok((
            response,
            live_visible_delta_count == 0 && !stream_processor.snapshot().had_tool_call,
        ));
    }
}

fn record_stream_finish_reason_telemetry(
    session: &mut AgentSession,
    stream_id: &str,
    provider: &str,
    protocol_format: ProtocolFormat,
    attempt: u32,
    fallback_count: u32,
    status_code: u16,
    body: &str,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
) -> Result<(), String> {
    let Some(reason) = stream_finish_reason_from_body(body) else {
        return Ok(());
    };
    session
        .record_runtime_event(
            "model.stream.finish_reason",
            Actor::Runtime,
            format!(
                "{{\"stream_id\":{},\"provider\":{},\"protocol_format\":{},\"attempt\":{},\"fallback_count\":{},\"status_code\":{},\"finish_reason\":{}}}",
                json_string(stream_id),
                json_string(provider),
                json_string(protocol_format_label(protocol_format)),
                attempt,
                fallback_count,
                status_code,
                json_string(&reason)
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    emit_new_session_events(session, emitted_event_count, event_sink);
    Ok(())
}

fn protocol_format_label(format: ProtocolFormat) -> &'static str {
    match format {
        ProtocolFormat::Anthropic => "anthropic",
        ProtocolFormat::OpenAI => "openai",
    }
}

fn stream_finish_reason_from_body(body: &str) -> Option<String> {
    for line in body.lines() {
        let payload = line
            .trim()
            .strip_prefix("data:")
            .map(str::trim)
            .unwrap_or_else(|| line.trim());
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
            continue;
        };
        if let Some(reason) = find_json_string_key(&value, "finish_reason")
            .or_else(|| find_json_string_key(&value, "stop_reason"))
            .filter(|value| !value.trim().is_empty())
        {
            return Some(reason.to_string());
        }
    }
    None
}

fn find_json_string_key<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(found) = object.get(key).and_then(|value| value.as_str()) {
                return Some(found);
            }
            object
                .values()
                .find_map(|value| find_json_string_key(value, key))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|value| find_json_string_key(value, key)),
        _ => None,
    }
}

const NATIVE_LOOP_MAX_TRANSIENT_HTTP_ATTEMPTS: u32 = 6;

fn should_retry_transient_status(status_code: u16, attempt: u32) -> bool {
    matches!(status_code, 408 | 409 | 429 | 500 | 502 | 503 | 504)
        && attempt < NATIVE_LOOP_MAX_TRANSIENT_HTTP_ATTEMPTS
}

fn native_loop_retry_delay_ms(attempt: u32, stream_id: &str) -> u64 {
    let base = 100u64.saturating_mul(1u64 << attempt.saturating_sub(1).min(5));
    let jitter = stream_id.bytes().fold(attempt as u64, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(byte as u64)
    }) % 37;
    base + jitter
}

fn record_native_loop_http_retry_scheduled(
    session: &mut AgentSession,
    stream_id: &str,
    status_code: u16,
    attempt: u32,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
) -> Result<(), String> {
    session
        .record_runtime_event(
            "model.http_retry_scheduled",
            Actor::Runtime,
            format!(
                "{{\"stream_id\":{},\"attempt\":{},\"next_attempt\":{},\"status_code\":{},\"delay_ms\":{},\"strategy\":\"transient_http_retry\"}}",
                json_string(stream_id),
                attempt,
                attempt + 1,
                status_code,
                native_loop_retry_delay_ms(attempt, stream_id)
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    emit_new_session_events(session, emitted_event_count, event_sink);
    Ok(())
}

fn record_native_loop_http_retry_completed(
    session: &mut AgentSession,
    stream_id: &str,
    status_code: u16,
    retry_count: u32,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
) -> Result<(), String> {
    if retry_count == 0 {
        return Ok(());
    }
    session
        .record_runtime_event(
            "agent.recovery.completed",
            Actor::Runtime,
            format!(
                "{{\"stream_id\":{},\"strategy\":\"transient_http_retry\",\"retries\":{},\"final_status_code\":{}}}",
                json_string(stream_id),
                retry_count,
                status_code
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    emit_new_session_events(session, emitted_event_count, event_sink);
    Ok(())
}

pub(in crate::native_agent_loop) fn guard_native_loop_prepared_request(
    session: &mut AgentSession,
    context_manager: &crate::agent_kernel::ContextManager,
    family: &NativeModelFamily,
    context_budget: &ContextBudget,
    call_id: &str,
    stage: &str,
    prepared: &PreparedModelHttpRequest,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
) -> Result<bool, String> {
    let report = guard_native_loop_prepared_request_report(
        session,
        context_manager,
        family,
        context_budget,
        call_id,
        stage,
        prepared,
        emitted_event_count,
        event_sink,
    )?;
    Ok(report.should_send())
}

pub(in crate::native_agent_loop) fn guard_native_loop_prepared_request_report(
    session: &mut AgentSession,
    context_manager: &crate::agent_kernel::ContextManager,
    family: &NativeModelFamily,
    context_budget: &ContextBudget,
    call_id: &str,
    stage: &str,
    prepared: &PreparedModelHttpRequest,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
) -> Result<NativeContextGuardReport, String> {
    let report = context_manager.guard_prepared_request(
        session,
        family,
        context_budget,
        call_id,
        stage,
        prepared,
    )?;
    emit_new_session_events(session, emitted_event_count, event_sink);
    if report.should_send() {
        return Ok(report);
    }
    let blocked_reason = match report.action {
        NativeContextGuardAction::CompactionRequired => "context_compaction_required",
        NativeContextGuardAction::Blocked => "context_budget_exceeded",
        NativeContextGuardAction::Send => "unknown",
    };
    session
        .record_model_call_blocked(call_id, native_loop_provider_label(family), blocked_reason)
        .map_err(|error| format!("{error:?}"))?;
    emit_new_session_events(session, emitted_event_count, event_sink);
    Ok(report)
}

pub(in crate::native_agent_loop) fn record_native_loop_model_call_started_for_prepared_request(
    session: &mut AgentSession,
    call_id: &str,
    endpoint: &NativeProviderEndpoint,
    plan: &PlannedModelCall,
    context_budget: &ContextBudget,
    prepared: &PreparedModelHttpRequest,
    tools_json: &str,
) -> Result<(), String> {
    let prompt_tokens_estimate = estimate_tokens(&prepared.body_json);
    let protected_reserve_tokens = context_budget.protected_reserve_tokens();
    let budget_warning_count = u64::from(
        prompt_tokens_estimate
            > context_budget
                .max_context_tokens
                .saturating_sub(protected_reserve_tokens),
    );
    let tool_catalog_hash = if tools_json.trim().is_empty() {
        "none".to_string()
    } else {
        stable_text_hash(tools_json)
    };
    session
        .record_model_call_started_with_metadata(
            call_id,
            native_loop_provider_label(&endpoint.family),
            &plan.adapter_id,
            plan.role_model_name
                .as_deref()
                .unwrap_or(&plan.actual_model_name),
            "executor",
            true,
            format!("{:?}", context_budget.scaffold_level),
            prompt_tokens_estimate,
            stable_text_hash(&prepared.body_json),
            tool_catalog_hash,
            context_budget.max_context_tokens,
            context_budget.prompt_scaffold_tokens(),
            context_budget.dynamic_context_tokens(),
            protected_reserve_tokens,
            budget_warning_count,
        )
        .map_err(|error| format!("{error:?}"))?;
    record_native_loop_role_call_event(session, call_id, endpoint, plan, "executor", "continuation")
}

pub(in crate::native_agent_loop) fn record_native_loop_role_call_event(
    session: &mut AgentSession,
    call_id: &str,
    endpoint: &NativeProviderEndpoint,
    plan: &PlannedModelCall,
    role: &str,
    stage: &str,
) -> Result<(), String> {
    session
        .record_runtime_event(
            &format!("agent.{role}.role_call"),
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"call_id\":{},\"role\":{},\"stage\":{},\"model_family\":{},\"adapter_id\":{},\"actual_model_name\":{},\"role_model_name\":{},\"temperature_milli\":{},\"thinking_mode\":{},\"native_tool_calls\":{}}}",
                json_string(call_id),
                json_string(role),
                json_string(stage),
                json_string(native_loop_provider_label(&endpoint.family)),
                json_string(&plan.adapter_id),
                json_string(&plan.actual_model_name),
                json_optional_string(plan.role_model_name.as_deref()),
                plan.temperature_milli
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "null".to_string()),
                json_string(&format!("{:?}", plan.thinking_mode)),
                plan.native_tool_calls
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    if endpoint.family == NativeModelFamily::DeepSeek {
        let flash_call = plan
            .role_model_name
            .as_deref()
            .unwrap_or(&plan.actual_model_name)
            .to_ascii_lowercase()
            .contains("flash");
        session
            .record_runtime_event(
                "deepseek.role_split.flash_savings",
                researchcode_kernel::Actor::Runtime,
                format!(
                    "{{\"call_id\":{},\"role\":{},\"stage\":{},\"actual_model_name\":{},\"flash_call\":{},\"estimated_flash_savings_usd\":{},\"basis\":\"telemetry_only_no_routing_change\"}}",
                    json_string(call_id),
                    json_string(role),
                    json_string(stage),
                    json_string(plan.role_model_name.as_deref().unwrap_or(&plan.actual_model_name)),
                    flash_call,
                    if flash_call { "0.0" } else { "0.0" }
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
    }
    Ok(())
}

#[derive(Debug, Default)]
pub(in crate::native_agent_loop) struct DeepSeekCacheZoneTelemetry {
    zone_a_hash: Option<String>,
    zone_b_hash: Option<String>,
    zone_c_hash: Option<String>,
}

pub(in crate::native_agent_loop) fn record_deepseek_cache_zone_telemetry(
    session: &mut AgentSession,
    telemetry: &mut DeepSeekCacheZoneTelemetry,
    family: &NativeModelFamily,
    call_id: &str,
    iteration: usize,
    stage: &str,
    prepared: &PreparedModelHttpRequest,
) -> Result<(), String> {
    if *family != NativeModelFamily::DeepSeek {
        return Ok(());
    }
    record_deepseek_cache_zone_event(
        session,
        &mut telemetry.zone_a_hash,
        "A",
        extract_cache_zone_hash(&prepared.body_json, "A"),
        call_id,
        iteration,
        stage,
    )?;
    record_deepseek_cache_zone_event(
        session,
        &mut telemetry.zone_b_hash,
        "B",
        extract_cache_zone_hash(&prepared.body_json, "B"),
        call_id,
        iteration,
        stage,
    )?;
    record_deepseek_cache_zone_event(
        session,
        &mut telemetry.zone_c_hash,
        "C",
        extract_cache_zone_hash(&prepared.body_json, "C"),
        call_id,
        iteration,
        stage,
    )
}

pub(in crate::native_agent_loop) fn record_deepseek_cache_zone_event(
    session: &mut AgentSession,
    previous_hash: &mut Option<String>,
    zone: &str,
    current_hash: Option<String>,
    call_id: &str,
    iteration: usize,
    stage: &str,
) -> Result<(), String> {
    let Some(current_hash) = current_hash else {
        return Ok(());
    };
    let hit = previous_hash.as_deref() == Some(current_hash.as_str());
    let previous_payload = json_optional_string(previous_hash.as_deref());
    let event_type = format!(
        "deepseek.cache.zone_{}.{}",
        zone.to_ascii_lowercase(),
        if hit { "hit" } else { "miss" }
    );
    session
        .record_runtime_event(
            &event_type,
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"call_id\":{},\"iteration\":{},\"stage\":{},\"zone\":{},\"hash\":{},\"previous_hash\":{},\"source\":\"runtime_prefix_reuse_observer\"}}",
                json_string(call_id),
                iteration,
                json_string(stage),
                json_string(zone),
                json_string(&current_hash),
                previous_payload,
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    *previous_hash = Some(current_hash);
    Ok(())
}

pub(in crate::native_agent_loop) fn extract_cache_zone_hash(
    body_json: &str,
    zone: &str,
) -> Option<String> {
    let normalized = body_json.replace("\\\"", "\"");
    let zone_marker = format!("<cache_zone name=\"{zone}\"");
    let zone_start = normalized.find(&zone_marker)?;
    let zone_text = &normalized[zone_start..];
    let hash_marker = "hash=\"";
    let hash_start = zone_text.find(hash_marker)? + hash_marker.len();
    let hash_text = &zone_text[hash_start..];
    let hash_end = hash_text.find('"')?;
    Some(hash_text[..hash_end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TRACE_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn request_body_trace_is_feature_gated_and_records_body_json() {
        let _guard = TRACE_ENV_LOCK.lock().unwrap();
        env::remove_var("RESEARCHCODE_TRACE_HTTP");

        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        let request = PreparedModelHttpRequest {
            method: "POST".to_string(),
            url: "https://api.deepseek.test/chat/completions".to_string(),
            authorization_env: "DEEPSEEK_API_KEY".to_string(),
            body_json:
                r#"{"model":"deepseek-test","messages":[{"role":"user","content":"approve plan"}]}"#
                    .to_string(),
            stream: true,
        };
        let mut emitted_event_count = session.event_count();
        let mut emitted = Vec::new();
        {
            let mut sink = |line: &str| emitted.push(line.to_string());
            let mut event_sink: Option<&mut dyn FnMut(&str)> = Some(&mut sink);
            record_native_loop_http_request_body_trace(
                &mut session,
                "stream_1",
                "deepseek",
                &request,
                1,
                ProtocolFormat::OpenAI,
                &mut emitted_event_count,
                &mut event_sink,
            )
            .unwrap();
        }
        assert!(emitted.is_empty());

        env::set_var("RESEARCHCODE_TRACE_HTTP", "1");
        {
            let mut sink = |line: &str| emitted.push(line.to_string());
            let mut event_sink: Option<&mut dyn FnMut(&str)> = Some(&mut sink);
            record_native_loop_http_request_body_trace(
                &mut session,
                "stream_1",
                "deepseek",
                &request,
                2,
                ProtocolFormat::OpenAI,
                &mut emitted_event_count,
                &mut event_sink,
            )
            .unwrap();
        }
        env::remove_var("RESEARCHCODE_TRACE_HTTP");

        let jsonl = emitted.join("\n");
        assert!(jsonl.contains("\"event_type\":\"model.call.request_body_recorded\""));
        assert!(jsonl.contains("\"attempt\":2"));
        assert!(jsonl.contains("approve plan"));
        assert!(jsonl.contains("\"body_hash\""));
    }

    #[test]
    fn failed_stream_attempt_does_not_call_structural_event_handler_before_fallback() {
        struct DirtyFallbackTransport {
            attempts: Mutex<usize>,
        }

        impl LiveHttpTransport for DirtyFallbackTransport {
            fn send(
                &self,
                _request: &PreparedModelHttpRequest,
            ) -> Result<LiveHttpResponse, String> {
                Err("non_streaming_send_should_not_be_used".to_string())
            }

            fn send_with_stream_observer(
                &self,
                _request: &PreparedModelHttpRequest,
                observer: &mut dyn FnMut(LiveHttpStreamEvent),
                _interrupt: &AtomicBool,
            ) -> Result<LiveHttpResponse, String> {
                let mut attempts = self.attempts.lock().unwrap();
                *attempts += 1;
                if *attempts == 1 {
                    observer(LiveHttpStreamEvent::HttpStatus { status_code: 400 });
                    observer(LiveHttpStreamEvent::ToolCallStarted {
                        index: Some(0),
                        id: Some("dirty_tool".to_string()),
                        name: "shell.command".to_string(),
                        input_json: None,
                        requires_finished: false,
                    });
                    return Ok(LiveHttpResponse {
                        status_code: 400,
                        body: "bad anthropic attempt".to_string(),
                    });
                }
                observer(LiveHttpStreamEvent::HttpStatus { status_code: 200 });
                observer(LiveHttpStreamEvent::ToolCallStarted {
                    index: Some(0),
                    id: Some("clean_tool".to_string()),
                    name: "file.read".to_string(),
                    input_json: None,
                    requires_finished: false,
                });
                observer(LiveHttpStreamEvent::ToolCallFinished { index: Some(0) });
                Ok(LiveHttpResponse {
                    status_code: 200,
                    body: r#"data: {"choices":[{"finish_reason":"tool_calls","delta":{}}]}
data: [DONE]"#
                        .to_string(),
                })
            }
        }

        let transport = DirtyFallbackTransport {
            attempts: Mutex::new(0),
        };
        let request = PreparedModelHttpRequest {
            method: "POST".to_string(),
            url: "https://api.deepseek.test/anthropic/messages".to_string(),
            authorization_env: "DEEPSEEK_API_KEY".to_string(),
            body_json: r#"{"model":"deepseek-test","system":"sys","messages":[{"role":"user","content":"hi"}],"stream":true}"#.to_string(),
            stream: true,
        };
        let mut session = AgentSession::new("proj", "sess_dirty_attempt", "task").unwrap();
        let mut emitted_event_count = session.event_count();
        let mut event_sink: Option<&mut dyn FnMut(&str)> = None;
        let mut fallback = DualProtocolFallback::new();
        let interrupt = AtomicBool::new(false);
        let mut handler_seen = Vec::new();
        let mut handler = |_session: &mut AgentSession,
                           event: &LiveHttpStreamEvent,
                           _completed: &[CompletedStreamingToolCall]|
         -> Result<(), String> {
            if let LiveHttpStreamEvent::ToolCallStarted { id, .. } = event {
                handler_seen.push(id.clone().unwrap_or_default());
            }
            Ok(())
        };

        let (response, _clean_attempt_without_live_tooling) = send_with_live_visible_stream_events(
            &transport,
            &request,
            &mut session,
            "stream_dirty_attempt",
            &NativeModelFamily::DeepSeek,
            &mut emitted_event_count,
            &mut event_sink,
            Some(&mut handler),
            None,
            Some(&mut fallback),
            &interrupt,
        )
        .unwrap();

        assert_eq!(response.status_code, 200);
        assert_eq!(handler_seen, vec!["clean_tool".to_string()]);
        assert_eq!(*transport.attempts.lock().unwrap(), 2);
        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("\"event_type\":\"model.stream.finish_reason\""));
        assert!(jsonl.contains("\"protocol_format\":\"openai\""));
        assert!(jsonl.contains("\"attempt\":2"));
        assert!(jsonl.contains("\"fallback_count\":1"));
        assert!(jsonl.contains("\"finish_reason\":\"tool_calls\""));
    }

    #[test]
    fn continuation_stream_fallback_discards_failed_attempt_structural_events() {
        struct DirtyContinuationFallbackTransport {
            attempts: Mutex<usize>,
        }

        impl LiveHttpTransport for DirtyContinuationFallbackTransport {
            fn send(
                &self,
                _request: &PreparedModelHttpRequest,
            ) -> Result<LiveHttpResponse, String> {
                Err("non_streaming_send_should_not_be_used".to_string())
            }

            fn send_with_stream_observer(
                &self,
                _request: &PreparedModelHttpRequest,
                observer: &mut dyn FnMut(LiveHttpStreamEvent),
                _interrupt: &AtomicBool,
            ) -> Result<LiveHttpResponse, String> {
                let mut attempts = self.attempts.lock().unwrap();
                *attempts += 1;
                if *attempts == 1 {
                    observer(LiveHttpStreamEvent::HttpStatus { status_code: 400 });
                    observer(LiveHttpStreamEvent::ToolCallStarted {
                        index: Some(0),
                        id: Some("dirty_continuation_tool".to_string()),
                        name: "shell.command".to_string(),
                        input_json: Some("{\"command\":\"echo dirty\"}".to_string()),
                        requires_finished: false,
                    });
                    observer(LiveHttpStreamEvent::ToolCallFinished { index: Some(0) });
                    return Ok(LiveHttpResponse {
                        status_code: 400,
                        body: "bad anthropic continuation attempt".to_string(),
                    });
                }
                observer(LiveHttpStreamEvent::HttpStatus { status_code: 200 });
                observer(LiveHttpStreamEvent::ToolCallStarted {
                    index: Some(0),
                    id: Some("clean_continuation_tool".to_string()),
                    name: "file.read".to_string(),
                    input_json: Some("{\"path\":\"README.md\"}".to_string()),
                    requires_finished: false,
                });
                observer(LiveHttpStreamEvent::ToolCallFinished { index: Some(0) });
                Ok(LiveHttpResponse {
                    status_code: 200,
                    body: r#"data: {"choices":[{"finish_reason":"tool_calls","delta":{}}]}
data: [DONE]"#
                        .to_string(),
                })
            }
        }

        let tools_json =
            r#"[{"name":"file_read","description":"Read","input_schema":{"type":"object"}}]"#;
        let request = build_deepseek_anthropic_multi_tool_result_request_with_thinking(
            &NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
            "system",
            "Read README.md",
            &[DeepSeekAnthropicToolUseBlock {
                id: "toolu_prev".to_string(),
                name: "file_read".to_string(),
                input_json: "{\"path\":\"README.md\"}".to_string(),
            }],
            &[DeepSeekAnthropicToolResultBlock {
                tool_use_id: "toolu_prev".to_string(),
                content: "README content".to_string(),
                is_error: false,
            }],
            256,
            true,
            tools_json,
            Some("thinking before tool"),
            Some("sig_prev"),
        )
        .unwrap();
        assert!(request.body_json.contains("\"type\":\"tool_result\""));
        assert!(!request.body_json.contains("\"reasoning_content\""));

        let transport = DirtyContinuationFallbackTransport {
            attempts: Mutex::new(0),
        };
        let mut session =
            AgentSession::new("proj", "sess_dirty_continuation_attempt", "task").unwrap();
        let mut emitted_event_count = session.event_count();
        let mut event_sink: Option<&mut dyn FnMut(&str)> = None;
        let mut fallback = DualProtocolFallback::new();
        let interrupt = AtomicBool::new(false);
        let mut handler_seen = Vec::new();
        let mut handler = |_session: &mut AgentSession,
                           event: &LiveHttpStreamEvent,
                           _completed: &[CompletedStreamingToolCall]|
         -> Result<(), String> {
            if let LiveHttpStreamEvent::ToolCallStarted { id, .. } = event {
                handler_seen.push(id.clone().unwrap_or_default());
            }
            Ok(())
        };

        let (response, _clean_attempt_without_live_tooling) = send_with_live_visible_stream_events(
            &transport,
            &request,
            &mut session,
            "stream_dirty_continuation_attempt",
            &NativeModelFamily::DeepSeek,
            &mut emitted_event_count,
            &mut event_sink,
            Some(&mut handler),
            None,
            Some(&mut fallback),
            &interrupt,
        )
        .unwrap();

        assert_eq!(response.status_code, 200);
        assert_eq!(handler_seen, vec!["clean_continuation_tool".to_string()]);
        assert_eq!(*transport.attempts.lock().unwrap(), 2);
        assert_eq!(fallback.fallback_count, 1);
        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("\"event_type\":\"model.stream.finish_reason\""));
        assert!(jsonl.contains("\"protocol_format\":\"openai\""));
        assert!(jsonl.contains("\"attempt\":2"));
        assert!(jsonl.contains("\"fallback_count\":1"));
        assert!(jsonl.contains("\"finish_reason\":\"tool_calls\""));
    }
}
