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
    execute_tool, execute_tool_with_permission_gate, ToolExecutionArgs, ToolExecutionMode,
    ToolExecutionRequest,
};
use crate::tool_result::{write_tool_result_artifact, ToolResultRecord};
use researchcode_kernel::hooks::{HookDecision, HookEvent};
use researchcode_kernel::model::{NativeModelFamily, NativeModelProfile, OptimizationLevel};
use researchcode_kernel::tool::{find_tool_spec, provider_tool_name_for_id};
use researchcode_kernel::{Actor, PermissionDecisionKind, PermissionRequestType};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

// Layer B: turn completion and structured-stop helpers.

use crate::native_agent_loop::native_agent_loop_util::{
    json_optional_string, json_string, resolve_workspace_path, resolve_workspace_write_path,
};
use crate::native_agent_loop::{
    NativeAgentPermissionDecision, PendingNativeToolExecution, EXTERNAL_PERMISSION_NOT_ALLOWED,
};

pub(in crate::native_agent_loop) fn record_tool_call_requested_preserving_provider_id(
    session: &mut AgentSession,
    tool_call_id: &str,
    provider_tool_call_id: Option<&str>,
    tool_id: &str,
) -> Result<(), String> {
    if let Some(provider_tool_call_id) = provider_tool_call_id {
        session
            .record_tool_call_requested_with_provider_id(
                tool_call_id,
                Some(provider_tool_call_id.to_string()),
                tool_id,
            )
            .map_err(|error| format!("{error:?}"))
    } else {
        session
            .record_tool_call_requested(tool_call_id, tool_id)
            .map_err(|error| format!("{error:?}"))
    }
}

pub(in crate::native_agent_loop) fn record_tool_call_completed_preserving_provider_id(
    session: &mut AgentSession,
    tool_call_id: &str,
    provider_tool_call_id: Option<&str>,
    tool_id: &str,
    ok: bool,
) -> Result<(), String> {
    if let Some(provider_tool_call_id) = provider_tool_call_id {
        session
            .record_tool_call_completed_with_provider_id(
                tool_call_id,
                Some(provider_tool_call_id.to_string()),
                tool_id,
                ok,
            )
            .map_err(|error| format!("{error:?}"))
    } else {
        session
            .record_tool_call_completed(tool_call_id, tool_id, ok)
            .map_err(|error| format!("{error:?}"))
    }
}

pub(in crate::native_agent_loop) fn record_tool_result_artifact_preserving_provider_id(
    session: &mut AgentSession,
    tool_call_id: &str,
    provider_tool_call_id: Option<&str>,
    tool_id: &str,
    artifact_id: String,
    content_hash: String,
    preview: String,
) -> Result<(), String> {
    if let Some(provider_tool_call_id) = provider_tool_call_id {
        session
            .record_tool_result_artifact_with_provider_id(
                tool_call_id,
                Some(provider_tool_call_id.to_string()),
                tool_id,
                artifact_id,
                content_hash,
                preview,
            )
            .map_err(|error| format!("{error:?}"))
    } else {
        session
            .record_tool_result_artifact(tool_call_id, tool_id, artifact_id, content_hash, preview)
            .map_err(|error| format!("{error:?}"))
    }
}

