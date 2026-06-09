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
    execute_tool, execute_tool_batch_concurrent, execute_tool_with_permission_gate,
    SiblingAbortController, ToolBatch, ToolExecutionArgs, ToolExecutionMode, ToolExecutionRequest,
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

use crate::native_agent_loop::native_agent_loop_execution::{
    dispatch_post_tool_use_hook, dispatch_pre_tool_use_hook, ensure_executing,
    execute_model_readable_error_collect, permission_args_json, permission_tool_for_id,
    prepare_exact_edit_execution_args, record_permission_decision_recorded,
    record_tool_call_completed_preserving_provider_id,
    record_tool_call_requested_preserving_provider_id,
    record_tool_result_artifact_preserving_provider_id, tool_args, tool_args_json,
    tool_permission_decision, NativePermissionDecisionOutcome,
};
use crate::native_agent_loop::native_agent_loop_prompt::{
    native_loop_tool_execution_error_result, native_prompt_wants_file_generation,
    validate_fast_auto_write_runtime_constraints, NativeToolBatch,
};
use crate::native_agent_loop::native_agent_loop_util::{
    json_escape, json_optional_string, json_optional_usize, json_string, safe_json_fragment,
    suggested_manifest_tool,
};
use crate::native_agent_loop::{
    NativeAgentPermissionDecision, NativeAgentToolExposure, PendingNativeToolExecution,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::native_agent_loop) struct ReplayedToolCompletionState {
    pub(in crate::native_agent_loop) completed: bool,
    pub(in crate::native_agent_loop) has_result: bool,
}

pub(in crate::native_agent_loop) fn replayed_tool_completion_state(
    session: &AgentSession,
    tool_call_id: &str,
) -> ReplayedToolCompletionState {
    let jsonl = session.export_events_jsonl();
    let quoted_tool_call_id = format!("\"tool_call_id\":\"{}\"", json_escape(tool_call_id));
    ReplayedToolCompletionState {
        completed: jsonl.lines().any(|line| {
            line.contains("\"event_type\":\"tool.call_completed\"")
                && line.contains(&quoted_tool_call_id)
        }),
        has_result: jsonl.lines().any(|line| {
            line.contains("\"event_type\":\"tool.result_recorded\"")
                && line.contains(&quoted_tool_call_id)
        }),
    }
}

pub(in crate::native_agent_loop) fn model_provider_tool_call_id(
    parsed_tool: &ParsedToolCall,
    fallback_id: String,
) -> String {
    parsed_tool
        .provider_tool_call_id
        .clone()
        .unwrap_or(fallback_id)
}

#[allow(dead_code)]
pub(in crate::native_agent_loop) fn execute_unsupported_model_tool_collect(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    index: usize,
    tool_id: &str,
    arguments_json: &str,
) -> Result<crate::tool_execution::ToolExecutionResult, String> {
    let tool_call_id = format!("native_loop_v2_tool_{index}");
    let result = unsupported_model_tool_result(&tool_call_id, tool_id, arguments_json);
    session
        .record_runtime_event(
            "tool.unsupported_recovered",
            researchcode_kernel::Actor::Runtime,
            result.detail_json.clone(),
        )
        .map_err(|error| format!("{error:?}"))?;
    let artifact = write_tool_result_artifact(
        artifact_store,
        &format!("native_loop_v2_unsupported_tool_result_{index}"),
        &ToolResultRecord::new(
            &tool_call_id,
            tool_id,
            false,
            result.preview.clone(),
            result.detail_json.clone(),
        ),
    )
    .map_err(|error| error.to_string())?;
    session
        .record_runtime_event(
            "tool.unsupported_result_recorded",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"tool_call_id\":\"{}\",\"tool_id\":\"{}\",\"artifact_id\":\"{}\",\"content_hash\":\"{}\"}}",
                json_escape(&tool_call_id),
                json_escape(tool_id),
                json_escape(&artifact.artifact_id),
                json_escape(&artifact.content_hash)
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    ensure_executing(session)?;
    Ok(result)
}

pub(in crate::native_agent_loop) fn append_stream_mismatch_error_results(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    iteration: usize,
    streamed_tool_batch: &mut NativeToolBatch,
    parsed_tool_calls: &[ParsedToolCall],
) -> Result<usize, String> {
    let streamed_provider_ids = streamed_tool_batch
        .iter()
        .map(|(provider_tool_call_id, _, _, _)| provider_tool_call_id.clone())
        .collect::<BTreeSet<_>>();
    let streamed_signatures = streamed_tool_batch
        .iter()
        .map(|(_, tool_id, args_json, _)| (tool_id.clone(), canonical_json_text(args_json)))
        .collect::<BTreeSet<_>>();
    let mut appended = 0usize;
    for (parsed_index, parsed_tool) in parsed_tool_calls.iter().enumerate() {
        let provider_tool_call_id = model_provider_tool_call_id(
            parsed_tool,
            format!("toolu_v2_{iteration}_parsed_{parsed_index}"),
        );
        let mediated = mediate_tool_call_with_provider_id(
            &parsed_tool.tool_id,
            Some(&provider_tool_call_id),
            &parsed_tool.arguments_json,
        );
        let canonical_args = canonical_json_text(&mediated.arguments_json);
        if streamed_provider_ids.contains(&provider_tool_call_id)
            || streamed_signatures.contains(&(mediated.tool_id.clone(), canonical_args))
        {
            continue;
        }
        let result_tool_id = if find_tool_spec(&mediated.tool_id).is_some() {
            mediated.tool_id.as_str()
        } else {
            parsed_tool.tool_id.as_str()
        };
        let result = execute_model_readable_error_collect(
            session,
            artifact_store,
            iteration.saturating_mul(1000) + 900 + parsed_index,
            Some(&provider_tool_call_id),
            result_tool_id,
            &ModelReadableToolError {
                error_code: "STREAMED_RESULT_MISSING".to_string(),
                tool_name: result_tool_id.to_string(),
                short_message: "The model produced a parsed tool call, but the streaming executor did not produce a matching tool result. Treat this call as failed; do not assume it ran, and do not repeat it unless genuinely necessary with corrected arguments.".to_string(),
                field_errors: Vec::new(),
                retryable: true,
                retry_hint: Some("Do not assume the streamed call ran; resend only if still necessary with corrected arguments.".to_string()),
                retry_example: None,
                counts_against_budget: true,
                suggested_replacement: None,
            },
        )?;
        streamed_tool_batch.push((
            provider_tool_call_id,
            result.tool_id.clone(),
            mediated.arguments_json,
            result,
        ));
        appended = appended.saturating_add(1);
    }
    Ok(appended)
}

pub(in crate::native_agent_loop) fn canonical_json_text(input: &str) -> String {
    serde_json::from_str::<serde_json::Value>(input)
        .map(|value| serde_json::to_string(&value).unwrap_or_else(|_| input.to_string()))
        .unwrap_or_else(|_| input.to_string())
}

pub(in crate::native_agent_loop) fn contains_executable_dsml_markup(text: &str) -> bool {
    let executable_text = text_without_fenced_code(text);
    matches!(
        crate::agent_kernel::TcmlService::default().process_text(&executable_text),
        PipelineOutcome::ParsedCalls(calls)
            if calls
                .iter()
                .any(|call| matches!(call.syntax, ToolCallSyntax::DeepSeekDsml))
    )
}

pub(in crate::native_agent_loop) fn text_without_fenced_code(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            output.push_str(line);
            output.push('\n');
        }
    }
    output
}

