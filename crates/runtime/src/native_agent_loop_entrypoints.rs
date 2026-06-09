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
    ToolIterationControlInput, TurnBudget, TurnController, TurnRoute, TurnRouter, TurnState,
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

// Layer B: turn completion helpers.

use crate::native_agent_loop::native_agent_loop_completion::stop_native_loop_with_structured_failure;
use crate::native_agent_loop::native_agent_loop_continuation::build_native_tool_evidence_continuation_request;
use crate::native_agent_loop::native_agent_loop_model_io::{
    record_native_loop_model_call_started_for_prepared_request,
    send_with_live_visible_stream_events,
};
use crate::native_agent_loop::native_agent_loop_prompt::{
    build_native_loop_tool_manifest, native_loop_continuation_hint, sanitize_http_failure_preview,
    NativeToolBatch,
};
use crate::native_agent_loop::native_agent_loop_tools::handle_native_stream_tool_event;
use crate::native_agent_loop::native_agent_loop_util::{
    json_escape, loop_result, loop_result_with_pending, planned_call_for_endpoint,
    record_native_model_http_failure_event,
};
use crate::native_agent_loop::{
    native_agent_effective_tool_exposure_for_route, run_native_agent_loop_v2_deepseek_inner,
    run_native_agent_loop_v2_deepseek_inner_with_kernel, NativeAgentLoopResult,
    NativeAgentLoopStatus, NativeAgentLoopV2Request, NativeAgentToolExposure,
    PendingNativeToolExecution,
};

pub fn run_native_agent_loop_v2_deepseek<T: LiveHttpTransport>(
    transport: &T,
    request: NativeAgentLoopV2Request,
) -> Result<NativeAgentLoopResult, String> {
    run_native_agent_loop_v2_deepseek_inner(transport, request, None, &AtomicBool::new(false))
}

pub fn run_native_agent_loop_v2_deepseek_with_event_sink<T: LiveHttpTransport>(
    transport: &T,
    request: NativeAgentLoopV2Request,
    event_sink: &mut dyn FnMut(&str),
) -> Result<NativeAgentLoopResult, String> {
    run_native_agent_loop_v2_deepseek_inner(
        transport,
        request,
        Some(event_sink),
        &AtomicBool::new(false),
    )
}

pub fn run_native_agent_loop_v2_deepseek_with_interrupt<T: LiveHttpTransport>(
    transport: &T,
    request: NativeAgentLoopV2Request,
    event_sink: Option<&mut dyn FnMut(&str)>,
    interrupt: &AtomicBool,
) -> Result<NativeAgentLoopResult, String> {
    run_native_agent_loop_v2_deepseek_inner(transport, request, event_sink, interrupt)
}

pub(crate) fn run_native_agent_loop_v2_deepseek_with_kernel_and_interrupt<T: LiveHttpTransport>(
    transport: &T,
    request: NativeAgentLoopV2Request,
    kernel_services: crate::agent_kernel::AgentKernel,
    event_sink: Option<&mut dyn FnMut(&str)>,
    interrupt: &AtomicBool,
) -> Result<NativeAgentLoopResult, String> {
    run_native_agent_loop_v2_deepseek_inner_with_kernel(
        transport,
        request,
        kernel_services,
        event_sink,
        interrupt,
    )
}