pub(in crate::native_agent_loop) fn execute_model_readable_error_collect(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    index: usize,
    provider_tool_call_id: Option<&str>,
    requested_tool_id: &str,
    error: &ModelReadableToolError,
) -> Result<crate::tool_execution::ToolExecutionResult, String> {
    let tool_call_id = format!("native_loop_v2_tool_{index}");
    let event_tool_id = if find_tool_spec(requested_tool_id).is_some() {
        Some(requested_tool_id.to_string())
    } else if find_tool_spec(&error.tool_name).is_some() {
        Some(error.tool_name.clone())
    } else {
        error
            .suggested_replacement
            .as_deref()
            .filter(|tool_id| find_tool_spec(tool_id).is_some())
            .map(|tool_id| tool_id.to_string())
    };
    let result_tool_id = event_tool_id
        .as_deref()
        .unwrap_or(requested_tool_id)
        .to_string();
    let result = model_error_to_tool_result(&tool_call_id, &result_tool_id, error);
    session
        .record_runtime_event(
            "tool.model_readable_error",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"tool_call_id\":{},\"provider_tool_call_id\":{},\"requested_tool_id\":{},\"result_tool_id\":{},\"error\":{}}}",
                json_string(&tool_call_id),
                json_optional_string(provider_tool_call_id),
                json_string(requested_tool_id),
                json_string(&result_tool_id),
                error.to_payload_json()
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    if let Some(event_tool_id) = event_tool_id.as_deref() {
        record_tool_call_requested_preserving_provider_id(
            session,
            &tool_call_id,
            provider_tool_call_id,
            event_tool_id,
        )?;
        record_tool_call_completed_preserving_provider_id(
            session,
            &tool_call_id,
            provider_tool_call_id,
            event_tool_id,
            false,
        )?;
    }
    let artifact = write_tool_result_artifact(
        artifact_store,
        &format!("native_loop_v2_model_readable_tool_error_{index}"),
        &ToolResultRecord::new(
            &tool_call_id,
            &result_tool_id,
            false,
            result.preview.clone(),
            result.detail_json.clone(),
        ),
    )
    .map_err(|error| error.to_string())?;
    if let Some(event_tool_id) = event_tool_id.as_deref() {
        record_tool_result_artifact_preserving_provider_id(
            session,
            &tool_call_id,
            provider_tool_call_id,
            event_tool_id,
            artifact.artifact_id,
            artifact.content_hash,
            result.preview.clone(),
        )?;
    } else {
        session
            .record_runtime_event(
                "tool.model_readable_error_recorded",
                researchcode_kernel::Actor::Runtime,
                format!(
                    "{{\"tool_call_id\":{},\"requested_tool\":{},\"error_code\":{},\"artifact_id\":{},\"content_hash\":{},\"preview\":{}}}",
                    json_string(&tool_call_id),
                    json_string(requested_tool_id),
                    json_string(&error.error_code),
                    json_string(&artifact.artifact_id),
                    json_string(&artifact.content_hash),
                    json_string(&result.preview)
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
    }
    ensure_executing(session)?;
    Ok(result)
}

pub(in crate::native_agent_loop) fn tool_args_json(arguments: &ParsedToolArguments) -> String {
    let mut object = serde_json::Map::new();
    if let Some(ref path) = arguments.path {
        object.insert("path".to_string(), serde_json::Value::String(path.clone()));
    }
    if let Some(ref root) = arguments.root {
        object.insert("root".to_string(), serde_json::Value::String(root.clone()));
    }
    if let Some(include_hidden) = arguments.include_hidden {
        object.insert(
            "include_hidden".to_string(),
            serde_json::Value::Bool(include_hidden),
        );
    }
    if let Some(offset) = arguments.offset {
        object.insert("offset".to_string(), serde_json::json!(offset));
    }
    if let Some(limit) = arguments.limit {
        object.insert("limit".to_string(), serde_json::json!(limit));
    }
    if let Some(max_bytes) = arguments.max_bytes {
        object.insert("max_bytes".to_string(), serde_json::json!(max_bytes));
    }
    if let Some(max_results) = arguments.max_results {
        object.insert("max_results".to_string(), serde_json::json!(max_results));
    }
    if let Some(max_files) = arguments.max_files {
        object.insert("max_files".to_string(), serde_json::json!(max_files));
    }
    if let Some(max_depth) = arguments.max_depth {
        object.insert("max_depth".to_string(), serde_json::json!(max_depth));
    }
    if let Some(ref command) = arguments.command {
        object.insert(
            "command".to_string(),
            serde_json::Value::String(command.clone()),
        );
    }
    if let Some(ref content) = arguments.content {
        object.insert(
            "content".to_string(),
            serde_json::Value::String(content.clone()),
        );
    }
    if let Some(ref pattern) = arguments.pattern {
        object.insert(
            "pattern".to_string(),
            serde_json::Value::String(pattern.clone()),
        );
    }
    if let Some(ref query) = arguments.query {
        object.insert(
            "query".to_string(),
            serde_json::Value::String(query.clone()),
        );
    }
    if let Some(ref old_string) = arguments.old_string {
        object.insert(
            "old_string".to_string(),
            serde_json::Value::String(old_string.clone()),
        );
    }
    if let Some(ref new_string) = arguments.new_string {
        object.insert(
            "new_string".to_string(),
            serde_json::Value::String(new_string.clone()),
        );
    }
    if let Some(ref base_hash) = arguments.base_hash {
        object.insert(
            "base_hash".to_string(),
            serde_json::Value::String(base_hash.clone()),
        );
    }
    if let Some(replace_all) = arguments.replace_all {
        object.insert(
            "replace_all".to_string(),
            serde_json::Value::Bool(replace_all),
        );
    }
    if let Some(ref edits_json) = arguments.edits_json {
        object.insert(
            "edits".to_string(),
            serde_json::from_str(edits_json)
                .unwrap_or_else(|_| serde_json::Value::String(edits_json.clone())),
        );
    }
    if let Some(ref input_csv) = arguments.input_csv {
        object.insert(
            "input_csv".to_string(),
            serde_json::Value::String(input_csv.clone()),
        );
    }
    if let Some(ref job_id) = arguments.job_id {
        object.insert(
            "job_id".to_string(),
            serde_json::Value::String(job_id.clone()),
        );
    }
    if let Some(ref answer) = arguments.answer {
        object.insert(
            "answer".to_string(),
            serde_json::Value::String(answer.clone()),
        );
    }
    if let Some(ref model_role) = arguments.model_role {
        object.insert(
            "model_role".to_string(),
            serde_json::Value::String(model_role.clone()),
        );
    }
    if let Some(ref write_scope_json) = arguments.write_scope_json {
        object.insert(
            "write_scope".to_string(),
            serde_json::from_str(write_scope_json)
                .unwrap_or_else(|_| serde_json::Value::String(write_scope_json.clone())),
        );
    }
    serde_json::Value::Object(object).to_string()
}

pub(in crate::native_agent_loop) fn dispatch_pre_tool_use_hook(
    hook_dispatcher: Option<&HookDispatcher>,
    tool_id: &str,
    tool_call_id: &str,
    args_json: &str,
) -> Option<String> {
    let dispatcher = hook_dispatcher?;
    if dispatcher.is_empty() {
        return None;
    }
    let outcomes = dispatcher.dispatch(&HookEvent::PreToolUse {
        tool_id: tool_id.to_string(),
        args_json: args_json.to_string(),
        provider_tool_use_id: Some(tool_call_id.to_string()),
    });
    for outcome in &outcomes {
        if let HookDecision::Deny { reason } = &outcome.decision {
            return Some(reason.clone());
        }
    }
    None
}

pub(in crate::native_agent_loop) fn dispatch_post_tool_use_hook(
    hook_dispatcher: Option<&HookDispatcher>,
    tool_id: &str,
    ok: bool,
    preview: &str,
) {
    if let Some(dispatcher) = hook_dispatcher {
        if !dispatcher.is_empty() {
            dispatcher.dispatch(&HookEvent::PostToolUse {
                tool_id: tool_id.to_string(),
                result_preview: preview.to_string(),
                ok,
                duration_ms: 0,
            });
        }
    }
}

pub(in crate::native_agent_loop) enum NativePermissionDecisionOutcome {
    Allow(PermissionDecisionKind),
    Pending,
    Denied(ModelReadableToolError),
}

#[cfg(test)]
pub(in crate::native_agent_loop) fn execute_patch(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    permission_gate: &mut PermissionGate,
    workspace_root: &Path,
    index: usize,
    arguments: &ParsedToolArguments,
    permission_mode: &PermissionMode,
    provided_permission_decisions: &[NativeAgentPermissionDecision],
    hook_dispatcher: Option<&HookDispatcher>,
) -> Result<Option<PendingNativeToolExecution>, String> {
    let tool_call_id = format!("native_loop_tool_{index}");
    let patch_id = format!("native_loop_patch_{index}");
    let path_arg = arguments
        .path
        .clone()
        .ok_or_else(|| "patch.apply requires path".to_string())?;
    let _ = resolve_workspace_write_path(workspace_root, &path_arg)?;
    if arguments.old_string.is_none() {
        return Err("patch.apply requires old_string".to_string());
    }
    let mut args = tool_args(arguments);
    session
        .record_tool_call_requested(&tool_call_id, "patch.apply")
        .map_err(|error| format!("{error:?}"))?;
    let args_json = tool_args_json(arguments);
    if let Some(reason) =
        dispatch_pre_tool_use_hook(hook_dispatcher, "patch.apply", &tool_call_id, &args_json)
    {
        return Err(format!("patch.apply blocked by PreToolUse hook: {reason}"));
    }
    let permission_id = format!("native_loop_patch_perm_{index}");
    let decision = match tool_permission_decision(
        session,
        artifact_store,
        permission_gate,
        permission_mode,
        "patch.apply",
        &permission_id,
        PermissionRequestType::FileWrite,
        &args,
        provided_permission_decisions,
    )? {
        NativePermissionDecisionOutcome::Allow(decision) => decision,
        NativePermissionDecisionOutcome::Denied(error) => {
            let _ = execute_model_readable_error_collect(
                session,
                artifact_store,
                index,
                None,
                "patch.apply",
                &error,
            )?;
            return Ok(None);
        }
        NativePermissionDecisionOutcome::Pending => {
            return Ok(Some(PendingNativeToolExecution {
                step_index: index,
                tool_call_id,
                tool_id: "patch.apply".to_string(),
                permission_id,
                request_type: PermissionRequestType::FileWrite,
                patch_id: Some(patch_id),
                args,
            }));
        }
    };
    args = match prepare_patch_execution_args(session, workspace_root, &patch_id, &args) {
        Ok(args) => args,
        Err(error) => {
            session
                .record_tool_call_completed(&tool_call_id, "patch.apply", false)
                .map_err(|session_error| format!("{session_error:?}"))?;
            return Err(error);
        }
    };
    let result = execute_tool_with_permission_gate(
        &ToolExecutionRequest {
            workspace_root: workspace_root.to_path_buf(),
            tool_call_id: tool_call_id.clone(),
            tool_id: "patch.apply".to_string(),
            mode: ToolExecutionMode::ApplyWithPermission {
                permission_decision: Some(decision),
            },
            args: args.clone(),
        },
        permission_gate,
    )
    .map_err(|error| format!("{error:?}"))?;
    let applied_path_arg = args
        .path
        .as_deref()
        .ok_or_else(|| "patch.apply requires path".to_string())?;
    let applied_path = resolve_workspace_write_path(workspace_root, applied_path_arg)?;
    session
        .record_patch_applied(&patch_id, applied_path.to_string_lossy())
        .and_then(|_| session.record_tool_call_completed(&tool_call_id, "patch.apply", result.ok))
        .map_err(|error| format!("{error:?}"))?;
    let artifact = write_tool_result_artifact(
        artifact_store,
        &format!("native_loop_patch_result_{index}"),
        &ToolResultRecord::new(
            &tool_call_id,
            "patch.apply",
            result.ok,
            result.preview.clone(),
            result.detail_json,
        ),
    )
    .map_err(|error| error.to_string())?;
    let patch_ok = result.ok;
    session
        .record_tool_result_artifact(
            &tool_call_id,
            "patch.apply",
            artifact.artifact_id,
            artifact.content_hash,
            result.preview.clone(),
        )
        .map_err(|error| format!("{error:?}"))?;
    ensure_executing(session)?;
    dispatch_post_tool_use_hook(hook_dispatcher, "patch.apply", patch_ok, &result.preview);
    Ok(None)
}

pub(in crate::native_agent_loop) fn prepare_patch_execution_args(
    session: &mut AgentSession,
    workspace_root: &Path,
    patch_id: &str,
    args: &ToolExecutionArgs,
) -> Result<ToolExecutionArgs, String> {
    let path_arg = args
        .path
        .clone()
        .ok_or_else(|| "patch.apply requires path".to_string())?;
    let path = resolve_workspace_write_path(workspace_root, &path_arg)?;
    let old_string = args
        .old_string
        .clone()
        .ok_or_else(|| "patch.apply requires old_string".to_string())?;
    let current_text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let base_hash = stable_text_hash(&current_text);
    let validation = validate_patch_allowing_protected(PatchCheck {
        path: &path.to_string_lossy(),
        current_text: Some(&current_text),
        current_hash: Some(&base_hash),
        old_string: &old_string,
        base_hash: &base_hash,
    });
    session
        .record_patch_proposal_created(patch_id, path.to_string_lossy())
        .and_then(|_| session.record_patch_proposal_validated(patch_id, validation.clone()))
        .map_err(|error| format!("{error:?}"))?;
    if validation != PatchValidation::Pass {
        return Err(format!(
            "native loop patch validation failed: {validation:?}"
        ));
    }
    let mut executable_args = args.clone();
    executable_args.base_hash = Some(base_hash);
    Ok(executable_args)
}

pub(in crate::native_agent_loop) fn prepare_exact_edit_execution_args(
    workspace_root: &Path,
    tool_id: &str,
    args: &ToolExecutionArgs,
) -> Result<ToolExecutionArgs, crate::tool_execution::ToolExecutionError> {
    if !matches!(tool_id, "file.write" | "file.edit" | "file.multi_edit") {
        return Ok(args.clone());
    }
    if args.base_hash.is_some() {
        return Ok(args.clone());
    }
    let path_arg = args
        .path
        .clone()
        .ok_or_else(|| crate::tool_execution::ToolExecutionError::MissingArgument("path".into()))?;
    let path = resolve_workspace_write_path(workspace_root, &path_arg).map_err(|error| {
        crate::tool_execution::ToolExecutionError::PathEscapesWorkspace(error.to_string())
    })?;
    let current_text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if tool_id == "file.write" && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(args.clone());
        }
        Err(error) => {
            return Err(crate::tool_execution::ToolExecutionError::ToolFailed(
                format!("{tool_id} could not read {path_arg} before dispatch: {error}"),
            ));
        }
    };
    let mut executable_args = args.clone();
    executable_args.base_hash = Some(stable_text_hash(&current_text));
    Ok(executable_args)
}

pub(in crate::native_agent_loop) fn ensure_executing(
    session: &mut AgentSession,
) -> Result<(), String> {
    match session.state() {
        AgentState::Executing => Ok(()),
        AgentState::Completed => session
            .begin_interactive_turn("native_loop.tool_execution", "terminal_reopen")
            .map_err(|error| format!("{error:?}")),
        AgentState::Created => session
            .transition_to(AgentState::Planning)
            .and_then(|_| session.transition_to(AgentState::RetrievingContext))
            .and_then(|_| session.transition_to(AgentState::Executing))
            .map_err(|error| format!("{error:?}")),
        AgentState::Planning => session
            .transition_to(AgentState::RetrievingContext)
            .and_then(|_| session.transition_to(AgentState::Executing))
            .map_err(|error| format!("{error:?}")),
        AgentState::RetrievingContext
        | AgentState::ApplyingPatch
        | AgentState::RunningCommand
        | AgentState::DiagnosingFailure
        | AgentState::Reviewing
        | AgentState::WaitingForUser => session
            .transition_to(AgentState::Executing)
            .map_err(|error| format!("{error:?}")),
        AgentState::WaitingForToolApproval => {
            Err("cannot execute another tool while waiting for tool approval".to_string())
        }
        AgentState::WaitingForPlanApproval => {
            Err("cannot execute tools while waiting for plan approval".to_string())
        }
        AgentState::Failed | AgentState::Cancelled => Err(format!(
            "cannot reopen terminal session state {:?} for tool execution",
            session.state()
        )),
    }
}

pub(in crate::native_agent_loop) fn execute_pending_tool_after_decision(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    workspace_root: &Path,
    pending: &PendingNativeToolExecution,
    decision: PermissionDecisionKind,
    hook_dispatcher: Option<&HookDispatcher>,
) -> Result<bool, String> {
    session
        .decide_permission(decision.clone())
        .map_err(|error| format!("{error:?}"))?;
    if !matches!(
        decision,
        PermissionDecisionKind::AllowOnce
            | PermissionDecisionKind::AllowSession
            | PermissionDecisionKind::AllowProjectRule
    ) {
        return Ok(false);
    }
    let mut args = pending.args.clone();
    if pending.tool_id == "patch.apply" {
        let patch_id = pending
            .patch_id
            .clone()
            .ok_or_else(|| "pending patch missing patch_id".to_string())?;
        args = match prepare_patch_execution_args(session, workspace_root, &patch_id, &args) {
            Ok(args) => args,
            Err(error) => {
                session
                    .record_tool_call_completed(&pending.tool_call_id, &pending.tool_id, false)
                    .map_err(|session_error| format!("{session_error:?}"))?;
                return Err(error);
            }
        };
    } else if matches!(
        pending.tool_id.as_str(),
        "file.write" | "file.edit" | "file.multi_edit"
    ) {
        args = match prepare_exact_edit_execution_args(workspace_root, &pending.tool_id, &args) {
            Ok(args) => args,
            Err(error) => {
                session
                    .record_tool_call_completed(&pending.tool_call_id, &pending.tool_id, false)
                    .map_err(|session_error| format!("{session_error:?}"))?;
                return Err(format!("{error:?}"));
            }
        };
    }
    let result = execute_tool(&ToolExecutionRequest {
        workspace_root: workspace_root.to_path_buf(),
        tool_call_id: pending.tool_call_id.clone(),
        tool_id: pending.tool_id.clone(),
        mode: ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(decision),
        },
        args: args.clone(),
    })
    .map_err(|error| format!("{error:?}"))?;
    if pending.tool_id == "patch.apply" {
        let patch_id = pending
            .patch_id
            .clone()
            .ok_or_else(|| "pending patch missing patch_id".to_string())?;
        let path = pending
            .args
            .path
            .clone()
            .ok_or_else(|| "pending patch missing path".to_string())?;
        let resolved = resolve_workspace_path(workspace_root, &path)?;
        session
            .record_patch_applied(&patch_id, resolved.to_string_lossy())
            .map_err(|error| format!("{error:?}"))?;
    }
    let resumed_ok = result.ok;
    session
        .record_tool_call_completed(&pending.tool_call_id, &pending.tool_id, resumed_ok)
        .map_err(|error| format!("{error:?}"))?;
    let artifact = write_tool_result_artifact(
        artifact_store,
        &format!("native_loop_resumed_tool_result_{}", pending.step_index),
        &ToolResultRecord::new(
            &pending.tool_call_id,
            &pending.tool_id,
            resumed_ok,
            result.preview.clone(),
            result.detail_json,
        ),
    )
    .map_err(|error| error.to_string())?;
    session
        .record_tool_result_artifact(
            &pending.tool_call_id,
            &pending.tool_id,
            artifact.artifact_id,
            artifact.content_hash,
            result.preview.clone(),
        )
        .map_err(|error| format!("{error:?}"))?;
    dispatch_post_tool_use_hook(
        hook_dispatcher,
        &pending.tool_id,
        resumed_ok,
        &result.preview,
    );
    ensure_executing(session)?;
    Ok(true)
}