pub(in crate::native_agent_loop) fn unsupported_model_tool_result(
    tool_call_id: &str,
    tool_id: &str,
    arguments_json: &str,
) -> crate::tool_execution::ToolExecutionResult {
    let preview =
        format!("unsupported tool id {tool_id}; recover by using the stable tool catalog aliases");
    let trimmed_arguments = arguments_json.trim();
    let arguments = if trimmed_arguments.is_empty() {
        "{}".to_string()
    } else if trimmed_arguments.starts_with('{') || trimmed_arguments.starts_with('[') {
        trimmed_arguments.to_string()
    } else {
        json_string(trimmed_arguments)
    };
    crate::tool_execution::ToolExecutionResult {
        tool_call_id: tool_call_id.to_string(),
        tool_id: tool_id.to_string(),
        ok: false,
        preview,
        detail_json: format!(
            "{{\"ok\":false,\"error_code\":\"unsupported_tool\",\"tool_id\":\"{}\",\"arguments\":{},\"recoverable\":true,\"next_action_hint\":\"Use file.read, file.list_directory, file.list_tree, repo.map, search.ripgrep, git.status, or another listed ResearchCode tool name. Do not invent new tool names.\"}}",
            json_escape(tool_id),
            arguments
        ),
        exit_code: None,
    }
}

/// Execute a batch of concurrent-safe tools in parallel and record results
/// to the session. Returns the tool result batch entries for continuation.
pub(in crate::native_agent_loop) fn record_deepseek_tool_call_assembled_event(
    session: &mut AgentSession,
    iteration: usize,
    stream_index: usize,
    source: &str,
    provider_tool_use_id: &str,
    tool_id: &str,
    argument_bytes: usize,
) -> Result<(), String> {
    session
        .record_runtime_event(
            "deepseek.tool_call.assembled",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"iteration\":{},\"stream_index\":{},\"source\":{},\"provider_tool_use_id\":{},\"tool_id\":{},\"argument_bytes\":{},\"assembled\":true}}",
                iteration,
                stream_index,
                json_string(source),
                json_string(provider_tool_use_id),
                json_string(tool_id),
                argument_bytes,
            ),
        )
        .map_err(|error| format!("{error:?}"))
}

