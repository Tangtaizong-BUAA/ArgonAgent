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

use crate::native_agent_loop::native_agent_loop_model_io::{
    emit_new_session_events, guard_native_loop_prepared_request_report,
    record_native_loop_role_call_event, send_with_live_visible_stream_events,
};
use crate::native_agent_loop::native_agent_loop_prompt::{
    compact_tool_evidence_summary, continuation_view_for_batch, is_tool_budget_refusal_text,
    NativeToolBatch,
};
use crate::native_agent_loop::native_agent_loop_util::{
    json_escape, json_string, native_loop_provider_label, safe_json_fragment,
};

pub(in crate::native_agent_loop) fn record_native_loop_turn_summary(
    session: &mut AgentSession,
    user_task: &str,
    tool_batch: &NativeToolBatch,
    completion_status: &str,
) {
    let mut tools_used = tool_batch
        .iter()
        .map(|(_, tool_id, _, _)| tool_id.clone())
        .collect::<Vec<_>>();
    tools_used.dedup();
    let evidence = tool_batch
        .iter()
        .rev()
        .take(4)
        .map(|(_, tool_id, _, result)| {
            format!(
                "{} ok={} {}",
                tool_id,
                result.ok,
                result.preview.chars().take(120).collect::<String>()
            )
        })
        .collect::<Vec<_>>();
    let _ = session.record_runtime_event(
        "agent.turn_summary",
        researchcode_kernel::Actor::Runtime,
        format!(
            "{{\"user_task\":{},\"completion_status\":{},\"tools_used\":{},\"evidence\":{},\"tool_batch_size\":{}}}",
            json_string(user_task),
            json_string(completion_status),
            safe_json_fragment(&format!(
                "[{}]",
                tools_used
                    .iter()
                    .map(|tool| format!("\"{}\"", json_escape(tool)))
                    .collect::<Vec<_>>()
                    .join(",")
            )),
            safe_json_fragment(&format!(
                "[{}]",
                evidence
                    .iter()
                    .map(|item| format!("\"{}\"", json_escape(item)))
                    .collect::<Vec<_>>()
                    .join(",")
            )),
            tool_batch.len()
        ),
    );
    let telemetry = crate::agent_kernel::AgentKernelTelemetry::aggregate_from(session.event_log());
    let _ = session.record_runtime_event(
        "agent.telemetry.turn_summary",
        researchcode_kernel::Actor::Runtime,
        telemetry.to_payload_json(),
    );
}

pub(in crate::native_agent_loop) fn completion_status_from_batch(
    tool_batch: &NativeToolBatch,
) -> &'static str {
    if tool_batch.iter().any(|(_, _, _, result)| !result.ok) {
        "completed_with_recoveries"
    } else {
        "completed"
    }
}

pub(in crate::native_agent_loop) fn stop_native_loop_with_structured_failure(
    session: &mut AgentSession,
    prompt: &str,
    tool_batch: &NativeToolBatch,
    reason: &str,
    category: &str,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
) -> Result<(), String> {
    stop_native_loop_with_structured_status(
        session,
        prompt,
        tool_batch,
        reason,
        category,
        "failed",
        emitted_event_count,
        event_sink,
    )
}

pub(in crate::native_agent_loop) fn stop_native_loop_with_structured_blocked(
    session: &mut AgentSession,
    prompt: &str,
    tool_batch: &NativeToolBatch,
    reason: &str,
    category: &str,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
) -> Result<(), String> {
    stop_native_loop_with_structured_status(
        session,
        prompt,
        tool_batch,
        reason,
        category,
        "blocked",
        emitted_event_count,
        event_sink,
    )
}

fn stop_native_loop_with_structured_status(
    session: &mut AgentSession,
    prompt: &str,
    tool_batch: &NativeToolBatch,
    reason: &str,
    category: &str,
    status: &str,
    emitted_event_count: &mut usize,
    event_sink: &mut Option<&mut dyn FnMut(&str)>,
) -> Result<(), String> {
    let next_action = if status == "blocked" {
        "surface_blocked_stop_and_release_turn"
    } else {
        "surface_failure_and_release_turn"
    };
    session
        .record_runtime_event(
            "agent.loop_stopped",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"status\":{},\"category\":{},\"reason\":{},\"tool_batch_size\":{},\"next_action\":{}}}",
                json_string(status),
                json_string(category),
                json_string(reason),
                tool_batch.len(),
                json_string(next_action)
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    record_native_loop_turn_summary(session, prompt, tool_batch, status);
    let target_state = if status == "blocked" {
        AgentState::WaitingForUser
    } else {
        AgentState::Failed
    };
    session
        .transition_to(target_state)
        .map_err(|error| format!("{error:?}"))?;
    emit_new_session_events(session, emitted_event_count, event_sink);
    Ok(())
}

pub(in crate::native_agent_loop) fn record_visible_assistant_message(
    session: &mut AgentSession,
    content: &str,
    reason: &str,
) -> Result<bool, String> {
    let visible_text = visible_text_without_tool_calls(content);
    let final_text = visible_text.trim();
    if final_text.is_empty() || is_tool_budget_refusal_text(final_text) {
        return Ok(false);
    }
    let block_id = format!("assistant_message:{reason}");
    session
        .record_runtime_event(
            "assistant.block_started",
            researchcode_kernel::Actor::Agent,
            format!(
                "{{\"block_id\":{},\"block_kind\":\"text\",\"reason\":{}}}",
                json_string(&block_id),
                json_string(reason)
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_runtime_event(
            "assistant.text_delta",
            researchcode_kernel::Actor::Agent,
            format!(
                "{{\"block_id\":{},\"text\":{},\"reason\":{},\"runtime_sanitized\":true}}",
                json_string(&block_id),
                json_string(final_text),
                json_string(reason)
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_runtime_event(
            "assistant.block_completed",
            researchcode_kernel::Actor::Agent,
            format!(
                "{{\"block_id\":{},\"block_kind\":\"text\",\"reason\":{}}}",
                json_string(&block_id),
                json_string(reason)
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_runtime_event(
            "assistant.message",
            researchcode_kernel::Actor::Agent,
            format!(
                "{{\"content\":{},\"reason\":{}}}",
                json_string(final_text),
                json_string(reason)
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    Ok(true)
}