pub(in crate::native_agent_loop) fn tool_permission_decision(
    session: &mut AgentSession,
    _artifact_store: &ArtifactStore,
    permission_gate: &mut PermissionGate,
    mode: &PermissionMode,
    tool_id: &str,
    permission_id: &str,
    request_type: PermissionRequestType,
    args: &ToolExecutionArgs,
    provided_permission_decisions: &[NativeAgentPermissionDecision],
) -> Result<NativePermissionDecisionOutcome, String> {
    let args_json = permission_args_json(args);
    let summary = permission_summary_for_tool(tool_id, args);
    let tool: Box<dyn PermissionCheck> = permission_tool_for_id(tool_id);
    let request = PermissionRequest {
        mode: *mode,
        tool_id,
        args: &args_json,
        request_type: request_type.clone(),
        session_id: permission_id,
        command_summary: summary.as_deref(),
    };
    let decision = permission_gate.evaluate(request, tool.as_ref());
    record_permission_decision_recorded(
        session,
        tool_id,
        mode,
        &request_type,
        &decision,
        permission_gate.denial_count(),
    )?;
    match decision {
        PermissionResolution::Allow => {
            return Ok(NativePermissionDecisionOutcome::Allow(
                PermissionDecisionKind::AllowOnce,
            ));
        }
        PermissionResolution::Deny { reason } => {
            return Ok(NativePermissionDecisionOutcome::Denied(
                ModelReadableToolError {
                    error_code: "PERMISSION_DENIED".to_string(),
                    tool_name: tool_id.to_string(),
                    short_message: format!(
                        "{tool_id} was denied by permission policy: {reason}. Do not retry the same call; choose an allowed tool/action or answer with available evidence."
                    ),
                    field_errors: Vec::new(),
                    retryable: false,
                    retry_hint: Some("Choose an allowed read-only tool/action or answer with available evidence.".to_string()),
                    retry_example: None,
                    counts_against_budget: false,
                    suggested_replacement: suggested_permission_denial_replacement(tool_id),
                },
            ));
        }
        PermissionResolution::Ask { .. } => {}
    }
    session
        .request_permission(
            permission_id,
            request_type.clone(),
            Some(tool_id.to_string()),
        )
        .map_err(|error| format!("{error:?}"))?;
    record_native_permission_context_event(session, permission_id, tool_id, &request_type, args)?;
    let Some(provided) = provided_permission_decisions
        .iter()
        .find(|candidate| candidate.permission_id == permission_id)
    else {
        return Ok(NativePermissionDecisionOutcome::Pending);
    };
    session
        .decide_permission(provided.decision.clone())
        .map_err(|error| format!("{error:?}"))?;
    match provided.decision {
        PermissionDecisionKind::AllowOnce
        | PermissionDecisionKind::AllowSession
        | PermissionDecisionKind::AllowProjectRule => {
            Ok(NativePermissionDecisionOutcome::Allow(provided.decision.clone()))
        }
        PermissionDecisionKind::Deny => Ok(NativePermissionDecisionOutcome::Denied(
            ModelReadableToolError {
                error_code: "PERMISSION_DENIED".to_string(),
                tool_name: tool_id.to_string(),
                short_message: format!(
                    "{tool_id} was denied by the user or permission policy. Do not retry the same call; choose an allowed tool/action or answer with available evidence."
                ),
                field_errors: Vec::new(),
                retryable: false,
                retry_hint: Some("Choose an allowed read-only tool/action or answer with available evidence.".to_string()),
                retry_example: None,
                counts_against_budget: false,
                suggested_replacement: suggested_permission_denial_replacement(tool_id),
            },
        )),
        PermissionDecisionKind::Modify => Err(EXTERNAL_PERMISSION_NOT_ALLOWED.to_string()),
    }
}