pub(in crate::native_agent_loop) fn record_deepseek_stream_tool_call_partial_event(
    session: &mut AgentSession,
    iteration: usize,
    event: &LiveHttpStreamEvent,
) -> Result<(), String> {
    let payload = match event {
        LiveHttpStreamEvent::ToolCallStarted {
            index,
            id,
            name,
            input_json,
            requires_finished,
        } => Some(format!(
            "{{\"iteration\":{},\"source\":\"stream_tool_call_started\",\"index\":{},\"provider_tool_use_id\":{},\"name_so_far\":{},\"argument_delta_bytes\":{},\"requires_finished\":{},\"partial\":true}}",
            iteration,
            json_optional_usize(*index),
            json_optional_string(id.as_deref()),
            json_string(name),
            input_json.as_ref().map(|value| value.len()).unwrap_or(0),
            requires_finished,
        )),
        LiveHttpStreamEvent::ToolCallArgumentsDelta { index, delta } => Some(format!(
            "{{\"iteration\":{},\"source\":\"stream_tool_call_arguments_delta\",\"index\":{},\"argument_delta_bytes\":{},\"partial\":true}}",
            iteration,
            json_optional_usize(*index),
            delta.len(),
        )),
        LiveHttpStreamEvent::ToolCallFinished { index } => Some(format!(
            "{{\"iteration\":{},\"source\":\"stream_tool_call_finished\",\"index\":{},\"partial\":true}}",
            iteration,
            json_optional_usize(*index),
        )),
        LiveHttpStreamEvent::HttpStatus { .. }
        | LiveHttpStreamEvent::VisibleTextDelta(_)
        | LiveHttpStreamEvent::ThinkingDelta { .. }
        | LiveHttpStreamEvent::ContentBlockStarted { .. }
        | LiveHttpStreamEvent::ContentBlockFinished { .. } => None,
    };
    if let Some(payload) = payload {
        session
            .record_runtime_event(
                "deepseek.tool_call.partial",
                researchcode_kernel::Actor::Runtime,
                payload,
            )
            .map_err(|error| format!("{error:?}"))?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(in crate::native_agent_loop) fn handle_native_stream_tool_event(
    session: &mut AgentSession,
    event: &LiveHttpStreamEvent,
    completed_calls: &[CompletedStreamingToolCall],
    streamed_tool_batch: &mut NativeToolBatch,
    streamed_tool_sequence: &mut usize,
    streamed_pending_tool: &mut Option<PendingNativeToolExecution>,
    artifact_store: &ArtifactStore,
    workspace_root: &Path,
    iteration: usize,
    family: &NativeModelFamily,
    manifest_allowed_tools: &BTreeSet<String>,
    exposure: &NativeAgentToolExposure,
    permission_mode: &PermissionMode,
    provided_permission_decisions: &[NativeAgentPermissionDecision],
    permission_gate: &mut PermissionGate,
    observation_cache: &mut ObservationCache,
    turn_controller: &mut NativeTurnController,
    prompt: &str,
    hook_dispatcher: Option<&HookDispatcher>,
    streamed_suppressed_count: &mut u32,
) -> Result<(), String> {
    if *family == NativeModelFamily::DeepSeek {
        record_deepseek_stream_tool_call_partial_event(session, iteration, event)?;
    }
    for completed in completed_calls.iter().cloned() {
        let stream_index = *streamed_tool_sequence;
        *streamed_tool_sequence = streamed_tool_sequence.saturating_add(1);
        let execution = execute_streamed_native_tool_call_collect(
            session,
            artifact_store,
            workspace_root,
            iteration,
            stream_index,
            family,
            manifest_allowed_tools,
            exposure,
            permission_mode,
            provided_permission_decisions,
            permission_gate,
            streamed_pending_tool,
            observation_cache,
            turn_controller,
            completed,
            prompt,
            hook_dispatcher,
        )?;
        if execution.suppressed_duplicate {
            *streamed_suppressed_count += 1;
        }
        if let Some(record) = execution.record {
            streamed_tool_batch.push(record);
        }
    }
    Ok(())
}

pub(in crate::native_agent_loop) struct StreamedToolExecution {
    record: Option<(
        String,
        String,
        String,
        crate::tool_execution::ToolExecutionResult,
    )>,
    suppressed_duplicate: bool,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::native_agent_loop) fn execute_streamed_native_tool_call_collect(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    workspace_root: &Path,
    iteration: usize,
    stream_index: usize,
    family: &NativeModelFamily,
    manifest_allowed_tools: &BTreeSet<String>,
    exposure: &NativeAgentToolExposure,
    permission_mode: &PermissionMode,
    provided_permission_decisions: &[NativeAgentPermissionDecision],
    permission_gate: &mut PermissionGate,
    streamed_pending_tool: &mut Option<PendingNativeToolExecution>,
    observation_cache: &mut ObservationCache,
    turn_controller: &mut NativeTurnController,
    completed: CompletedStreamingToolCall,
    prompt: &str,
    hook_dispatcher: Option<&HookDispatcher>,
) -> Result<StreamedToolExecution, String> {
    if !is_stream_candidate_provider_tool(&completed.parsed.tool_id, exposure) {
        return Ok(StreamedToolExecution {
            record: None,
            suppressed_duplicate: false,
        });
    }
    if *family == NativeModelFamily::DeepSeek {
        record_deepseek_tool_call_assembled_event(
            session,
            iteration,
            stream_index,
            "streaming_assembler",
            &completed.provider_tool_use_id,
            &completed.parsed.tool_id,
            completed.parsed.arguments_json.len(),
        )?;
    }
    let mediated = mediate_tool_call_with_provider_id(
        &completed.parsed.tool_id,
        Some(&completed.provider_tool_use_id),
        &completed.parsed.arguments_json,
    );
    let tool_id = mediated.tool_id.clone();
    let arguments = mediated.arguments.clone();
    if !is_stream_executable_tool(&tool_id, exposure) {
        return Ok(StreamedToolExecution {
            record: None,
            suppressed_duplicate: false,
        });
    }
    let ledger_tool_call_id = format!("native_loop_v2_stream_ledger_{iteration}_{stream_index}");
    turn_controller.record_tool_pending(session, &ledger_tool_call_id, &tool_id, iteration)?;
    for event in &mediated.events {
        session
            .record_runtime_event(
                &event.event_type,
                researchcode_kernel::Actor::Runtime,
                event.payload_json.clone(),
            )
            .map_err(|error| format!("{error:?}"))?;
    }
    let index = iteration * 100 + stream_index;
    let result = if !manifest_allowed_tools.contains(&tool_id) {
        let error = ModelReadableToolError {
            error_code: "TOOL_NOT_IN_MANIFEST".to_string(),
            tool_name: tool_id.clone(),
            short_message: format!(
                "Tool '{}' is not available in the current manifest/exposure.",
                tool_id
            ),
            field_errors: Vec::new(),
            retryable: true,
            retry_hint: Some("Use a tool_id exposed in the current manifest.".to_string()),
            retry_example: Some(
                r#"{"tool_id":"file.read","arguments":{"path":"README.md"}}"#.to_string(),
            ),
            counts_against_budget: true,
            suggested_replacement: suggested_manifest_tool(manifest_allowed_tools, &tool_id),
        };
        execute_model_readable_error_collect(
            session,
            artifact_store,
            index,
            Some(&completed.provider_tool_use_id),
            &completed.parsed.tool_id,
            &error,
        )?
    } else if let Some(error) = mediated.error.clone() {
        execute_model_readable_error_collect(
            session,
            artifact_store,
            index,
            Some(&completed.provider_tool_use_id),
            &completed.parsed.tool_id,
            &error,
        )?
    } else if matches!(
        tool_id.as_str(),
        "file.write" | "file.edit" | "file.multi_edit"
    ) {
        match execute_permissioned_write_collect(
            session,
            artifact_store,
            permission_gate,
            workspace_root,
            index,
            Some(&completed.provider_tool_use_id),
            &tool_id,
            &arguments,
            prompt,
            Some(format!("native_loop_v2_stream_perm_{index}")),
            permission_mode,
            provided_permission_decisions,
            hook_dispatcher,
        )? {
            PermissionedWriteOutcome::Executed(result) => result,
            PermissionedWriteOutcome::Pending(pending_tool) => {
                *streamed_pending_tool = Some(pending_tool);
                return Ok(StreamedToolExecution {
                    record: None,
                    suppressed_duplicate: false,
                });
            }
        }
    } else if matches!(tool_id.as_str(), "shell.command" | "patch.apply") {
        let tool_call_id = format!("native_loop_v2_stream_tool_{index}");
        record_tool_call_requested_preserving_provider_id(
            session,
            &tool_call_id,
            Some(&completed.provider_tool_use_id),
            &tool_id,
        )?;
        let args = tool_args(&arguments);
        let request_type = if tool_id == "shell.command" {
            PermissionRequestType::Command
        } else {
            PermissionRequestType::FileWrite
        };
        let permission_id = format!("native_loop_v2_stream_perm_{index}");
        let result = if let Some(reason) = dispatch_pre_tool_use_hook(
            hook_dispatcher,
            &tool_id,
            &tool_call_id,
            &tool_args_json(&arguments),
        ) {
            let error = ModelReadableToolError {
                error_code: "PRE_TOOL_USE_DENIED".to_string(),
                tool_name: tool_id.clone(),
                short_message: format!("PreToolUse hook blocked {tool_id}: {reason}"),
                field_errors: Vec::new(),
                retryable: true,
                retry_hint: Some(
                    "Choose a tool call that satisfies the active runtime hook constraints."
                        .to_string(),
                ),
                retry_example: Some(
                    r#"{"tool_id":"file.read","arguments":{"path":"README.md"}}"#.to_string(),
                ),
                counts_against_budget: true,
                suggested_replacement: Some("file.read".to_string()),
            };
            session
                .record_runtime_event(
                    "tool.model_readable_error",
                    researchcode_kernel::Actor::Runtime,
                    format!(
                        "{{\"tool_call_id\":{},\"provider_tool_call_id\":{},\"requested_tool_id\":{},\"result_tool_id\":{},\"error\":{}}}",
                        json_string(&tool_call_id),
                        json_string(&completed.provider_tool_use_id),
                        json_string(&completed.parsed.tool_id),
                        json_string(&tool_id),
                        error.to_payload_json()
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
            model_error_to_tool_result(&tool_call_id, &tool_id, &error)
        } else {
            let permission_outcome = tool_permission_decision(
                session,
                artifact_store,
                permission_gate,
                permission_mode,
                &tool_id,
                &permission_id,
                request_type.clone(),
                &args,
                provided_permission_decisions,
            )?;
            match permission_outcome {
                NativePermissionDecisionOutcome::Allow(decision) => {
                    match prepare_exact_edit_execution_args(workspace_root, &tool_id, &args) {
                        Err(error) => {
                            native_loop_tool_execution_error_result(&tool_call_id, &tool_id, &error)
                        }
                        Ok(executable_args) => execute_tool_with_permission_gate(
                            &ToolExecutionRequest {
                                workspace_root: workspace_root.to_path_buf(),
                                tool_call_id: tool_call_id.clone(),
                                tool_id: tool_id.clone(),
                                mode: ToolExecutionMode::ApplyWithPermission {
                                    permission_decision: Some(decision),
                                },
                                args: executable_args,
                            },
                            permission_gate,
                        )
                        .map_err(|error| format!("{error:?}"))?,
                    }
                }
                NativePermissionDecisionOutcome::Denied(error) => {
                    session
                        .record_runtime_event(
                            "tool.model_readable_error",
                            researchcode_kernel::Actor::Runtime,
                            format!(
                                "{{\"tool_call_id\":{},\"provider_tool_call_id\":{},\"requested_tool_id\":{},\"result_tool_id\":{},\"error\":{}}}",
                                json_string(&tool_call_id),
                                json_string(&completed.provider_tool_use_id),
                                json_string(&completed.parsed.tool_id),
                                json_string(&tool_id),
                                error.to_payload_json()
                            ),
                        )
                        .map_err(|error| format!("{error:?}"))?;
                    model_error_to_tool_result(&tool_call_id, &tool_id, &error)
                }
                NativePermissionDecisionOutcome::Pending => {
                    *streamed_pending_tool = Some(PendingNativeToolExecution {
                        step_index: index,
                        tool_call_id,
                        tool_id: tool_id.clone(),
                        permission_id,
                        request_type,
                        patch_id: if tool_id == "patch.apply" {
                            Some(format!("native_loop_v2_stream_patch_{index}"))
                        } else {
                            None
                        },
                        args,
                    });
                    return Ok(StreamedToolExecution {
                        record: None,
                        suppressed_duplicate: false,
                    });
                }
            }
        };
        record_tool_call_completed_preserving_provider_id(
            session,
            &tool_call_id,
            Some(&completed.provider_tool_use_id),
            &tool_id,
            result.ok,
        )?;
        let artifact = write_tool_result_artifact(
            artifact_store,
            &format!("native_loop_v2_stream_permissioned_result_{index}"),
            &ToolResultRecord::new(
                &tool_call_id,
                &tool_id,
                result.ok,
                result.preview.clone(),
                result.detail_json.clone(),
            ),
        )
        .map_err(|error| error.to_string())?;
        record_tool_result_artifact_preserving_provider_id(
            session,
            &tool_call_id,
            Some(&completed.provider_tool_use_id),
            &tool_id,
            artifact.artifact_id,
            artifact.content_hash,
            result.preview.clone(),
        )?;
        result
    } else if let Some(cache_key) =
        observation_cache.check_and_record_in_workspace(&tool_id, &arguments, workspace_root)
    {
        session
            .record_runtime_event(
                "agent.loop_recovery",
                researchcode_kernel::Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"reason\":\"repeated_tool_batch\",\"status\":\"duplicate_observation_suppression\",\"source\":\"streaming_tool_call\"}}",
                    iteration
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        let result = execute_duplicate_observation_collect(
            session,
            artifact_store,
            index,
            Some(&completed.provider_tool_use_id),
            &tool_id,
            &mediated.arguments_json,
            &cache_key,
        )?;
        turn_controller.record_tool_completed(
            session,
            &ledger_tool_call_id,
            &tool_id,
            result.ok,
        )?;
        session
            .record_runtime_event(
                "agent.tool.streaming_duplicate_suppressed",
                researchcode_kernel::Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"stream_index\":{},\"provider_tool_use_id\":{},\"tool_id\":{},\"cache_key\":{}}}",
                    iteration,
                    stream_index,
                    json_string(&completed.provider_tool_use_id),
                    json_string(&tool_id),
                    json_string(&cache_key)
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        return Ok(StreamedToolExecution {
            record: Some((
                completed.provider_tool_use_id.clone(),
                tool_id,
                mediated.arguments_json,
                result,
            )),
            suppressed_duplicate: true,
        });
    } else {
        execute_read_only_collect(
            session,
            artifact_store,
            workspace_root,
            index,
            Some(&completed.provider_tool_use_id),
            &tool_id,
            &arguments,
            hook_dispatcher,
        )?
    };
    turn_controller.record_tool_completed(session, &ledger_tool_call_id, &tool_id, result.ok)?;
    session
        .record_runtime_event(
            "agent.tool.streaming_completed",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"iteration\":{},\"stream_index\":{},\"provider_tool_use_id\":{},\"tool_id\":{},\"ok\":{},\"preview\":{}}}",
                iteration,
                stream_index,
                json_string(&completed.provider_tool_use_id),
                json_string(&tool_id),
                result.ok,
                json_string(&result.preview)
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    Ok(StreamedToolExecution {
        record: Some((
            completed.provider_tool_use_id,
            tool_id,
            mediated.arguments_json,
            result,
        )),
        suppressed_duplicate: false,
    })
}

pub(in crate::native_agent_loop) fn is_stream_executable_tool(
    tool_id: &str,
    _exposure: &NativeAgentToolExposure,
) -> bool {
    matches!(
        tool_id,
        "file.read"
            | "file.list_directory"
            | "file.list_tree"
            | "search.ripgrep"
            | "repo.map"
            | "git.status"
            | "research.csv_profile"
            | "task.dispatch"
            | "file.write"
            | "file.edit"
            | "file.multi_edit"
    )
}

pub(in crate::native_agent_loop) fn is_stream_candidate_provider_tool(
    tool_id: &str,
    _exposure: &NativeAgentToolExposure,
) -> bool {
    matches!(
        tool_id,
        "file.read"
            | "file_read"
            | "file.list_directory"
            | "file_list_directory"
            | "file.list_tree"
            | "file_list_tree"
            | "search.ripgrep"
            | "search_ripgrep"
            | "repo.map"
            | "repo_map"
            | "git.status"
            | "git_status"
            | "research.csv_profile"
            | "research_csv_profile"
            | "task.dispatch"
            | "task_dispatch"
            | "file.write"
            | "file_write"
            | "file.edit"
            | "file_edit"
            | "file.multi_edit"
            | "file_multi_edit"
    )
}

pub(in crate::native_agent_loop) fn execute_concurrent_read_only_batch(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    workspace_root: &Path,
    batch: &ToolBatch,
    iteration: usize,
    hook_dispatcher: Option<&HookDispatcher>,
) -> Result<
    Vec<(
        String,
        String,
        String,
        crate::tool_execution::ToolExecutionResult,
    )>,
    String,
> {
    let abort = SiblingAbortController::new();
    let results = execute_tool_batch_concurrent(batch, workspace_root.to_path_buf(), &abort);
    let mut tool_batch = Vec::new();
    for (idx, tool_result) in results.iter().enumerate() {
        let tool_call_id = format!("native_loop_v2_conc_tool_{iteration}_{idx}");
        let tool = &batch.tools[idx];
        record_tool_call_requested_preserving_provider_id(
            session,
            &tool_call_id,
            Some(&tool.tool_call_id),
            &tool.tool_id,
        )?;
        record_tool_call_completed_preserving_provider_id(
            session,
            &tool_call_id,
            Some(&tool.tool_call_id),
            &tool.tool_id,
            tool_result.ok,
        )?;
        let record = crate::tool_result::ToolResultRecord::new(
            &tool_result.tool_call_id,
            &tool_result.tool_id,
            tool_result.ok,
            &tool_result.preview,
            &tool_result.detail_json,
        );
        let artifact =
            crate::tool_result::write_tool_result_artifact(artifact_store, &tool_call_id, &record)
                .map_err(|error| format!("{error:?}"))?;
        record_tool_result_artifact_preserving_provider_id(
            session,
            &tool_call_id,
            Some(&tool.tool_call_id),
            &tool.tool_id,
            artifact.artifact_id,
            artifact.content_hash,
            tool_result.preview.clone(),
        )?;
        dispatch_post_tool_use_hook(
            hook_dispatcher,
            &tool.tool_id,
            tool_result.ok,
            &tool_result.preview,
        );
        tool_batch.push((
            tool.tool_call_id.clone(),
            tool.tool_id.clone(),
            tool_execution_args_json(&tool.args),
            tool_result.clone(),
        ));
    }
    Ok(tool_batch)
}

fn tool_execution_args_json(args: &ToolExecutionArgs) -> String {
    let mut object = serde_json::Map::new();
    if let Some(value) = &args.path {
        object.insert("path".to_string(), serde_json::Value::String(value.clone()));
    }
    if let Some(value) = &args.root {
        object.insert("root".to_string(), serde_json::Value::String(value.clone()));
    }
    if let Some(value) = args.include_hidden {
        object.insert("include_hidden".to_string(), serde_json::Value::Bool(value));
    }
    if let Some(value) = &args.pattern {
        object.insert(
            "pattern".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.query {
        object.insert(
            "query".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.content {
        object.insert(
            "content".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = args.max_bytes {
        object.insert("max_bytes".to_string(), serde_json::json!(value));
    }
    if let Some(value) = args.max_results {
        object.insert("max_results".to_string(), serde_json::json!(value));
    }
    if let Some(value) = args.max_files {
        object.insert("max_files".to_string(), serde_json::json!(value));
    }
    if let Some(value) = args.max_depth {
        object.insert("max_depth".to_string(), serde_json::json!(value));
    }
    if let Some(value) = args.offset {
        object.insert("offset".to_string(), serde_json::json!(value));
    }
    if let Some(value) = args.limit {
        object.insert("limit".to_string(), serde_json::json!(value));
    }
    if let Some(value) = &args.command {
        object.insert(
            "command".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.old_string {
        object.insert(
            "old_string".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.new_string {
        object.insert(
            "new_string".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.base_hash {
        object.insert(
            "base_hash".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = args.replace_all {
        object.insert("replace_all".to_string(), serde_json::Value::Bool(value));
    }
    if let Some(value) = &args.edits_json {
        object.insert(
            "edits_json".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.input_csv {
        object.insert(
            "input_csv".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.job_id {
        object.insert(
            "job_id".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.output_dir {
        object.insert(
            "output_dir".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.answer {
        object.insert(
            "answer".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.model_role {
        object.insert(
            "model_role".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.write_scope_json {
        object.insert(
            "write_scope_json".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    serde_json::Value::Object(object).to_string()
}

pub(in crate::native_agent_loop) fn tool_calls_are_cached_observations(
    tool_calls: &[ParsedToolCall],
    cache: &mut ObservationCache,
    _manifest_allowed_tools: &BTreeSet<String>,
    per_iteration_tool_cap: usize,
    workspace_root: &Path,
) -> bool {
    let mut checked = 0usize;
    for parsed_tool in tool_calls.iter().take(per_iteration_tool_cap) {
        checked += 1;
        let mediated = mediate_tool_call(&parsed_tool.tool_id, &parsed_tool.arguments_json);
        if mediated.error.is_some() {
            return false;
        }
        if cache.contains_in_workspace(&mediated.tool_id, &mediated.arguments, workspace_root) {
            continue;
        }
        return false;
    }
    checked > 0
}

pub(in crate::native_agent_loop) fn execute_duplicate_observation_collect(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    index: usize,
    provider_tool_call_id: Option<&str>,
    tool_id: &str,
    arguments_json: &str,
    cache_key: &str,
) -> Result<crate::tool_execution::ToolExecutionResult, String> {
    let tool_call_id = format!("native_loop_v2_tool_{index}");
    let result = crate::tool_execution::ToolExecutionResult {
        tool_call_id: tool_call_id.clone(),
        tool_id: tool_id.to_string(),
        ok: true,
        preview: format!(
            "File unchanged since the earlier {tool_id} call in this conversation. The previous tool_result is still current; refer to it instead of re-reading. If you have all evidence needed, produce the final answer/report now. For code-edit tasks only, proceed to write/edit/patch instead of further reads."
        ),
        detail_json: format!(
            "{{\"ok\":true,\"skipped\":true,\"reason\":\"duplicate_observation\",\"tool_id\":{},\"cache_key\":{},\"arguments\":{},\"next_action_hint\":\"This file/path was already observed in this conversation. Do not call read/list/search tools on covered ranges or listings again. If you have collected enough evidence, produce the final answer/report now. For coding tasks only, switch to write/edit/patch tools. If you genuinely need new evidence, choose a path NOT present in the Evidence Ledger.\"}}",
            json_string(tool_id),
            json_string(cache_key),
            safe_json_fragment(arguments_json)
        ),
        exit_code: None,
    };
    record_tool_call_requested_preserving_provider_id(
        session,
        &tool_call_id,
        provider_tool_call_id,
        tool_id,
    )?;
    record_tool_call_completed_preserving_provider_id(
        session,
        &tool_call_id,
        provider_tool_call_id,
        tool_id,
        true,
    )?;
    let artifact = write_tool_result_artifact(
        artifact_store,
        &format!("native_loop_v2_duplicate_observation_{index}"),
        &ToolResultRecord::new(
            &tool_call_id,
            tool_id,
            true,
            result.preview.clone(),
            result.detail_json.clone(),
        ),
    )
    .map_err(|error| error.to_string())?;
    record_tool_result_artifact_preserving_provider_id(
        session,
        &tool_call_id,
        provider_tool_call_id,
        tool_id,
        artifact.artifact_id,
        artifact.content_hash,
        result.preview.clone(),
    )?;
    session
        .record_runtime_event(
            "tool.duplicate_observation_suppressed",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"tool_call_id\":{},\"tool_id\":{},\"cache_key\":{},\"skipped\":true,\"reason\":\"duplicate_observation\",\"preview\":{}}}",
                json_string(&tool_call_id),
                json_string(tool_id),
                json_string(cache_key),
                json_string(&result.preview)
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    ensure_executing(session)?;
    Ok(result)
}

#[allow(dead_code)]
pub(in crate::native_agent_loop) fn execute_read_only_collect(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    workspace_root: &Path,
    index: usize,
    provider_tool_call_id: Option<&str>,
    tool_id: &str,
    arguments: &ParsedToolArguments,
    hook_dispatcher: Option<&HookDispatcher>,
) -> Result<crate::tool_execution::ToolExecutionResult, String> {
    let tool_call_id = format!("native_loop_v2_tool_{index}");
    record_tool_call_requested_preserving_provider_id(
        session,
        &tool_call_id,
        provider_tool_call_id,
        tool_id,
    )?;
    let args_json = tool_args_json(arguments);
    if let Some(reason) =
        dispatch_pre_tool_use_hook(hook_dispatcher, tool_id, &tool_call_id, &args_json)
    {
        return Err(format!(
            "tool '{tool_id}' blocked by PreToolUse hook: {reason}"
        ));
    }
    let mut result = match execute_tool(&ToolExecutionRequest {
        workspace_root: workspace_root.to_path_buf(),
        tool_call_id: tool_call_id.clone(),
        tool_id: tool_id.to_string(),
        mode: ToolExecutionMode::ReadOnlyPreview,
        args: tool_args(arguments),
    }) {
        Ok(result) => result,
        Err(error) => native_loop_tool_execution_error_result(&tool_call_id, tool_id, &error),
    };
    if tool_id == "file.read"
        && !result.ok
        && extract_json_string(&result.detail_json, "error_code").as_deref()
            == Some("path_is_directory")
    {
        if let Some(path) = extract_json_string(&result.detail_json, "path") {
            let list_recovery_id = format!("{tool_call_id}_auto_list_directory");
            let list_recovery = execute_tool(&ToolExecutionRequest {
                workspace_root: workspace_root.to_path_buf(),
                tool_call_id: list_recovery_id.clone(),
                tool_id: "file.list_directory".to_string(),
                mode: ToolExecutionMode::ReadOnlyPreview,
                args: ToolExecutionArgs {
                    path: Some(path.clone()),
                    root: Some(path.clone()),
                    max_results: Some(200),
                    ..ToolExecutionArgs::default()
                },
            })
            .unwrap_or_else(|error| {
                native_loop_tool_execution_error_result(
                    &list_recovery_id,
                    "file.list_directory",
                    &error,
                )
            });
            let repo_recovery = if list_recovery.ok {
                None
            } else {
                let repo_recovery_id = format!("{tool_call_id}_auto_repo_map");
                Some(
                    execute_tool(&ToolExecutionRequest {
                        workspace_root: workspace_root.to_path_buf(),
                        tool_call_id: repo_recovery_id.clone(),
                        tool_id: "repo.map".to_string(),
                        mode: ToolExecutionMode::ReadOnlyPreview,
                        args: ToolExecutionArgs {
                            root: Some(path.clone()),
                            max_files: Some(160),
                            max_depth: Some(4),
                            ..ToolExecutionArgs::default()
                        },
                    })
                    .unwrap_or_else(|error| {
                        native_loop_tool_execution_error_result(
                            &repo_recovery_id,
                            "repo.map",
                            &error,
                        )
                    }),
                )
            };
            let original_detail = result.detail_json.clone();
            result.preview = format!(
                "{}; auto_recovery file.list_directory ok={} {}",
                result.preview,
                list_recovery.ok,
                list_recovery.preview.chars().take(240).collect::<String>()
            );
            result.detail_json = format!(
                "{{\"ok\":false,\"error_code\":\"path_is_directory\",\"path\":{},\"recoverable\":true,\"suggested_tool\":\"file.list_directory\",\"next_action_hint\":\"Use the embedded auto_list_directory result, then read concrete files only; do not call file.read on this directory again.\",\"original_detail\":{},\"auto_list_directory\":{{\"ok\":{},\"tool_call_id\":{},\"preview\":{},\"detail\":{}}},\"auto_repo_map\":{}}}",
                json_string(&path),
                original_detail,
                list_recovery.ok,
                json_string(&list_recovery.tool_call_id),
                json_string(&list_recovery.preview),
                list_recovery.detail_json,
                if let Some(repo_recovery) = &repo_recovery {
                    format!(
                        "{{\"ok\":{},\"tool_call_id\":{},\"preview\":{},\"detail\":{}}}",
                        repo_recovery.ok,
                        json_string(&repo_recovery.tool_call_id),
                        json_string(&repo_recovery.preview),
                        repo_recovery.detail_json
                    )
                } else {
                    "null".to_string()
                }
            );
            session
                .record_runtime_event(
                    "tool.auto_recovery",
                    researchcode_kernel::Actor::Runtime,
                    format!(
                        "{{\"tool_call_id\":\"{}\",\"failed_tool\":\"file.read\",\"recovery_tool\":\"file.list_directory\",\"path\":{},\"ok\":{}}}",
                        json_escape(&tool_call_id),
                        json_string(&path),
                        list_recovery.ok
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
        }
    }
    record_tool_call_completed_preserving_provider_id(
        session,
        &tool_call_id,
        provider_tool_call_id,
        tool_id,
        result.ok,
    )?;
    let artifact = write_tool_result_artifact(
        artifact_store,
        &format!("native_loop_v2_tool_result_{index}"),
        &ToolResultRecord::new(
            &tool_call_id,
            tool_id,
            result.ok,
            result.preview.clone(),
            result.detail_json.clone(),
        ),
    )
    .map_err(|error| error.to_string())?;
    record_tool_result_artifact_preserving_provider_id(
        session,
        &tool_call_id,
        provider_tool_call_id,
        tool_id,
        artifact.artifact_id,
        artifact.content_hash,
        result.preview.clone(),
    )?;
    ensure_executing(session)?;
    dispatch_post_tool_use_hook(hook_dispatcher, tool_id, result.ok, &result.preview);
    if tool_id == "task.dispatch" {
        record_task_dispatch_subagent_completion(session, arguments, &result)?;
    }
    Ok(result)
}

fn record_task_dispatch_subagent_completion(
    session: &mut AgentSession,
    arguments: &ParsedToolArguments,
    result: &crate::tool_execution::ToolExecutionResult,
) -> Result<(), String> {
    let task_id = extract_json_string(&result.detail_json, "task_id")
        .unwrap_or_else(|| format!("subagent_{}", stable_text_hash(&result.detail_json)));
    let prompt = arguments
        .content
        .as_deref()
        .or(arguments.query.as_deref())
        .unwrap_or_default();
    let model_role = arguments.model_role.as_deref().unwrap_or("compactor");
    let write_scope = arguments
        .write_scope_json
        .as_deref()
        .unwrap_or("{\"paths\":[]}");
    session
        .record_runtime_event(
            "subagent.spawned",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"subagent_id\":{},\"agent_type\":\"explorer\",\"model_role\":{},\"status\":\"running\",\"isolation\":\"event_summary_only\",\"write_scope\":{},\"prompt_preview\":{}}}",
                json_string(&task_id),
                json_string(model_role),
                write_scope,
                json_string(&prompt.chars().take(240).collect::<String>())
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_runtime_event(
            "subagent.completed",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"subagent_id\":{},\"status\":\"completed\",\"summary\":{},\"evidence_refs\":{},\"parent_merge\":\"summary_only\"}}",
                json_string(&task_id),
                json_string(&result.preview),
                extract_json_value(&result.detail_json, "evidence_refs").unwrap_or_else(|| {
                    format!(
                        "[{}]",
                        json_string(&format!("tool_result:{}", stable_text_hash(&result.detail_json)))
                    )
                })
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    Ok(())
}

pub(in crate::native_agent_loop) enum PermissionedWriteOutcome {
    Executed(crate::tool_execution::ToolExecutionResult),
    Pending(PendingNativeToolExecution),
}

pub(in crate::native_agent_loop) enum PermissionedCommandOutcome {
    Executed(crate::tool_execution::ToolExecutionResult),
    Pending(PendingNativeToolExecution),
}

pub(in crate::native_agent_loop) fn execute_permissioned_command_collect(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    permission_gate: &mut PermissionGate,
    workspace_root: &Path,
    index: usize,
    provider_tool_call_id: Option<&str>,
    arguments: &ParsedToolArguments,
    permission_mode: &PermissionMode,
    provided_permission_decisions: &[NativeAgentPermissionDecision],
    hook_dispatcher: Option<&HookDispatcher>,
) -> Result<PermissionedCommandOutcome, String> {
    let tool_call_id = format!("native_loop_v2_tool_{index}");
    let tool_id = "shell.command";
    let args = tool_args(arguments);
    if let Some(command) = args.command.as_deref() {
        let classification = classify_command_with_reasons(command);
        if classification.decision == CommandDecision::Deny {
            let result = execute_model_readable_error_collect(
                session,
                artifact_store,
                index,
                provider_tool_call_id,
                tool_id,
                &ModelReadableToolError {
                    error_code: "COMMAND_CLASSIFIER_BLOCKED".to_string(),
                    tool_name: tool_id.to_string(),
                    short_message: format!(
                        "The shell command was blocked by the command classifier: {}. Ask for a safer command or use a read-only workspace tool instead.",
                        classification.reasons.join("; ")
                    ),
                    field_errors: Vec::new(),
                    retryable: true,
                    retry_hint: Some("Use a read-only file/search/git tool, or request approval for a safer command.".to_string()),
                    retry_example: Some(r#"{"tool_id":"file.list_directory","arguments":{"path":"."}}"#.to_string()),
                    counts_against_budget: true,
                    suggested_replacement: Some("file.list_directory".to_string()),
                },
            )?;
            return Ok(PermissionedCommandOutcome::Executed(result));
        }
    }
    record_tool_call_requested_preserving_provider_id(
        session,
        &tool_call_id,
        provider_tool_call_id,
        tool_id,
    )?;
    let args_json = tool_args_json(arguments);
    if let Some(reason) =
        dispatch_pre_tool_use_hook(hook_dispatcher, tool_id, &tool_call_id, &args_json)
    {
        return Err(format!(
            "shell.command blocked by PreToolUse hook: {reason}"
        ));
    }

    let permission_id = format!("native_loop_v2_command_perm_{index}");
    let request_type = PermissionRequestType::Command;
    let decision = match tool_permission_decision(
        session,
        artifact_store,
        permission_gate,
        permission_mode,
        tool_id,
        &permission_id,
        request_type.clone(),
        &args,
        provided_permission_decisions,
    )? {
        NativePermissionDecisionOutcome::Allow(decision) => decision,
        NativePermissionDecisionOutcome::Denied(error) => {
            let result = execute_model_readable_error_collect(
                session,
                artifact_store,
                index,
                provider_tool_call_id,
                tool_id,
                &error,
            )?;
            return Ok(PermissionedCommandOutcome::Executed(result));
        }
        NativePermissionDecisionOutcome::Pending => {
            return Ok(PermissionedCommandOutcome::Pending(
                PendingNativeToolExecution {
                    step_index: index,
                    tool_call_id,
                    tool_id: tool_id.to_string(),
                    permission_id,
                    request_type,
                    patch_id: None,
                    args,
                },
            ));
        }
    };

    let result = execute_tool_with_permission_gate(
        &ToolExecutionRequest {
            workspace_root: workspace_root.to_path_buf(),
            tool_call_id: tool_call_id.clone(),
            tool_id: tool_id.to_string(),
            mode: ToolExecutionMode::ApplyWithPermission {
                permission_decision: Some(decision),
            },
            args,
        },
        permission_gate,
    )
    .map_err(|error| format!("{error:?}"))?;
    record_tool_call_completed_preserving_provider_id(
        session,
        &tool_call_id,
        provider_tool_call_id,
        tool_id,
        result.ok,
    )?;
    let artifact = write_tool_result_artifact(
        artifact_store,
        &format!("native_loop_v2_permissioned_command_result_{index}"),
        &ToolResultRecord::new(
            &tool_call_id,
            tool_id,
            result.ok,
            result.preview.clone(),
            result.detail_json.clone(),
        ),
    )
    .map_err(|error| error.to_string())?;
    record_tool_result_artifact_preserving_provider_id(
        session,
        &tool_call_id,
        provider_tool_call_id,
        tool_id,
        artifact.artifact_id,
        artifact.content_hash,
        result.preview.clone(),
    )?;
    ensure_executing(session)?;
    dispatch_post_tool_use_hook(hook_dispatcher, tool_id, result.ok, &result.preview);
    Ok(PermissionedCommandOutcome::Executed(result))
}

pub(in crate::native_agent_loop) fn execute_permissioned_write_collect(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    permission_gate: &mut PermissionGate,
    workspace_root: &Path,
    index: usize,
    provider_tool_call_id: Option<&str>,
    tool_id: &str,
    arguments: &ParsedToolArguments,
    prompt: &str,
    permission_id_override: Option<String>,
    permission_mode: &PermissionMode,
    provided_permission_decisions: &[NativeAgentPermissionDecision],
    hook_dispatcher: Option<&HookDispatcher>,
) -> Result<PermissionedWriteOutcome, String> {
    let tool_call_id = format!("native_loop_v2_tool_{index}");
    record_tool_call_requested_preserving_provider_id(
        session,
        &tool_call_id,
        provider_tool_call_id,
        tool_id,
    )?;
    let args_json = tool_args_json(arguments);
    if let Some(reason) =
        dispatch_pre_tool_use_hook(hook_dispatcher, tool_id, &tool_call_id, &args_json)
    {
        return Err(format!(
            "tool '{tool_id}' blocked by PreToolUse hook: {reason}"
        ));
    }

    let args = tool_args(arguments);
    let permission_id =
        permission_id_override.unwrap_or_else(|| format!("native_loop_v2_write_perm_{index}"));
    let request_type = PermissionRequestType::FileWrite;
    let decision = match tool_permission_decision(
        session,
        artifact_store,
        permission_gate,
        permission_mode,
        tool_id,
        &permission_id,
        request_type.clone(),
        &args,
        provided_permission_decisions,
    )? {
        NativePermissionDecisionOutcome::Allow(decision) => decision,
        NativePermissionDecisionOutcome::Denied(error) => {
            let result = execute_model_readable_error_collect(
                session,
                artifact_store,
                index,
                provider_tool_call_id,
                tool_id,
                &error,
            )?;
            return Ok(PermissionedWriteOutcome::Executed(result));
        }
        NativePermissionDecisionOutcome::Pending => {
            return Ok(PermissionedWriteOutcome::Pending(
                PendingNativeToolExecution {
                    step_index: index,
                    tool_call_id,
                    tool_id: tool_id.to_string(),
                    permission_id,
                    request_type,
                    patch_id: None,
                    args,
                },
            ));
        }
    };

    let result = match prepare_exact_edit_execution_args(workspace_root, tool_id, &args) {
        Err(error) => native_loop_tool_execution_error_result(&tool_call_id, tool_id, &error),
        Ok(executable_args) => {
            if let Some(result) = validate_fast_auto_write_runtime_constraints(
                &tool_call_id,
                tool_id,
                arguments,
                prompt,
            ) {
                result
            } else {
                match execute_tool_with_permission_gate(
                    &ToolExecutionRequest {
                        workspace_root: workspace_root.to_path_buf(),
                        tool_call_id: tool_call_id.clone(),
                        tool_id: tool_id.to_string(),
                        mode: ToolExecutionMode::ApplyWithPermission {
                            permission_decision: Some(decision),
                        },
                        args: executable_args,
                    },
                    permission_gate,
                ) {
                    Ok(result) => result,
                    Err(error) => execute_fast_auto_write_create_repair(
                        workspace_root,
                        &tool_call_id,
                        tool_id,
                        arguments,
                        prompt,
                        &error,
                    )
                    .unwrap_or_else(|| {
                        native_loop_tool_execution_error_result(&tool_call_id, tool_id, &error)
                    }),
                }
            }
        }
    };
    record_tool_call_completed_preserving_provider_id(
        session,
        &tool_call_id,
        provider_tool_call_id,
        tool_id,
        result.ok,
    )?;
    let artifact = write_tool_result_artifact(
        artifact_store,
        &format!("native_loop_v2_write_result_{index}"),
        &ToolResultRecord::new(
            &tool_call_id,
            tool_id,
            result.ok,
            result.preview.clone(),
            result.detail_json.clone(),
        ),
    )
    .map_err(|error| error.to_string())?;
    record_tool_result_artifact_preserving_provider_id(
        session,
        &tool_call_id,
        provider_tool_call_id,
        tool_id,
        artifact.artifact_id,
        artifact.content_hash,
        result.preview.clone(),
    )?;
    ensure_executing(session)?;
    dispatch_post_tool_use_hook(hook_dispatcher, tool_id, result.ok, &result.preview);
    Ok(PermissionedWriteOutcome::Executed(result))
}

pub(in crate::native_agent_loop) fn execute_fast_auto_write_create_repair(
    workspace_root: &Path,
    tool_call_id: &str,
    tool_id: &str,
    arguments: &ParsedToolArguments,
    prompt: &str,
    error: &crate::tool_execution::ToolExecutionError,
) -> Option<crate::tool_execution::ToolExecutionResult> {
    if tool_id != "file.write" || !native_prompt_wants_file_generation(prompt) {
        return None;
    }
    match error {
        crate::tool_execution::ToolExecutionError::MissingArgument(argument)
            if argument == "base_hash" => {}
        crate::tool_execution::ToolExecutionError::ValidationFailed(reason)
            if reason == "FailStale" => {}
        _ => return None,
    }
    let repaired_path =
        next_fast_auto_write_create_path(workspace_root, arguments.path.as_deref()?)?;
    let mut repaired = arguments.clone();
    repaired.path = Some(repaired_path.clone());
    repaired.base_hash = None;
    match execute_tool(&ToolExecutionRequest {
        workspace_root: workspace_root.to_path_buf(),
        tool_call_id: tool_call_id.to_string(),
        tool_id: tool_id.to_string(),
        mode: ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(researchcode_kernel::PermissionDecisionKind::AllowOnce),
        },
        args: tool_args(&repaired),
    }) {
        Ok(mut result) => {
            result.preview = format!(
                "{}; fast_auto_write_repaired_path={repaired_path}",
                result.preview
            );
            result.detail_json = merge_fast_auto_write_repair_detail(
                &result.detail_json,
                arguments.path.as_deref().unwrap_or_default(),
                &repaired_path,
            );
            Some(result)
        }
        Err(retry_error) => Some(native_loop_tool_execution_error_result(
            tool_call_id,
            tool_id,
            &retry_error,
        )),
    }
}

fn next_fast_auto_write_create_path(workspace_root: &Path, requested_path: &str) -> Option<String> {
    let requested = Path::new(requested_path);
    if requested.is_absolute() {
        return None;
    }
    if requested
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    if !workspace_root.join(requested).exists() {
        return None;
    }
    let parent = requested
        .parent()
        .filter(|path| !path.as_os_str().is_empty());
    let stem = requested
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("generated_app");
    let extension = requested.extension().and_then(|value| value.to_str());
    for index in 2..100 {
        let filename = match extension {
            Some(extension) if !extension.is_empty() => format!("{stem}_{index}.{extension}"),
            _ => format!("{stem}_{index}"),
        };
        let candidate = parent.map_or_else(|| PathBuf::from(&filename), |dir| dir.join(&filename));
        if !workspace_root.join(&candidate).exists() {
            return Some(candidate.to_string_lossy().replace('\\', "/"));
        }
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let filename = match extension {
        Some(extension) if !extension.is_empty() => format!("{stem}_{nonce}.{extension}"),
        _ => format!("{stem}_{nonce}"),
    };
    let candidate = parent.map_or_else(|| PathBuf::from(&filename), |dir| dir.join(&filename));
    Some(candidate.to_string_lossy().replace('\\', "/"))
}

fn merge_fast_auto_write_repair_detail(
    detail_json: &str,
    original_path: &str,
    repaired_path: &str,
) -> String {
    let suffix = format!(
        "\"fast_auto_write_repaired\":true,\"original_path\":{},\"repaired_path\":{}",
        json_string(original_path),
        json_string(repaired_path)
    );
    if let Some(prefix) = detail_json.strip_suffix('}') {
        if prefix.ends_with('{') {
            format!("{prefix}{suffix}}}")
        } else {
            format!("{prefix},{suffix}}}")
        }
    } else {
        format!("{{{suffix}}}")
    }
}
