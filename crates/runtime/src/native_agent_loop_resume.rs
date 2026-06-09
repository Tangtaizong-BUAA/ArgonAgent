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

use crate::native_agent_loop::native_agent_loop_execution::execute_pending_tool_after_decision;
use crate::native_agent_loop::native_agent_loop_tools::replayed_tool_completion_state;
use crate::native_agent_loop::native_agent_loop_util::{json_string, loop_result};
use crate::native_agent_loop::{
    NativeAgentLoopResult, NativeAgentLoopResumeRequest, NativeAgentLoopStatus,
};

pub fn resume_native_agent_loop_after_external_decision<T: LiveHttpTransport>(
    _transport: &T,
    request: NativeAgentLoopResumeRequest,
) -> Result<NativeAgentLoopResult, String> {
    let event_log = EventLog::import_jsonl(&request.previous_event_jsonl)
        .map_err(|error| format!("{error:?}"))?;
    let mut session =
        AgentSession::resume_from_event_log(event_log).map_err(|error| format!("{error:?}"))?;
    let artifact_store = ArtifactStore::new(&request.artifact_root);
    let mut tool_call_count = 0usize;
    let model_call_count = 0usize;

    session
        .record_runtime_event(
            "agent.recovery.started",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"reason\":\"external_decision_resume\",\"tool_call_id\":{},\"tool_id\":{},\"permission_id\":{}}}",
                json_string(&request.pending_tool.tool_call_id),
                json_string(&request.pending_tool.tool_id),
                json_string(&request.pending_tool.permission_id)
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    let prior_state = replayed_tool_completion_state(&session, &request.pending_tool.tool_call_id);
    if prior_state.completed && !prior_state.has_result {
        session
            .record_runtime_event(
                "agent.recovery.blocked",
                researchcode_kernel::Actor::Runtime,
                format!(
                    "{{\"reason\":\"completed_tool_missing_recorded_result\",\"tool_call_id\":{},\"tool_id\":{}}}",
                    json_string(&request.pending_tool.tool_call_id),
                    json_string(&request.pending_tool.tool_id)
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        return Ok(loop_result(
            NativeAgentLoopStatus::Blocked,
            session,
            tool_call_count,
            model_call_count,
        ));
    }
    if prior_state.completed && prior_state.has_result {
        session
            .record_runtime_event(
                "agent.resume.exactly_once_reused_recorded_result",
                researchcode_kernel::Actor::Runtime,
                format!(
                    "{{\"tool_call_id\":{},\"tool_id\":{},\"action\":\"skip_reexecution\"}}",
                    json_string(&request.pending_tool.tool_call_id),
                    json_string(&request.pending_tool.tool_id)
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        return Ok(loop_result(
            NativeAgentLoopStatus::Completed,
            session,
            tool_call_count,
            model_call_count,
        ));
    } else {
        let allowed = execute_pending_tool_after_decision(
            &mut session,
            &artifact_store,
            &request.workspace_root,
            &request.pending_tool,
            request.decision.clone(),
            None,
        )?;
        if !allowed {
            return Ok(loop_result(
                NativeAgentLoopStatus::Blocked,
                session,
                tool_call_count,
                model_call_count,
            ));
        }
        tool_call_count += 1;
    }

    session
        .start_review()
        .and_then(|_| session.complete_after_review())
        .map_err(|error| format!("{error:?}"))?;
    Ok(loop_result(
        NativeAgentLoopStatus::Completed,
        session,
        tool_call_count,
        model_call_count,
    ))
}