fn record_native_permission_context_event(
    session: &mut AgentSession,
    permission_id: &str,
    tool_id: &str,
    request_type: &PermissionRequestType,
    args: &ToolExecutionArgs,
) -> Result<(), String> {
    let args_preview = native_permission_args_preview(args);
    let path_preview = args
        .path
        .as_deref()
        .or(args.root.as_deref())
        .or(args.output_dir.as_deref())
        .or(args.input_csv.as_deref())
        .unwrap_or("");
    session
        .record_runtime_event(
            "permission.context",
            Actor::Runtime,
            format!(
                "{{\"permission_id\":{},\"tool_id\":{},\"request_type\":{},\"args_preview\":{},\"path_preview\":{},\"risk_level\":{}}}",
                json_string(permission_id),
                json_string(tool_id),
                json_string(native_permission_request_type_to_wire(request_type)),
                json_string(&args_preview),
                json_string(path_preview),
                json_string(native_permission_risk_level(tool_id, request_type, args))
            ),
        )
        .map_err(|error| format!("{error:?}"))
}

fn native_permission_args_preview(args: &ToolExecutionArgs) -> String {
    if let Some(command) = args
        .command
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return command.chars().take(320).collect();
    }
    if let Some(path) = args
        .path
        .as_deref()
        .or(args.root.as_deref())
        .or(args.output_dir.as_deref())
        .or(args.input_csv.as_deref())
        .filter(|value| !value.trim().is_empty())
    {
        let descriptor = args
            .query
            .as_deref()
            .or(args.pattern.as_deref())
            .unwrap_or("file operation");
        return format!("{descriptor}: {path}").chars().take(320).collect();
    }
    format!("{args:?}").chars().take(320).collect()
}

fn native_permission_risk_level(
    tool_id: &str,
    request_type: &PermissionRequestType,
    args: &ToolExecutionArgs,
) -> &'static str {
    let command = args.command.as_deref().unwrap_or_default().to_lowercase();
    if matches!(
        classify_command_with_reasons(&command).decision,
        CommandDecision::Deny
    ) || command.contains("rm -rf")
        || command.contains("sudo ")
    {
        return "critical";
    }
    if matches!(
        request_type,
        PermissionRequestType::PackageInstall
            | PermissionRequestType::Network
            | PermissionRequestType::ProtectedPath
    ) {
        return "high";
    }
    if tool_id == "shell.command" || matches!(request_type, PermissionRequestType::Command) {
        return "medium";
    }
    "low"
}

fn native_permission_request_type_to_wire(value: &PermissionRequestType) -> &'static str {
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

pub(in crate::native_agent_loop) fn record_permission_decision_recorded(
    session: &mut AgentSession,
    tool_id: &str,
    mode: &PermissionMode,
    request_type: &PermissionRequestType,
    decision: &PermissionResolution,
    denial_count_after: u32,
) -> Result<(), String> {
    session
        .record_runtime_event(
            "permission.decision.recorded",
            Actor::Runtime,
            format!(
                "{{\"tool_id\":{},\"mode\":{},\"request_type\":{},\"decision\":{},\"denial_count_after\":{}}}",
                json_string(tool_id),
                json_string(&format!("{mode:?}")),
                json_string(&format!("{request_type:?}")),
                json_string(&format!("{decision:?}")),
                denial_count_after
            ),
        )
        .map_err(|error| format!("{error:?}"))
}

pub(in crate::native_agent_loop) fn suggested_permission_denial_replacement(
    tool_id: &str,
) -> Option<String> {
    match tool_id {
        "shell.command" => Some("file.list_directory".to_string()),
        "file.write" | "file.edit" | "file.multi_edit" | "patch.apply" => None,
        _ => None,
    }
}

pub(in crate::native_agent_loop) fn permission_tool_for_id(
    tool_id: &str,
) -> Box<dyn PermissionCheck> {
    match tool_id {
        "shell.command" => Box::new(ShellCommandTool),
        "patch.apply" => Box::new(PatchApplyTool),
        "file.write" => Box::new(FileWriteTool),
        "file.edit" | "file.multi_edit" => Box::new(FileEditTool),
        _ => Box::new(DefaultTool::new(tool_id)),
    }
}

pub(in crate::native_agent_loop) fn permission_summary_for_tool(
    tool_id: &str,
    args: &ToolExecutionArgs,
) -> Option<String> {
    match tool_id {
        "shell.command" => args.command.clone(),
        "patch.apply" | "file.write" | "file.edit" | "file.multi_edit" => args.path.clone(),
        _ => None,
    }
}

pub(in crate::native_agent_loop) fn permission_args_json(
    args: &ToolExecutionArgs,
) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    if let Some(value) = &args.path {
        object.insert("path".to_string(), serde_json::Value::String(value.clone()));
    }
    if let Some(value) = &args.command {
        object.insert(
            "command".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.content {
        object.insert(
            "content".to_string(),
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
    serde_json::Value::Object(object)
}

pub(in crate::native_agent_loop) fn tool_args(
    arguments: &ParsedToolArguments,
) -> ToolExecutionArgs {
    ToolExecutionArgs {
        path: arguments.path.clone(),
        root: arguments.root.clone().or_else(|| Some(".".to_string())),
        include_hidden: arguments.include_hidden,
        command: arguments.command.clone(),
        content: arguments.content.clone(),
        pattern: arguments
            .pattern
            .clone()
            .or_else(|| arguments.query.clone()),
        query: arguments.query.clone(),
        old_string: arguments.old_string.clone(),
        new_string: arguments.new_string.clone(),
        base_hash: arguments.base_hash.clone(),
        replace_all: arguments.replace_all,
        offset: arguments.offset,
        limit: arguments.limit,
        max_bytes: arguments.max_bytes,
        max_results: arguments.max_results,
        max_files: arguments.max_files,
        max_depth: arguments.max_depth,
        edits_json: arguments.edits_json.clone(),
        input_csv: arguments.input_csv.clone(),
        job_id: arguments.job_id.clone(),
        ..ToolExecutionArgs::default()
    }
}
