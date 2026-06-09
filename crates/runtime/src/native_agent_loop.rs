//! Live-capable native DeepSeek/Qwen agent loop orchestration.
//!
//! This module turns the existing native model transport, parser gates,
//! permission events, ToolExecutionService, and event log into a reusable loop.
//! The HTTP boundary is injectable: tests use scripted responses, while a future
//! production path can supply a real socket transport without changing the loop.

use crate::agent_kernel::{
    AgentKernel, ContinuationStrategy, EvidenceClass, IterationOutcome, LoopStopReason,
    NativeLoopIterationContext, PermissionMode, ToolBatchGuardAction, ToolIterationControlAction,
    ToolIterationControlInput, TurnController, TurnState,
};
use crate::artifact::ArtifactStore;
use crate::context_budget::{
    allocate_native_context_budget_for_turn, DEEPSEEK_REASONING_REPLAY_BUDGET_TOKENS,
};
use crate::error_recovery::ErrorRecoveryState;
use crate::hook_dispatcher::HookDispatcher;
use crate::live_http_transport::{
    LiveHttpResponse, LiveHttpStreamEvent, LiveHttpTransport, ScriptedLiveHttpTransport,
};
use crate::live_model_executor::{
    prepare_live_model_execution, record_live_model_stream_response, LiveModelExecutionRequest,
    LiveModelStreamRecordRequest,
};
use crate::live_model_request::{
    apply_role_sampling_to_prepared_request, ModelRequestMessage, PreparedModelHttpRequest,
};
use crate::model_adapter::ModelRole;
use crate::native_profile::deepseek::adaptation::DeepSeekAdaptationManager;
use crate::native_profile::deepseek::reasoning::ReasoningReplayManager;
use crate::native_provider::NativeProviderEndpoint;
use crate::native_turn_controller::{
    estimate_tokens, NativeContextGuardAction, NativeContextGuardReport, NativeTurnController,
};
use crate::patch::stable_text_hash;
use crate::session::AgentSession;
use crate::state::AgentState;
use crate::tcml::{
    extract_json_string, visible_text_without_tool_calls, CompletedStreamingToolCall,
    ParsedToolArguments, PipelineOutcome,
};
use crate::tcml::{
    mediate_tool_call_with_provider_id, tool_manifest_generated_payload_json,
    ModelReadableToolError,
};
use crate::tool_execution::ToolExecutionArgs;
use crate::tool_orchestration::ToolCall as OrchestrationToolCall;
use crate::tool_result::{write_tool_result_artifact, ToolResultRecord};
use researchcode_kernel::model::NativeModelFamily;
use researchcode_kernel::tool::{find_tool_spec, ToolRisk};
use researchcode_kernel::{PermissionDecisionKind, PermissionRequestType};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

// Layer B: turn completion and structured-stop helpers.
#[path = "native_agent_loop_completion.rs"]
mod native_agent_loop_completion;
use self::native_agent_loop_completion::*;
// Layer B: post-tool and compacted continuation request builders.
#[path = "native_agent_loop_continuation.rs"]
mod native_agent_loop_continuation;
use self::native_agent_loop_continuation::*;
// Layer A: public native loop entrypoints.
#[path = "native_agent_loop_entrypoints.rs"]
mod native_agent_loop_entrypoints;
pub use self::native_agent_loop_entrypoints::*;
// Layer D: atomic permission and tool execution helpers.
#[path = "native_agent_loop_execution.rs"]
mod native_agent_loop_execution;
use self::native_agent_loop_execution::*;
// Test/dev fixtures: visibility unchanged until Phase 2.b.
#[path = "native_agent_loop_fixtures.rs"]
mod native_agent_loop_fixtures;
pub use self::native_agent_loop_fixtures::*;
// Layer C: model HTTP request/stream recording boundary.
#[path = "native_agent_loop_model_io.rs"]
mod native_agent_loop_model_io;
use self::native_agent_loop_model_io::*;
// Layer D: native prompt and manifest construction.
#[path = "native_agent_loop_prompt.rs"]
mod native_agent_loop_prompt;
use self::native_agent_loop_prompt::*;
pub(crate) use self::native_agent_loop_prompt::{
    native_agent_effective_tool_exposure_for_route, native_agent_tool_exposure_for_route,
};
// Layer A: external-decision resume entrypoint.
#[path = "native_agent_loop_resume.rs"]
mod native_agent_loop_resume;
pub use self::native_agent_loop_resume::*;
// Layer C: native tool batch orchestration.
#[path = "native_agent_loop_tools.rs"]
mod native_agent_loop_tools;
use self::native_agent_loop_tools::*;
// Layer Z: shared pure helpers and serialization utilities.
#[path = "native_agent_loop_util.rs"]
mod native_agent_loop_util;
use self::native_agent_loop_util::*;

const EXTERNAL_PERMISSION_NOT_ALLOWED: &str = "__native_loop_external_permission_not_allowed__";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAgentPermissionDecision {
    pub permission_id: String,
    pub decision: PermissionDecisionKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeAgentToolExposure {
    ReadOnly,
    FastAutoWrite,
    CodeEdit,
}

#[derive(Debug, Clone)]
pub struct NativeAgentLoopV2Request {
    pub project_id: String,
    pub session_id: String,
    pub task_id: String,
    pub turn_id: Option<String>,
    pub workspace_root: PathBuf,
    pub artifact_root: PathBuf,
    pub endpoint: NativeProviderEndpoint,
    pub prompt: String,
    pub max_tokens: u64,
    pub max_iterations: usize,
    pub max_tool_calls: usize,
    pub tool_exposure: NativeAgentToolExposure,
    pub permission_mode: PermissionMode,
    pub provided_permission_decisions: Vec<NativeAgentPermissionDecision>,
    pub deepseek_adaptation: Option<DeepSeekAdaptationManager>,
    pub error_recovery: Option<ErrorRecoveryState>,
    pub hook_dispatcher: Option<HookDispatcher>,
    pub concurrent_tool_execution: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeAgentLoopStatus {
    Completed,
    Blocked,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAgentLoopResult {
    pub status: NativeAgentLoopStatus,
    pub final_state: AgentState,
    pub event_count: usize,
    pub tool_call_count: usize,
    pub model_call_count: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    pub prompt_cache_hit_tokens: u64,
    pub prompt_cache_miss_tokens: u64,
    pub event_jsonl: String,
    pub pending_tool: Option<PendingNativeToolExecution>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutputTruncationRecovery {
    stop_reason: String,
    partial_visible_content: String,
    completion_tokens: u64,
    retry_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeLoopTerminalState {
    loop_id: String,
    status: NativeAgentLoopStatus,
    reason: String,
    category: String,
    iteration: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NativeLoopState {
    loop_id: String,
    current_iteration: Option<usize>,
    terminal: Option<NativeLoopTerminalState>,
}

impl NativeLoopState {
    fn new(loop_id: impl Into<String>) -> Self {
        Self {
            loop_id: loop_id.into(),
            ..Self::default()
        }
    }

    fn begin_iteration(&mut self, iteration: usize) {
        self.current_iteration = Some(iteration);
    }

    fn record_terminal(
        &mut self,
        session: &mut AgentSession,
        status: NativeAgentLoopStatus,
        reason: impl Into<String>,
        category: impl Into<String>,
    ) -> Result<(), String> {
        let reason = reason.into();
        let category = category.into();
        if let Some(existing) = &self.terminal {
            session
                .record_runtime_event(
                    "agent.loop_state.terminal_duplicate_suppressed",
                    researchcode_kernel::Actor::Runtime,
                    format!(
                        "{{\"loop_id\":{},\"existing_status\":{},\"existing_reason\":{},\"existing_category\":{},\"requested_status\":{},\"requested_reason\":{},\"requested_category\":{},\"iteration\":{}}}",
                        json_string(&existing.loop_id),
                        json_string(native_loop_status_label(&existing.status)),
                        json_string(&existing.reason),
                        json_string(&existing.category),
                        json_string(native_loop_status_label(&status)),
                        json_string(&reason),
                        json_string(&category),
                        self.current_iteration
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "null".to_string())
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
            return Ok(());
        }
        let terminal = NativeLoopTerminalState {
            loop_id: self.loop_id.clone(),
            status,
            reason,
            category,
            iteration: self.current_iteration,
        };
        session
            .record_runtime_event(
                "agent.loop_state.terminal",
                researchcode_kernel::Actor::Runtime,
                format!(
                    "{{\"loop_id\":{},\"status\":{},\"reason\":{},\"category\":{},\"iteration\":{}}}",
                    json_string(&terminal.loop_id),
                    json_string(native_loop_status_label(&terminal.status)),
                    json_string(&terminal.reason),
                    json_string(&terminal.category),
                    terminal
                        .iteration
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "null".to_string())
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        self.terminal = Some(terminal);
        Ok(())
    }
}

fn native_loop_status_label(status: &NativeAgentLoopStatus) -> &'static str {
    match status {
        NativeAgentLoopStatus::Completed => "completed",
        NativeAgentLoopStatus::Blocked => "blocked",
        NativeAgentLoopStatus::Failed => "failed",
        NativeAgentLoopStatus::Interrupted => "interrupted",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingNativeToolExecution {
    pub step_index: usize,
    pub tool_call_id: String,
    pub tool_id: String,
    pub permission_id: String,
    pub request_type: PermissionRequestType,
    pub patch_id: Option<String>,
    pub args: ToolExecutionArgs,
}

fn record_context_compaction_completed_after_guard(
    session: &mut AgentSession,
    guard_report: &NativeContextGuardReport,
    retry_call_id: &str,
    compacted_stage: &str,
    prepared: &PreparedModelHttpRequest,
) -> Result<(), String> {
    let Some(summary) = guard_report.compaction_summary.as_ref() else {
        return Ok(());
    };
    let marker = guard_report
        .compaction_marker
        .as_deref()
        .unwrap_or("[compacted-context]");
    let spine = guard_report
        .compaction_spine
        .as_ref()
        .map(|spine| spine.to_markdown())
        .unwrap_or_default();
    let spine_json = guard_report
        .compaction_spine
        .as_ref()
        .map(|spine| spine.to_json_compact_string())
        .unwrap_or_else(|| "null".to_string());
    let prompt_tokens_after_injection = estimate_tokens(&prepared.body_json);
    session
        .record_runtime_event(
            "context.compaction.completed",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"call_id\":{},\"retry_call_id\":{},\"stage\":{},\"compacted_stage\":{},\"status\":\"compacted\",\"marker\":{},\"token_estimate_before\":{},\"token_estimate_after\":{},\"prompt_tokens_after_injection\":{},\"target_limit_tokens\":{},\"compaction_reason\":{},\"spine\":{},\"spine_json\":{},\"summary\":{}}}",
                json_string(&guard_report.call_id),
                json_string(retry_call_id),
                json_string(&guard_report.stage),
                json_string(compacted_stage),
                json_string(marker),
                summary.token_estimate_before,
                summary.token_estimate_after,
                prompt_tokens_after_injection,
                guard_report.target_limit_tokens,
                json_string(&summary.compaction_reason),
                json_string(&spine),
                spine_json,
                json_string(&summary.to_markdown())
            ),
        )
        .map_err(|error| format!("{error:?}"))
}

fn native_concurrent_read_only_eligible(tool_id: &str) -> bool {
    find_tool_spec(tool_id)
        .map(|spec| spec.risk == ToolRisk::ReadOnly)
        .unwrap_or(false)
}

fn stream_stop_reason_indicates_output_truncation(stop_reason: Option<&str>) -> bool {
    let Some(reason) = stop_reason else {
        return false;
    };
    let normalized = reason.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "length"
            | "max_tokens"
            | "max_output_tokens"
            | "output_limit"
            | "output_token_limit"
            | "token_limit"
            | "model_length"
    ) || normalized.contains("max_tokens")
        || normalized.contains("max output")
        || normalized.contains("output token")
}

fn output_truncation_recovery_hint(recovery: &OutputTruncationRecovery) -> String {
    let (partial_preview, partial_truncated) =
        compact_text(&recovery.partial_visible_content, 2_400);
    format!(
        "The previous assistant output was cut off by the provider output-token limit (stop_reason={}, completion_tokens={}, recovery_attempt={}). Continue from exactly where that visible answer stopped. Do not restart from the beginning unless needed for coherence. Partial visible output{}:\n{}",
        recovery.stop_reason,
        recovery.completion_tokens,
        recovery.retry_index,
        if partial_truncated { " (truncated preview)" } else { "" },
        partial_preview
    )
}

fn prompt_with_output_truncation_recovery(
    prompt: &str,
    recovery: Option<&OutputTruncationRecovery>,
) -> String {
    let Some(recovery) = recovery else {
        return prompt.to_string();
    };
    format!(
        "{prompt}\n\n# Output Truncation Recovery\n{}",
        output_truncation_recovery_hint(recovery)
    )
}

fn combine_recovered_visible_content(
    recovery: Option<&OutputTruncationRecovery>,
    current_visible_content: &str,
) -> String {
    let Some(recovery) = recovery else {
        return current_visible_content.to_string();
    };
    let previous = recovery.partial_visible_content.trim_end();
    let current = current_visible_content.trim_start();
    if previous.is_empty() {
        return current_visible_content.to_string();
    }
    if current.is_empty() {
        return recovery.partial_visible_content.clone();
    }
    if current.starts_with(previous) {
        return current.to_string();
    }
    if previous.ends_with(current) {
        return previous.to_string();
    }
    format!("{previous}\n{current}")
}

fn record_output_truncation_recovery_event(
    session: &mut AgentSession,
    call_id: &str,
    stream_id: &str,
    stop_reason: &str,
    completion_tokens: u64,
    visible_chars: usize,
    next_max_tokens: u64,
    retry_index: u32,
) -> Result<(), String> {
    session
        .record_runtime_event(
            "agent.recovery.output_truncated",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"call_id\":{},\"stream_id\":{},\"stop_reason\":{},\"completion_tokens\":{},\"visible_chars\":{},\"next_max_tokens\":{},\"retry_index\":{},\"action\":\"continue_with_larger_output_budget\"}}",
                json_string(call_id),
                json_string(stream_id),
                json_string(stop_reason),
                completion_tokens,
                visible_chars,
                next_max_tokens,
                retry_index
            ),
        )
        .map_err(|error| format!("{error:?}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAgentLoopResumeRequest {
    pub previous_event_jsonl: String,
    pub workspace_root: PathBuf,
    pub artifact_root: PathBuf,
    pub pending_tool: PendingNativeToolExecution,
    pub decision: PermissionDecisionKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptedNativeAgentLoopFixtureResult {
    pub loop_result: NativeAgentLoopResult,
    pub final_file_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAgentLoopExternalDecisionPackage {
    pub package_dir: PathBuf,
    pub workspace_root: PathBuf,
    pub artifact_root: PathBuf,
    pub event_log_path: PathBuf,
    pub pending_tool_path: PathBuf,
    pub manifest_path: PathBuf,
    pub blocked_result: NativeAgentLoopResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAgentLoopExternalDecisionPackageResumeResult {
    pub loop_result: NativeAgentLoopResult,
    pub event_log_path: PathBuf,
    pub final_file_hash: String,
}

fn run_native_agent_loop_v2_deepseek_inner<T: LiveHttpTransport>(
    transport: &T,
    request: NativeAgentLoopV2Request,
    event_sink: Option<&mut dyn FnMut(&str)>,
    interrupt: &AtomicBool,
) -> Result<NativeAgentLoopResult, String> {
    let kernel_services = AgentKernel::for_request(&request);
    run_native_agent_loop_v2_deepseek_inner_with_kernel(
        transport,
        request,
        kernel_services,
        event_sink,
        interrupt,
    )
}

pub(crate) fn run_native_agent_loop_v2_deepseek_inner_with_kernel<T: LiveHttpTransport>(
    transport: &T,
    request: NativeAgentLoopV2Request,
    mut kernel_services: AgentKernel,
    mut event_sink: Option<&mut dyn FnMut(&str)>,
    interrupt: &AtomicBool,
) -> Result<NativeAgentLoopResult, String> {
    let artifact_store = ArtifactStore::new(&request.artifact_root);
    let mut session = AgentSession::new(&request.project_id, &request.session_id, &request.task_id)
        .map_err(|error| format!("{error:?}"))?;
    let turn_id = match request.turn_id.as_deref() {
        Some(turn_id) => turn_id.to_string(),
        None => {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| format!("{error:?}"))?
                .as_nanos();
            format!("{}:native_turn_{nonce}", request.session_id)
        }
    };
    session
        .begin_interactive_turn(&turn_id, "native_loop_v2")
        .map_err(|error| format!("{error:?}"))?;
    let hook_dispatcher = request.hook_dispatcher.as_ref();
    let mut emitted_event_count = 0usize;
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .map_err(|error| format!("{error:?}"))?;
    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
    let user_prompt_for_event = native_loop_user_prompt_for_event(&request.prompt);
    session
        .record_model_stream_delta("user_input", "user", "input", user_prompt_for_event)
        .map_err(|error| format!("{error:?}"))?;
    let turn_route = kernel_services.classify_turn(&request.prompt, None, 0);
    session
        .record_runtime_event(
            "turn.route.classified",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"turn_id\":{},\"route\":{},\"strategy\":\"deterministic_rules\"}}",
                json_string(&turn_id),
                json_string(&format!("{turn_route:?}"))
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
    let effective_tool_exposure =
        native_agent_effective_tool_exposure_for_route(&request.tool_exposure, &turn_route);
    let manifest_workflow_state = match effective_tool_exposure {
        NativeAgentToolExposure::ReadOnly => "reading",
        NativeAgentToolExposure::FastAutoWrite => "editing",
        NativeAgentToolExposure::CodeEdit => "editing",
    };
    let built_manifest = build_native_loop_tool_manifest(
        &effective_tool_exposure,
        &turn_route,
        &request.endpoint,
        manifest_workflow_state,
    );
    let manifest_allowed_tools = built_manifest
        .manifest
        .canonical_tool_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    session
        .record_runtime_event(
            "tool.manifest.generated",
            researchcode_kernel::Actor::Runtime,
            tool_manifest_generated_payload_json(&built_manifest.manifest),
        )
        .map_err(|error| format!("{error:?}"))?;
    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
    let tools_json = built_manifest.tool_schema_json;
    let plan = planned_call_for_endpoint(
        &request.endpoint,
        ModelRole::Executor,
        request.prompt.clone(),
        true,
    )?;
    let iteration_controller = kernel_services.turn_controller.clone();
    let turn_budget = iteration_controller.turn_budget_for_request(
        request.max_iterations,
        request.max_tool_calls,
        request.max_tokens,
        native_prompt_is_long_running(&request.prompt),
    );
    let effective_max_tool_calls = turn_budget.max_tool_calls as usize;
    let context_budget = allocate_native_context_budget_for_turn(
        request.endpoint.family.clone(),
        ModelRole::Executor,
        None,
        &turn_budget,
    );
    let mut turn_controller = NativeTurnController::new_for_session(&session, &request.session_id)?;
    turn_controller.record_turn_started(
        &mut session,
        &request.endpoint.family,
        turn_budget.max_iterations as usize,
        effective_max_tool_calls,
    )?;
    iteration_controller.record_tool_call_budget_normalized(
        &mut session,
        request.max_tool_calls,
        effective_max_tool_calls,
    )?;
    turn_controller.record_ledger_update(&mut session, "prepare_context")?;
    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
    let mut model_call_count = 0usize;
    let mut tool_call_count = 0usize;
    let mut turn_state = TurnState::new(request.session_id.clone(), 0);
    turn_state.budget = turn_budget;
    // Backward-compatibility batch for provider tool_result replay and legacy
    // visible fallback helpers. Novelty/convergence must not be derived from
    // this vector: OK non-duplicate results are not necessarily new
    // observations. Use EvidenceLedger plus ObservationCache distinct-key
    // growth for progress decisions.
    let mut last_tool_batch: Vec<(
        String,
        String,
        String,
        crate::tool_execution::ToolExecutionResult,
    )> = Vec::new();
    let evidence_ledger_handle = kernel_services.evidence_ledger.clone();
    let mut evidence_ledger = evidence_ledger_handle
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let convergence_enforcer = kernel_services.convergence.clone();
    let mut iteration_controller = iteration_controller;
    let mut repeated_tool_contract_failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut deepseek_cache_zone_telemetry = DeepSeekCacheZoneTelemetry::default();
    let mut reasoning_replay = request
        .deepseek_adaptation
        .as_ref()
        .map(|manager| manager.reasoning.clone())
        .unwrap_or_else(ReasoningReplayManager::default);
    let mut error_recovery = request.error_recovery.unwrap_or_default();
    let mut active_max_tokens = request
        .max_tokens
        .max(error_recovery.max_tokens.escalated_max_tokens as u64);
    let mut output_truncation_recovery: Option<OutputTruncationRecovery> = None;
    let mut deepseek_adaptation = request.deepseek_adaptation;
    let mut dual_protocol = deepseek_adaptation.as_mut().map(|da| &mut da.protocol);
    let per_iteration_tool_cap = 8usize;
    let mut native_loop_state = NativeLoopState::new(turn_id.clone());
    for iteration in 0..turn_state.budget.max_iterations as usize {
        native_loop_state.begin_iteration(iteration);
        let iteration_output_truncation_recovery = output_truncation_recovery.clone();
        let (iteration_outcome, iteration_error) = {
            let mut iteration_context = NativeLoopIterationContext {
                session: &mut session,
                turn_state: &mut turn_state,
                iteration,
                tool_call_count,
                effective_max_tool_calls,
                has_last_tool_batch: !last_tool_batch.is_empty(),
                interrupt,
                error: None,
            };
            let outcome = iteration_controller.run_iteration(&mut iteration_context);
            (outcome, iteration_context.error.take())
        };
        let iteration_ids = match iteration_outcome {
            IterationOutcome::Continue { ids } => ids,
            IterationOutcome::Interrupted => {
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Interrupted,
                    "interrupt_requested",
                    "preflight",
                )?;
                return Ok(loop_result(
                    NativeAgentLoopStatus::Interrupted,
                    session,
                    tool_call_count,
                    model_call_count,
                ));
            }
            IterationOutcome::Block { reason } => {
                if let Some(error) = iteration_error {
                    return Err(error);
                }
                let _ = reason;
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Failed,
                    "iteration_preflight_block",
                    "preflight",
                )?;
                return Ok(loop_result(
                    NativeAgentLoopStatus::Failed,
                    session,
                    tool_call_count,
                    model_call_count,
                ));
            }
            IterationOutcome::Stop {
                reason:
                    LoopStopReason::ProgressPlateau(reason) | LoopStopReason::ConvergencePlateau(reason),
            } => {
                let category = if reason.contains("max_tool_calls") {
                    "turn_budget"
                } else {
                    "progress_plateau"
                };
                stop_native_loop_with_structured_blocked(
                    &mut session,
                    &request.prompt,
                    &last_tool_batch,
                    reason,
                    category,
                    &mut emitted_event_count,
                    &mut event_sink,
                )?;
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Blocked,
                    reason,
                    category,
                )?;
                return Ok(loop_result(
                    NativeAgentLoopStatus::Blocked,
                    session,
                    tool_call_count,
                    model_call_count,
                ));
            }
        };
        let call_id = iteration_ids.call_id;
        let stream_id = iteration_ids.stream_id;
        let transcript_id = iteration_ids.transcript_id;
        let mut streamed_tool_batch: NativeToolBatch = Vec::new();
        let mut streamed_suppressed_count = 0u32;
        let mut streamed_tool_sequence = 0usize;
        let mut streamed_pending_tool: Option<PendingNativeToolExecution> = None;
        let distinct_keys_before = turn_state.observation_cache.distinct_key_count();
        let response = if !last_tool_batch.is_empty() {
            let continuation_view = continuation_view_for_batch(&evidence_ledger, &last_tool_batch);
            let continuation_batch = continuation_view.current_legacy_batch();
            let has_raw_reasoning = reasoning_replay
                .latest(&request.session_id)
                .is_some_and(|entry| !entry.raw_reasoning.trim().is_empty());
            let continuation_strategy = iteration_controller.record_continuation_plan(
                &mut session,
                &call_id,
                &request.endpoint.family,
                &request.endpoint.protocol,
                has_raw_reasoning,
                DEEPSEEK_REASONING_REPLAY_BUDGET_TOKENS,
                continuation_batch.len(),
            )?;
            let use_output_truncation_recovery = iteration_output_truncation_recovery.is_some();
            let use_plain_evidence_continuation = continuation_strategy
                == ContinuationStrategy::PlainEvidence
                || use_output_truncation_recovery;
            emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
            let mut prepared = if use_plain_evidence_continuation {
                let mut continuation_hint = native_loop_continuation_hint(
                    &request.prompt,
                    &continuation_batch,
                    &effective_tool_exposure,
                );
                if let Some(recovery) = iteration_output_truncation_recovery.as_ref() {
                    continuation_hint.push_str("\n\n");
                    continuation_hint.push_str(&output_truncation_recovery_hint(recovery));
                }
                build_native_tool_evidence_continuation_request(
                    &request.endpoint,
                    &request.prompt,
                    &continuation_view,
                    active_max_tokens,
                    &tools_json,
                    &effective_tool_exposure,
                    &continuation_hint,
                )?
            } else {
                build_native_tool_result_continuation_request(
                    &request.endpoint,
                    &request.prompt,
                    &continuation_view,
                    active_max_tokens,
                    &tools_json,
                    reasoning_replay
                        .latest(&request.session_id)
                        .map(|entry| entry.raw_reasoning.as_str()),
                )?
            };
            apply_role_sampling_to_prepared_request(
                &mut prepared,
                plan.role_model_name.as_deref(),
                plan.temperature_milli,
            )?;
            record_native_loop_model_call_started_for_prepared_request(
                &mut session,
                &call_id,
                &request.endpoint,
                &plan,
                &context_budget,
                &prepared,
                &tools_json,
            )?;
            record_deepseek_cache_zone_telemetry(
                &mut session,
                &mut deepseek_cache_zone_telemetry,
                &request.endpoint.family,
                &call_id,
                iteration,
                "continuation",
                &prepared,
            )?;
            emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
            let guard_report = guard_native_loop_prepared_request_report(
                &mut session,
                &kernel_services.context_manager,
                &request.endpoint.family,
                &context_budget,
                &call_id,
                "tool_continuation",
                &prepared,
                &mut emitted_event_count,
                &mut event_sink,
            )?;
            let mut active_call_id = call_id.clone();
            let mut active_stream_id = stream_id.clone();
            let mut active_transcript_id = transcript_id.clone();
            if guard_report.action == NativeContextGuardAction::CompactionRequired {
                let Some(summary) = guard_report.compaction_summary.as_ref() else {
                    model_call_count += 1;
                    native_loop_state.record_terminal(
                        &mut session,
                        NativeAgentLoopStatus::Blocked,
                        "context_compaction_missing_summary",
                        "context_guard",
                    )?;
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                    return Ok(loop_result(
                        NativeAgentLoopStatus::Blocked,
                        session,
                        tool_call_count,
                        model_call_count,
                    ));
                };
                let folded_reasoning_entries =
                    reasoning_replay.compact_old_reasoning(iteration as u32);
                if folded_reasoning_entries > 0 {
                    session
                        .record_runtime_event(
                            "deepseek.reasoning.compacted",
                            researchcode_kernel::Actor::Runtime,
                            format!(
                                "{{\"turn\":{},\"entries\":{},\"preserved_reasoning_count\":{},\"reason\":\"context_compaction\"}}",
                                iteration, folded_reasoning_entries, folded_reasoning_entries
                            ),
                        )
                        .map_err(|error| format!("{error:?}"))?;
                }
                active_call_id = format!("{call_id}_compacted");
                active_stream_id = format!("{stream_id}_compacted");
                active_transcript_id = format!("{transcript_id}_compacted");
                let compacted_prompt = compacted_prompt_for_model(
                    &request.prompt,
                    summary,
                    guard_report.compaction_spine.as_ref(),
                    &guard_report.compaction_preserved_messages,
                );
                let compacted_hint = "Continue after runtime context compaction. Use the compacted context and prior tool evidence; do not ask for the omitted raw transcript.";
                prepared = build_native_tool_evidence_continuation_request(
                    &request.endpoint,
                    &compacted_prompt,
                    &continuation_view,
                    active_max_tokens,
                    &tools_json,
                    &effective_tool_exposure,
                    compacted_hint,
                )?;
                apply_role_sampling_to_prepared_request(
                    &mut prepared,
                    plan.role_model_name.as_deref(),
                    plan.temperature_milli,
                )?;
                let compacted_guard = guard_native_loop_prepared_request_report(
                    &mut session,
                    &kernel_services.context_manager,
                    &request.endpoint.family,
                    &context_budget,
                    &active_call_id,
                    "compacted_tool_continuation",
                    &prepared,
                    &mut emitted_event_count,
                    &mut event_sink,
                )?;
                if !compacted_guard.should_send() {
                    model_call_count += 1;
                    native_loop_state.record_terminal(
                        &mut session,
                        NativeAgentLoopStatus::Blocked,
                        "context_guard_blocked_after_compaction",
                        "context_guard",
                    )?;
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                    return Ok(loop_result(
                        NativeAgentLoopStatus::Blocked,
                        session,
                        tool_call_count,
                        model_call_count,
                    ));
                }
                record_context_compaction_completed_after_guard(
                    &mut session,
                    &guard_report,
                    &active_call_id,
                    "compacted_tool_continuation",
                    &prepared,
                )?;
                record_native_loop_model_call_started_for_prepared_request(
                    &mut session,
                    &active_call_id,
                    &request.endpoint,
                    &plan,
                    &context_budget,
                    &prepared,
                    &tools_json,
                )?;
                session
                    .record_runtime_event(
                        "model.retry_compact_context",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"failed_call_id\":{},\"retry_call_id\":{},\"strategy\":\"rebuild_from_compacted_context\"}}",
                            json_string(&call_id),
                            json_string(&active_call_id)
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
            } else if !guard_report.should_send() {
                model_call_count += 1;
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Blocked,
                    "context_guard_blocked",
                    "context_guard",
                )?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                return Ok(loop_result(
                    NativeAgentLoopStatus::Blocked,
                    session,
                    tool_call_count,
                    model_call_count,
                ));
            }
            let call_id = active_call_id;
            let stream_id = active_stream_id;
            let transcript_id = active_transcript_id;
            evidence_ledger.begin_iteration(iteration as u32);
            let effective_permission_mode = kernel_services.permission_gate.mode();
            let mut stream_tool_handler = |session: &mut AgentSession,
                                           event: &LiveHttpStreamEvent,
                                           completed_calls: &[CompletedStreamingToolCall]|
             -> Result<(), String> {
                handle_native_stream_tool_event(
                    session,
                    event,
                    completed_calls,
                    &mut streamed_tool_batch,
                    &mut streamed_tool_sequence,
                    &mut streamed_pending_tool,
                    &artifact_store,
                    &request.workspace_root,
                    iteration,
                    &request.endpoint.family,
                    &manifest_allowed_tools,
                    &effective_tool_exposure,
                    &effective_permission_mode,
                    &request.provided_permission_decisions,
                    &mut kernel_services.permission_gate,
                    &mut turn_state.observation_cache,
                    &mut turn_controller,
                    &request.prompt,
                    hook_dispatcher,
                    &mut streamed_suppressed_count,
                )
            };
            let (http_response, record_content_deltas) = send_with_live_visible_stream_events(
                transport,
                &prepared,
                &mut session,
                &stream_id,
                &request.endpoint.family,
                &mut emitted_event_count,
                &mut event_sink,
                Some(&mut stream_tool_handler),
                None,
                None,
                interrupt,
            )?;
            if let Some(pending_tool) = streamed_pending_tool.take() {
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Blocked,
                    "pending_permission",
                    "permission",
                )?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                return Ok(loop_result_with_pending(
                    NativeAgentLoopStatus::Blocked,
                    session,
                    tool_call_count,
                    model_call_count,
                    Some(pending_tool),
                ));
            }
            if !(200..300).contains(&http_response.status_code) {
                model_call_count += 1;
                let failure_preview = sanitize_http_failure_preview(&http_response.body);
                let failure_reason = if use_plain_evidence_continuation {
                    "plain_evidence_continuation_http_failure"
                } else {
                    "tool_result_continuation_http_failure"
                };
                record_native_model_http_failure_event(
                    &mut session,
                    &call_id,
                    &request.endpoint.family,
                    http_response.status_code,
                    &failure_preview,
                    failure_reason,
                    if use_plain_evidence_continuation {
                        "stop_with_structured_failure"
                    } else {
                        "retry_plain_evidence_continuation"
                    },
                )?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                if !use_plain_evidence_continuation {
                    let retry_call_id = format!("native_loop_v2_evidence_retry_call_{iteration}");
                    let retry_stream_id =
                        format!("native_loop_v2_evidence_retry_stream_{iteration}");
                    let retry_transcript_id =
                        format!("native_loop_v2_evidence_retry_transcript_{iteration}");
                    let continuation_hint = native_loop_continuation_hint(
                        &request.prompt,
                        &continuation_batch,
                        &effective_tool_exposure,
                    );
                    let retry_hint = format!(
                    "The provider rejected the structured tool_result replay. Continue from the compact tool evidence as plain text. {continuation_hint}"
                );
                    let retry_request = build_native_tool_evidence_continuation_request(
                        &request.endpoint,
                        &request.prompt,
                        &continuation_view,
                        active_max_tokens,
                        &tools_json,
                        &effective_tool_exposure,
                        &retry_hint,
                    )?;
                    record_native_loop_model_call_started_for_prepared_request(
                        &mut session,
                        &retry_call_id,
                        &request.endpoint,
                        &plan,
                        &context_budget,
                        &retry_request,
                        &tools_json,
                    )?;
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                    session
                        .record_runtime_event(
                            "model.retry_compact_context",
                            researchcode_kernel::Actor::Runtime,
                            format!(
                                "{{\"failed_call_id\":{},\"retry_call_id\":{},\"strategy\":\"compact_runtime_evidence\"}}",
                                json_string(&call_id),
                                json_string(&retry_call_id)
                            ),
                        )
                        .map_err(|error| format!("{error:?}"))?;
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                    if !guard_native_loop_prepared_request(
                        &mut session,
                        &kernel_services.context_manager,
                        &request.endpoint.family,
                        &context_budget,
                        &retry_call_id,
                        "compact_retry_continuation",
                        &retry_request,
                        &mut emitted_event_count,
                        &mut event_sink,
                    )? {
                        model_call_count += 1;
                        native_loop_state.record_terminal(
                            &mut session,
                            NativeAgentLoopStatus::Blocked,
                            "context_guard_blocked_compact_retry_continuation",
                            "context_guard",
                        )?;
                        emit_new_session_events(
                            &session,
                            &mut emitted_event_count,
                            &mut event_sink,
                        );
                        return Ok(loop_result(
                            NativeAgentLoopStatus::Blocked,
                            session,
                            tool_call_count,
                            model_call_count,
                        ));
                    }
                    let (retry_response, retry_record_content_deltas) =
                        send_with_live_visible_stream_events(
                            transport,
                            &retry_request,
                            &mut session,
                            &retry_stream_id,
                            &request.endpoint.family,
                            &mut emitted_event_count,
                            &mut event_sink,
                            None,
                            Some(&mut error_recovery),
                            dual_protocol.as_deref_mut(),
                            interrupt,
                        )?;
                    if (200..300).contains(&retry_response.status_code) {
                        let response = record_live_model_stream_response(
                            &mut session,
                            &artifact_store,
                            LiveModelStreamRecordRequest {
                                call_id: &retry_call_id,
                                stream_id: &retry_stream_id,
                                endpoint: &request.endpoint,
                                role: ModelRole::Executor,
                                plan: &plan,
                                request_preview: native_loop_evidence_continuation_preview(
                                    &request.endpoint.family,
                                ),
                                transcript_id: &retry_transcript_id,
                                response_sse_body: &retry_response.body,
                                record_content_deltas: retry_record_content_deltas,
                            },
                        )?;
                        model_call_count += 1;
                        session
                            .record_runtime_event(
                                "model.http_failure_recovery_succeeded",
                                researchcode_kernel::Actor::Runtime,
                                format!(
                                    "{{\"failed_call_id\":{},\"retry_call_id\":{},\"strategy\":\"plain_evidence_continuation\"}}",
                                    json_string(&call_id),
                                    json_string(&retry_call_id)
                                ),
                            )
                            .map_err(|error| format!("{error:?}"))?;
                        emit_new_session_events(
                            &session,
                            &mut emitted_event_count,
                            &mut event_sink,
                        );
                        response
                    } else {
                        model_call_count += 1;
                        let retry_failure_preview =
                            sanitize_http_failure_preview(&retry_response.body);
                        record_native_model_http_failure_event(
                            &mut session,
                            &retry_call_id,
                            &request.endpoint.family,
                            retry_response.status_code,
                            &retry_failure_preview,
                            "plain_evidence_continuation_http_failure",
                            "stop_with_structured_failure",
                        )?;
                        stop_native_loop_with_structured_failure(
                            &mut session,
                            &request.prompt,
                            &continuation_batch,
                            "tool_result_continuation_http_failure",
                            "provider_failure",
                            &mut emitted_event_count,
                            &mut event_sink,
                        )?;
                        native_loop_state.record_terminal(
                            &mut session,
                            NativeAgentLoopStatus::Failed,
                            "tool_result_continuation_http_failure",
                            "provider_failure",
                        )?;
                        emit_new_session_events(
                            &session,
                            &mut emitted_event_count,
                            &mut event_sink,
                        );
                        return Ok(loop_result(
                            NativeAgentLoopStatus::Failed,
                            session,
                            tool_call_count,
                            model_call_count,
                        ));
                    }
                } else {
                    stop_native_loop_with_structured_failure(
                        &mut session,
                        &request.prompt,
                        &continuation_batch,
                        failure_reason,
                        "provider_failure",
                        &mut emitted_event_count,
                        &mut event_sink,
                    )?;
                    native_loop_state.record_terminal(
                        &mut session,
                        NativeAgentLoopStatus::Failed,
                        failure_reason,
                        "provider_failure",
                    )?;
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                    return Ok(loop_result(
                        NativeAgentLoopStatus::Failed,
                        session,
                        tool_call_count,
                        model_call_count,
                    ));
                }
            } else {
                let response = record_live_model_stream_response(
                    &mut session,
                    &artifact_store,
                    LiveModelStreamRecordRequest {
                        call_id: &call_id,
                        stream_id: &stream_id,
                        endpoint: &request.endpoint,
                        role: ModelRole::Executor,
                        plan: &plan,
                        request_preview: if use_plain_evidence_continuation {
                            native_loop_evidence_continuation_preview(&request.endpoint.family)
                        } else {
                            native_loop_continuation_preview(&request.endpoint.family)
                        },
                        transcript_id: &transcript_id,
                        response_sse_body: &http_response.body,
                        record_content_deltas,
                    },
                )?;
                model_call_count += 1;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                response
            }
        } else {
            let mut messages = vec![
                ModelRequestMessage {
                    role: "system".to_string(),
                    content: native_loop_system_prompt(
                        &request.endpoint.family,
                        &request.endpoint.protocol,
                        &effective_tool_exposure,
                        Some(&tools_json),
                        Some("current user turn and runtime context are supplied in the user message"),
                    ),
                    cache_control_ttl: None,
                },
                ModelRequestMessage {
                    role: "user".to_string(),
                    content: native_loop_prompt_with_turn_directives(
                        &prompt_with_output_truncation_recovery(
                            &request.prompt,
                            iteration_output_truncation_recovery.as_ref(),
                        ),
                        &effective_tool_exposure,
                    ),
                    cache_control_ttl: None,
                },
            ];
            if iteration_controller.should_apply_initial_cache_breakpoints(
                &request.endpoint.family,
                &request.endpoint.protocol,
            ) {
                use researchcode_kernel::model::DeepSeekVariant;
                let capabilities =
                    DeepSeekVariant::from_model_name(&request.endpoint.actual_model_name)
                        .capabilities();
                crate::native_profile::deepseek::cache_prefix::apply_cache_breakpoints_to_model_messages(
                    &mut messages,
                    &capabilities,
                );
            }
            let execution = LiveModelExecutionRequest {
                call_id: call_id.clone(),
                role: "executor".to_string(),
                endpoint: request.endpoint.clone(),
                messages,
                max_tokens: active_max_tokens,
                stream: true,
                tools_json: Some(tools_json.clone()),
                live_calls_enabled: true,
                network_approved: true,
            };
            let prepared = prepare_live_model_execution(&mut session, &execution)
                .map_err(|error| format!("{error:?}"))?;
            record_native_loop_role_call_event(
                &mut session,
                &call_id,
                &request.endpoint,
                &plan,
                "executor",
                "initial",
            )?;
            if let Some(prepared_request) = prepared.prepared_request.as_ref() {
                record_deepseek_cache_zone_telemetry(
                    &mut session,
                    &mut deepseek_cache_zone_telemetry,
                    &request.endpoint.family,
                    &call_id,
                    iteration,
                    "initial",
                    prepared_request,
                )?;
            }
            emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
            let Some(mut prepared_request) = prepared.prepared_request else {
                model_call_count += 1;
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Blocked,
                    "provider_prepare_missing_request",
                    "provider_prepare",
                )?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                return Ok(loop_result(
                    NativeAgentLoopStatus::Blocked,
                    session,
                    tool_call_count,
                    model_call_count,
                ));
            };
            apply_role_sampling_to_prepared_request(
                &mut prepared_request,
                plan.role_model_name.as_deref(),
                plan.temperature_milli,
            )?;
            let guard_report = guard_native_loop_prepared_request_report(
                &mut session,
                &kernel_services.context_manager,
                &request.endpoint.family,
                &context_budget,
                &execution.call_id,
                "initial",
                &prepared_request,
                &mut emitted_event_count,
                &mut event_sink,
            )?;
            let mut active_call_id = execution.call_id.clone();
            let mut active_stream_id = stream_id.clone();
            let mut active_transcript_id = transcript_id.clone();
            if guard_report.action == NativeContextGuardAction::CompactionRequired {
                let Some(summary) = guard_report.compaction_summary.as_ref() else {
                    model_call_count += 1;
                    native_loop_state.record_terminal(
                        &mut session,
                        NativeAgentLoopStatus::Blocked,
                        "context_compaction_missing_summary",
                        "context_guard",
                    )?;
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                    return Ok(loop_result(
                        NativeAgentLoopStatus::Blocked,
                        session,
                        tool_call_count,
                        model_call_count,
                    ));
                };
                let folded_reasoning_entries =
                    reasoning_replay.compact_old_reasoning(iteration as u32);
                if folded_reasoning_entries > 0 {
                    session
                        .record_runtime_event(
                            "deepseek.reasoning.compacted",
                            researchcode_kernel::Actor::Runtime,
                            format!(
                                "{{\"turn\":{},\"entries\":{},\"preserved_reasoning_count\":{},\"reason\":\"context_compaction\"}}",
                                iteration, folded_reasoning_entries, folded_reasoning_entries
                            ),
                        )
                        .map_err(|error| format!("{error:?}"))?;
                }
                active_call_id = format!("{}_compacted", execution.call_id);
                active_stream_id = format!("{stream_id}_compacted");
                active_transcript_id = format!("{transcript_id}_compacted");
                prepared_request = build_native_compacted_initial_request(
                    &request.endpoint,
                    &request.prompt,
                    summary,
                    guard_report.compaction_spine.as_ref(),
                    &guard_report.compaction_preserved_messages,
                    active_max_tokens,
                    &tools_json,
                    &effective_tool_exposure,
                )?;
                let compacted_guard = guard_native_loop_prepared_request_report(
                    &mut session,
                    &kernel_services.context_manager,
                    &request.endpoint.family,
                    &context_budget,
                    &active_call_id,
                    "compacted_initial",
                    &prepared_request,
                    &mut emitted_event_count,
                    &mut event_sink,
                )?;
                if !compacted_guard.should_send() {
                    model_call_count += 1;
                    native_loop_state.record_terminal(
                        &mut session,
                        NativeAgentLoopStatus::Blocked,
                        "context_guard_blocked_after_compaction",
                        "context_guard",
                    )?;
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                    return Ok(loop_result(
                        NativeAgentLoopStatus::Blocked,
                        session,
                        tool_call_count,
                        model_call_count,
                    ));
                }
                record_context_compaction_completed_after_guard(
                    &mut session,
                    &guard_report,
                    &active_call_id,
                    "compacted_initial",
                    &prepared_request,
                )?;
                record_native_loop_model_call_started_for_prepared_request(
                    &mut session,
                    &active_call_id,
                    &request.endpoint,
                    &plan,
                    &context_budget,
                    &prepared_request,
                    &tools_json,
                )?;
                session
                    .record_runtime_event(
                        "model.retry_compact_context",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"failed_call_id\":{},\"retry_call_id\":{},\"strategy\":\"rebuild_initial_from_compacted_context\"}}",
                            json_string(&execution.call_id),
                            json_string(&active_call_id)
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
            } else if !guard_report.should_send() {
                model_call_count += 1;
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Blocked,
                    "context_guard_blocked",
                    "context_guard",
                )?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                return Ok(loop_result(
                    NativeAgentLoopStatus::Blocked,
                    session,
                    tool_call_count,
                    model_call_count,
                ));
            }
            let call_id = active_call_id;
            let stream_id = active_stream_id;
            let transcript_id = active_transcript_id;
            evidence_ledger.begin_iteration(iteration as u32);
            let effective_permission_mode = kernel_services.permission_gate.mode();
            let mut stream_tool_handler = |session: &mut AgentSession,
                                           event: &LiveHttpStreamEvent,
                                           completed_calls: &[CompletedStreamingToolCall]|
             -> Result<(), String> {
                handle_native_stream_tool_event(
                    session,
                    event,
                    completed_calls,
                    &mut streamed_tool_batch,
                    &mut streamed_tool_sequence,
                    &mut streamed_pending_tool,
                    &artifact_store,
                    &request.workspace_root,
                    iteration,
                    &request.endpoint.family,
                    &manifest_allowed_tools,
                    &effective_tool_exposure,
                    &effective_permission_mode,
                    &request.provided_permission_decisions,
                    &mut kernel_services.permission_gate,
                    &mut turn_state.observation_cache,
                    &mut turn_controller,
                    &request.prompt,
                    hook_dispatcher,
                    &mut streamed_suppressed_count,
                )
            };
            let (http_response, record_content_deltas) = match send_with_live_visible_stream_events(
                transport,
                &prepared_request,
                &mut session,
                &stream_id,
                &request.endpoint.family,
                &mut emitted_event_count,
                &mut event_sink,
                Some(&mut stream_tool_handler),
                None,
                None,
                interrupt,
            ) {
                Ok(response) => response,
                Err(error) if native_prompt_wants_file_generation(&request.prompt) => {
                    model_call_count += 1;
                    session
                        .record_runtime_event(
                            "agent.write_progress.blocked",
                            researchcode_kernel::Actor::Runtime,
                            format!(
                                "{{\"iteration\":{},\"reason\":\"model_transport_error_before_write\",\"action\":\"stop_without_runtime_write_fallback\",\"error\":{}}}",
                                iteration,
                                json_string(&error)
                            ),
                        )
                        .map_err(|error| format!("{error:?}"))?;
                    stop_native_loop_with_structured_failure(
                        &mut session,
                        &request.prompt,
                        &last_tool_batch,
                        "model_transport_error_before_write",
                        "provider_failure",
                        &mut emitted_event_count,
                        &mut event_sink,
                    )?;
                    native_loop_state.record_terminal(
                        &mut session,
                        NativeAgentLoopStatus::Failed,
                        "model_transport_error_before_write",
                        "provider_failure",
                    )?;
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                    return Ok(loop_result(
                        NativeAgentLoopStatus::Failed,
                        session,
                        tool_call_count,
                        model_call_count,
                    ));
                }
                Err(error) => return Err(error),
            };
            model_call_count += 1;
            if let Some(pending_tool) = streamed_pending_tool.take() {
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Blocked,
                    "pending_permission",
                    "permission",
                )?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                return Ok(loop_result_with_pending(
                    NativeAgentLoopStatus::Blocked,
                    session,
                    tool_call_count,
                    model_call_count,
                    Some(pending_tool),
                ));
            }
            if !(200..300).contains(&http_response.status_code) {
                session
                    .record_model_call_blocked(
                        &execution.call_id,
                        native_loop_provider_label(&request.endpoint.family),
                        format!("http_status_{}", http_response.status_code),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                stop_native_loop_with_structured_failure(
                    &mut session,
                    &request.prompt,
                    &last_tool_batch,
                    "initial_model_http_failure",
                    "provider_failure",
                    &mut emitted_event_count,
                    &mut event_sink,
                )?;
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Failed,
                    "initial_model_http_failure",
                    "provider_failure",
                )?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                return Ok(loop_result(
                    NativeAgentLoopStatus::Failed,
                    session,
                    tool_call_count,
                    model_call_count,
                ));
            }
            let response = record_live_model_stream_response(
                &mut session,
                &artifact_store,
                LiveModelStreamRecordRequest {
                    call_id: &call_id,
                    stream_id: &stream_id,
                    endpoint: &request.endpoint,
                    role: ModelRole::Executor,
                    plan: &plan,
                    request_preview: "native loop v2 initial request",
                    transcript_id: &transcript_id,
                    response_sse_body: &http_response.body,
                    record_content_deltas,
                },
            )?;
            emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
            response
        };
        if request.endpoint.family == NativeModelFamily::DeepSeek {
            if let Some(reasoning_content) = response
                .volatile_reasoning_content
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                reasoning_replay.capture_raw_response(
                    &request.session_id,
                    turn_state.iterations,
                    &call_id,
                    reasoning_content,
                );
            }
        }
        let output_truncated =
            stream_stop_reason_indicates_output_truncation(response.stop_reason.as_deref());
        if !output_truncated && iteration_output_truncation_recovery.is_some() {
            output_truncation_recovery = None;
        }
        if contains_executable_dsml_markup(&response.visible_content_preview) {
            iteration_controller.record_executable_dsml_fallback(
                &mut session,
                iteration,
                response.visible_content_preview.len(),
                &request.endpoint.family,
            )?;
            emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
        }
        let tool_calls = match kernel_services
            .tcml
            .process_text(&response.visible_content_preview)
        {
            PipelineOutcome::ParsedCalls(calls) => calls,
            PipelineOutcome::NoToolCall | PipelineOutcome::StreamingCalls(_) => Vec::new(),
        };
        if !tool_calls.is_empty() {
            iteration_controller.record_visible_tool_call_batch_assembled(
                &mut session,
                &request.endpoint.family,
                iteration,
                "visible_content_parse",
                tool_calls.len(),
                response.visible_content_preview.len(),
            )?;
            emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
        }
        // Use streamed tool results when available — prefer real execution results
        // over post-hoc parsed calls to avoid double execution.
        if !streamed_tool_batch.is_empty() {
            let had_stream_parse_mismatch = streamed_tool_batch.len() != tool_calls.len();
            let mismatch_error_count = if !had_stream_parse_mismatch {
                0
            } else {
                append_stream_mismatch_error_results(
                    &mut session,
                    &artifact_store,
                    iteration,
                    &mut streamed_tool_batch,
                    &tool_calls,
                )?
            };
            // Size mismatch: use real streamed executions for matched calls and
            // synthetic model-readable errors for parsed calls that did not
            // produce a streamed result. This preserves exactly-once tool_result
            // pairing without re-running side-effecting tools.
            iteration_controller.record_streaming_batch_ready(
                &mut session,
                iteration,
                streamed_tool_batch.len(),
                tool_calls.len(),
                mismatch_error_count,
                had_stream_parse_mismatch,
            )?;
            emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
            tool_call_count += streamed_tool_batch
                .len()
                .saturating_sub(streamed_suppressed_count as usize);
            replace_native_tool_batch_from_legacy(
                &mut evidence_ledger,
                &mut last_tool_batch,
                streamed_tool_batch,
            );
            for _ in 0..streamed_suppressed_count {
                evidence_ledger.record_suppressed();
            }
            let progress_error_results = last_tool_batch
                .iter()
                .filter(|(_, _, _, result)| !result.ok)
                .count() as u32;
            let iteration_ok_results = last_tool_batch
                .iter()
                .filter(|(_, _, _, result)| result.ok)
                .count();
            let iteration_error_results = progress_error_results as usize;
            let repeated_error_signatures = last_tool_batch
                .iter()
                .filter(|(_, _, _, result)| !result.ok)
                .filter_map(|(_, _, _, result)| {
                    model_readable_error_signature(result)
                        .map(|signature| (result.tool_id.clone(), signature))
                })
                .collect::<Vec<_>>();
            let seen_tool_batches_snapshot = turn_state.seen_tool_batches.clone();
            match iteration_controller.observe_completed_tool_iteration(
                &mut session,
                &mut turn_state,
                &convergence_enforcer,
                &evidence_ledger,
                ToolIterationControlInput {
                    iteration,
                    ok_results: iteration_ok_results,
                    error_results: iteration_error_results,
                    duplicate_results: streamed_suppressed_count,
                    distinct_keys_before,
                    progress_error_results,
                    repeated_error_signatures,
                    batch_signature: "streamed_tool_batch",
                    seen_tool_batches: &seen_tool_batches_snapshot,
                },
            )? {
                ToolIterationControlAction::Continue => {}
                ToolIterationControlAction::SoftWarning => {
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                }
                ToolIterationControlAction::Stop { reason } => {
                    if (native_prompt_wants_file_generation(&request.prompt)
                        || native_prompt_wants_write_or_edit(&request.prompt))
                        && !last_tool_batch.iter().any(|(_, tool_id, _, result)| {
                            result.ok
                                && matches!(
                                    tool_id.as_str(),
                                    "file.write" | "file.edit" | "file.multi_edit" | "patch.apply"
                                )
                        })
                    {
                        session
                            .record_runtime_event(
                                "agent.write_progress.blocked",
                                researchcode_kernel::Actor::Runtime,
                                format!(
                                    "{{\"iteration\":{},\"reason\":\"write_task_stopped_without_state_change\",\"original_reason\":{}}}",
                                    iteration,
                                    json_string(reason)
                                ),
                            )
                            .map_err(|error| format!("{error:?}"))?;
                    }
                    stop_native_loop_with_structured_blocked(
                        &mut session,
                        &request.prompt,
                        &last_tool_batch,
                        reason,
                        "streaming_tool_iteration_stop",
                        &mut emitted_event_count,
                        &mut event_sink,
                    )?;
                    native_loop_state.record_terminal(
                        &mut session,
                        NativeAgentLoopStatus::Blocked,
                        reason,
                        "streaming_tool_iteration_stop",
                    )?;
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                    return Ok(loop_result(
                        NativeAgentLoopStatus::Blocked,
                        session,
                        tool_call_count,
                        model_call_count,
                    ));
                }
            }
            continue;
        }
        if output_truncated {
            let visible_text = visible_text_without_tool_calls(&response.visible_content_preview);
            let next_max_tokens = if error_recovery.max_tokens.escalation_level == 0 {
                error_recovery.max_tokens.escalate() as u64
            } else {
                error_recovery.max_tokens.record_retry();
                error_recovery
                    .max_tokens
                    .escalated_max_tokens
                    .max(request.max_tokens as u32) as u64
            };
            active_max_tokens = active_max_tokens.max(next_max_tokens);
            let retry_index = error_recovery.max_tokens.recovery_retries + 1;
            let stop_reason = response
                .stop_reason
                .as_deref()
                .unwrap_or("output_token_limit")
                .to_string();
            record_output_truncation_recovery_event(
                &mut session,
                &call_id,
                &stream_id,
                &stop_reason,
                response.completion_tokens,
                visible_text.chars().count(),
                active_max_tokens,
                retry_index,
            )?;
            emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
            output_truncation_recovery = Some(OutputTruncationRecovery {
                stop_reason,
                partial_visible_content: combine_recovered_visible_content(
                    iteration_output_truncation_recovery.as_ref(),
                    &visible_text,
                ),
                completion_tokens: response.completion_tokens,
                retry_index,
            });
            continue;
        }
        if tool_calls.is_empty() {
            let visible_content_for_answer = combine_recovered_visible_content(
                iteration_output_truncation_recovery.as_ref(),
                &response.visible_content_preview,
            );
            let visible_text = visible_text_without_tool_calls(&visible_content_for_answer);
            if !visible_text.trim().is_empty() {
                // Keep transition-statement detection as telemetry only. Older
                // builds used this string heuristic to force another loop
                // iteration, which made ordinary inter-tool narration act like
                // hidden control flow. The provider's structured tool calls are
                // the only authority for continuing tool execution here.
                let prior_tool_work = tool_call_count > 0 || !last_tool_batch.is_empty();
                if prior_tool_work && visible_text_looks_like_transition_statement(&visible_text) {
                    session
                        .record_runtime_event(
                            "agent.visible_only_transition_detected",
                            researchcode_kernel::Actor::Runtime,
                            format!(
                                "{{\"iteration\":{},\"tool_call_count\":{},\"action\":\"telemetry_only\",\"loop_control\":false}}",
                                iteration, tool_call_count
                            ),
                        )
                        .map_err(|error| format!("{error:?}"))?;
                }
                if !record_visible_assistant_message(
                    &mut session,
                    &visible_content_for_answer,
                    "model_visible_answer",
                )? {
                    stop_native_loop_with_structured_failure(
                        &mut session,
                        &request.prompt,
                        &last_tool_batch,
                        "model_visible_answer_rejected",
                        "no_visible_answer",
                        &mut emitted_event_count,
                        &mut event_sink,
                    )?;
                    native_loop_state.record_terminal(
                        &mut session,
                        NativeAgentLoopStatus::Failed,
                        "model_visible_answer_rejected",
                        "no_visible_answer",
                    )?;
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                    return Ok(loop_result(
                        NativeAgentLoopStatus::Failed,
                        session,
                        tool_call_count,
                        model_call_count,
                    ));
                }
                turn_controller.ensure_can_complete(&mut session, "model_visible_answer")?;
                record_native_loop_turn_summary(
                    &mut session,
                    &request.prompt,
                    &last_tool_batch,
                    completion_status_from_batch(&last_tool_batch),
                );
                session
                    .start_review()
                    .and_then(|_| session.complete_after_review())
                    .map_err(|error| format!("{error:?}"))?;
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Completed,
                    "model_visible_answer",
                    "model_answer",
                )?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                return Ok(loop_result(
                    NativeAgentLoopStatus::Completed,
                    session,
                    tool_call_count,
                    model_call_count,
                ));
            }
            iteration_controller.record_empty_visible_recovery(&mut session, iteration)?;
            stop_native_loop_with_structured_failure(
                &mut session,
                &request.prompt,
                &last_tool_batch,
                "empty_visible_response",
                "no_visible_answer",
                &mut emitted_event_count,
                &mut event_sink,
            )?;
            native_loop_state.record_terminal(
                &mut session,
                NativeAgentLoopStatus::Failed,
                "empty_visible_response",
                "no_visible_answer",
            )?;
            emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
            return Ok(loop_result(
                NativeAgentLoopStatus::Failed,
                session,
                tool_call_count,
                model_call_count,
            ));
        }
        let batch_signature = iteration_controller.tool_batch_signature(&tool_calls);
        let batch_status = iteration_controller
            .classify_tool_batch_signature(&turn_state.seen_tool_batches, &batch_signature);
        let repeated_cached_observation_batch = batch_status.repeated_tool_batch
            && tool_calls_are_cached_observations(
                &tool_calls,
                &mut turn_state.observation_cache,
                &manifest_allowed_tools,
                per_iteration_tool_cap,
                &request.workspace_root,
            );
        match kernel_services.tool_orchestration.observe_batch_guard(
            &mut iteration_controller,
            &mut session,
            &mut turn_state,
            iteration,
            batch_signature.clone(),
            batch_status,
            repeated_cached_observation_batch,
        )? {
            ToolBatchGuardAction::Stop { reason } => {
                stop_native_loop_with_structured_blocked(
                    &mut session,
                    &request.prompt,
                    &last_tool_batch,
                    reason,
                    "duplicate_observation_plateau",
                    &mut emitted_event_count,
                    &mut event_sink,
                )?;
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Blocked,
                    reason,
                    "duplicate_observation_plateau",
                )?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                return Ok(loop_result(
                    NativeAgentLoopStatus::Blocked,
                    session,
                    tool_call_count,
                    model_call_count,
                ));
            }
            ToolBatchGuardAction::UseSyntheticRecovery => {
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                let synthetic_batch = tool_calls
                    .iter()
                    .take(per_iteration_tool_cap)
                    .enumerate()
                    .map(|(tool_index, tool)| {
                        let provider_tool_call_id = model_provider_tool_call_id(
                            tool,
                            format!("toolu_v2_loop_guard_{iteration}_{tool_index}"),
                        );
                        let mediated = mediate_tool_call_with_provider_id(
                            &tool.tool_id,
                            Some(&provider_tool_call_id),
                            &tool.arguments_json,
                        );
                        let tool_id = mediated.tool_id.clone();
                        let arguments_json = if mediated.error.is_none() {
                            mediated.arguments_json.clone()
                        } else {
                            tool.arguments_json.clone()
                        };
                        (
                            provider_tool_call_id,
                            tool_id.clone(),
                            arguments_json.clone(),
                            native_loop_synthetic_tool_error(
                                iteration,
                                tool_index,
                                &tool_id,
                                &arguments_json,
                                &request.prompt,
                            ),
                        )
                    })
                    .collect();
                replace_native_tool_batch_from_legacy(
                    &mut evidence_ledger,
                    &mut last_tool_batch,
                    synthetic_batch,
                );
                continue;
            }
            ToolBatchGuardAction::Continue => {
                if repeated_cached_observation_batch {
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                }
            }
        }
        // Keep provider continuations scoped to the immediately preceding
        // assistant tool-use batch. Accumulating older tool results makes each
        // continuation replay stale evidence and encourages browse/read loops.
        last_tool_batch.clear();
        evidence_ledger.clear();
        iteration_controller.reset_loop_guard_recovery();
        let mut iteration_ok_results = 0usize;
        let mut iteration_error_results = 0usize;
        let mut duplicate_suppressed_count = streamed_suppressed_count;
        for _ in 0..streamed_suppressed_count {
            evidence_ledger.record_suppressed();
        }
        let mut concurrent_handled_indices: std::collections::HashSet<usize> =
            std::collections::HashSet::new();
        // Concurrent execution: when enabled, partition concurrent-safe read-only tools
        // into batches and execute each batch in parallel.
        if request.concurrent_tool_execution
            && matches!(effective_tool_exposure, NativeAgentToolExposure::ReadOnly)
        {
            let num_tools = tool_calls.len().min(per_iteration_tool_cap);
            if num_tools > 1 {
                // Build orchestration calls matching 1:1 with tool_calls[0..num_tools].
                // Even read-only concurrent batches must enter through TCML so aliases,
                // schema repairs, relational defaults, and canonical evidence stay aligned
                // with the serial tool path.
                let mut orchestration_calls: Vec<(
                    usize,
                    OrchestrationToolCall,
                    String,
                    ParsedToolArguments,
                )> = Vec::new();
                for (idx, pt) in tool_calls.iter().take(per_iteration_tool_cap).enumerate() {
                    let candidate_tool_id = crate::tcml::canonical_tool_id(&pt.tool_id);
                    if !native_concurrent_read_only_eligible(&candidate_tool_id) {
                        break;
                    }
                    let provider_tool_call_id =
                        model_provider_tool_call_id(pt, format!("toolu_v2_conc_{iteration}_{idx}"));
                    let mediated = mediate_tool_call_with_provider_id(
                        &pt.tool_id,
                        Some(&provider_tool_call_id),
                        &pt.arguments_json,
                    );
                    for event in &mediated.events {
                        session
                            .record_runtime_event(
                                &event.event_type,
                                researchcode_kernel::Actor::Runtime,
                                event.payload_json.clone(),
                            )
                            .map_err(|error| format!("{error:?}"))?;
                    }
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                    let tool_id = mediated.tool_id.clone();
                    let arguments = mediated.arguments.clone();
                    let canonical_args_json = canonical_json_text(&tool_args_json(&arguments));

                    if let Some(error) = mediated.error.clone() {
                        let result = execute_model_readable_error_collect(
                            &mut session,
                            &artifact_store,
                            iteration * 10 + idx,
                            Some(&provider_tool_call_id),
                            &pt.tool_id,
                            &error,
                        )?;
                        session
                            .record_runtime_event(
                                "agent.loop_recovery",
                                researchcode_kernel::Actor::Runtime,
                                format!(
                                    "{{\"iteration\":{},\"tool_id\":\"{}\",\"reason\":\"tool_contract_rejected_concurrent\",\"preview\":\"{}\",\"detail\":{}}}",
                                    iteration,
                                    json_escape(&pt.tool_id),
                                    json_escape(&result.preview),
                                    safe_json_fragment(&result.detail_json)
                                ),
                            )
                            .map_err(|error| format!("{error:?}"))?;
                        emit_new_session_events(
                            &session,
                            &mut emitted_event_count,
                            &mut event_sink,
                        );
                        turn_controller.record_recovery_started(
                            &mut session,
                            "tool_contract_rejected",
                            iteration,
                        )?;
                        record_native_tool_batch_item(
                            &mut evidence_ledger,
                            &mut last_tool_batch,
                            provider_tool_call_id.clone(),
                            result.tool_id.clone(),
                            canonical_args_json,
                            result,
                            EvidenceClass::Error,
                        );
                        iteration_error_results += 1;
                        tool_call_count += 1;
                        concurrent_handled_indices.insert(idx);
                        continue;
                    }

                    if turn_state.observation_cache.contains_in_workspace(
                        &tool_id,
                        &arguments,
                        &request.workspace_root,
                    ) {
                        let cache_key = turn_state
                            .observation_cache
                            .check_and_record_in_workspace(
                                &tool_id,
                                &arguments,
                                &request.workspace_root,
                            )
                            .unwrap_or_else(|| {
                                crate::agent_kernel::observation_key(&tool_id, &arguments)
                                    .unwrap_or_else(|| tool_id.clone())
                            });
                        let result = execute_duplicate_observation_collect(
                            &mut session,
                            &artifact_store,
                            iteration * 10 + idx,
                            Some(&provider_tool_call_id),
                            &tool_id,
                            &canonical_args_json,
                            &cache_key,
                        )?;
                        duplicate_suppressed_count += 1;
                        record_native_tool_batch_item(
                            &mut evidence_ledger,
                            &mut last_tool_batch,
                            provider_tool_call_id.clone(),
                            tool_id.clone(),
                            canonical_args_json,
                            result,
                            EvidenceClass::Recovery,
                        );
                        concurrent_handled_indices.insert(idx);
                        emit_new_session_events(
                            &session,
                            &mut emitted_event_count,
                            &mut event_sink,
                        );
                        continue;
                    }
                    orchestration_calls.push((
                        idx,
                        OrchestrationToolCall {
                            tool_call_id: provider_tool_call_id,
                            tool_id,
                            args: tool_args(&arguments),
                        },
                        canonical_args_json,
                        arguments,
                    ));
                }
                let calls: Vec<OrchestrationToolCall> = orchestration_calls
                    .iter()
                    .map(|(_, c, _, _)| c.clone())
                    .collect();
                let batches = kernel_services
                    .tool_orchestration
                    .partition_tool_batch(&calls);
                let mut batch_offset = 0usize;
                for batch in &batches {
                    if batch.tools.len() <= 1 {
                        batch_offset += batch.tools.len();
                        continue;
                    }
                    let conc_results = execute_concurrent_read_only_batch(
                        &mut session,
                        &artifact_store,
                        &request.workspace_root,
                        batch,
                        iteration,
                        hook_dispatcher,
                    )?;
                    for (local_idx, (tool_use_id, tool_id, args_json, result)) in
                        conc_results.into_iter().enumerate()
                    {
                        let ledger_id = format!("native_loop_v2_conc_ledger_{iteration}_{tool_id}");
                        turn_controller.record_tool_completed(
                            &mut session,
                            &ledger_id,
                            &tool_id,
                            result.ok,
                        )?;
                        if result.ok {
                            iteration_ok_results += 1;
                        } else {
                            iteration_error_results += 1;
                        }
                        tool_call_count += 1;
                        let class = ledger_class_for_tool_result(&result);
                        record_native_tool_batch_item(
                            &mut evidence_ledger,
                            &mut last_tool_batch,
                            tool_use_id,
                            tool_id,
                            orchestration_calls
                                .get(batch_offset + local_idx)
                                .map(|(_, _, canonical_args, _)| canonical_args.clone())
                                .unwrap_or(args_json),
                            result,
                            class,
                        );
                        // Mark original tool_calls index as handled
                        let orig_idx = batch_offset + local_idx;
                        if let Some((idx, call, _, arguments)) = orchestration_calls.get(orig_idx) {
                            let _ = turn_state.observation_cache.check_and_record_in_workspace(
                                &call.tool_id,
                                arguments,
                                &request.workspace_root,
                            );
                            concurrent_handled_indices.insert(*idx);
                        }
                    }
                    batch_offset += batch.tools.len();
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                }
                let concurrent_tool_count: usize = batches
                    .iter()
                    .filter(|b| b.tools.len() > 1)
                    .map(|b| b.tools.len())
                    .sum();
                if concurrent_tool_count >= num_tools {
                    continue;
                }
            }
        }
        for (tool_index, parsed_tool) in tool_calls.iter().take(per_iteration_tool_cap).enumerate()
        {
            if concurrent_handled_indices.contains(&tool_index) {
                continue;
            }
            let provider_tool_call_id = model_provider_tool_call_id(
                parsed_tool,
                format!("toolu_v2_{iteration}_{tool_index}"),
            );
            let mediated = mediate_tool_call_with_provider_id(
                &parsed_tool.tool_id,
                Some(&provider_tool_call_id),
                &parsed_tool.arguments_json,
            );
            for event in &mediated.events {
                session
                    .record_runtime_event(
                        &event.event_type,
                        researchcode_kernel::Actor::Runtime,
                        event.payload_json.clone(),
                    )
                    .map_err(|error| format!("{error:?}"))?;
            }
            emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
            let tool_id = mediated.tool_id.clone();
            let arguments = mediated.arguments.clone();
            let ledger_tool_call_id = format!("native_loop_v2_ledger_{iteration}_{tool_index}");
            if !manifest_allowed_tools.contains(&tool_id) {
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
                    suggested_replacement: suggested_manifest_tool(
                        &manifest_allowed_tools,
                        &tool_id,
                    ),
                };
                let result = execute_model_readable_error_collect(
                    &mut session,
                    &artifact_store,
                    iteration * 10 + tool_index,
                    Some(&provider_tool_call_id),
                    &parsed_tool.tool_id,
                    &error,
                )?;
                session
                    .record_runtime_event(
                        "agent.loop_recovery",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"iteration\":{},\"tool_id\":\"{}\",\"reason\":\"tool_not_in_manifest\",\"preview\":\"{}\",\"detail\":{}}}",
                            iteration,
                            json_escape(&parsed_tool.tool_id),
                            json_escape(&result.preview),
                            safe_json_fragment(&result.detail_json)
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                turn_controller.record_recovery_started(
                    &mut session,
                    "tool_not_in_manifest",
                    iteration,
                )?;
                record_native_tool_batch_item(
                    &mut evidence_ledger,
                    &mut last_tool_batch,
                    provider_tool_call_id.clone(),
                    result.tool_id.clone(),
                    mediated.arguments_json.clone(),
                    result,
                    EvidenceClass::Error,
                );
                iteration_error_results += 1;
                tool_call_count += 1;
                continue;
            }
            if let Some(mut error) = mediated.error.clone() {
                let failure_signature = tool_contract_failure_signature(&mediated, &error);
                let repeated_failure_count = repeated_tool_contract_failures
                    .entry(failure_signature.clone())
                    .and_modify(|count| *count += 1)
                    .or_insert(1);
                let repeated_failure_count = *repeated_failure_count;
                if repeated_failure_count >= 3 {
                    error = escalated_model_readable_tool_error(&mediated, &error);
                    session
                        .record_runtime_event(
                            "agent.recovery.escalated",
                            researchcode_kernel::Actor::Runtime,
                            format!(
                                "{{\"iteration\":{},\"tool_id\":{},\"requested_tool\":{},\"reason\":\"repeated_tool_contract_rejected\",\"failure_signature\":{},\"failure_count\":{},\"error_code\":{},\"retryable\":false,\"guidance\":{}}}",
                                iteration,
                                json_string(&mediated.tool_id),
                                json_string(&mediated.requested_tool_id),
                                json_string(&failure_signature),
                                repeated_failure_count,
                                json_string(&error.error_code),
                                json_string(&error.short_message)
                            ),
                        )
                        .map_err(|error| format!("{error:?}"))?;
                    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                }
                let result = execute_model_readable_error_collect(
                    &mut session,
                    &artifact_store,
                    iteration * 10 + tool_index,
                    Some(&provider_tool_call_id),
                    &parsed_tool.tool_id,
                    &error,
                )?;
                session
                    .record_runtime_event(
                        "agent.loop_recovery",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"iteration\":{},\"tool_id\":\"{}\",\"reason\":\"tool_contract_rejected\",\"preview\":\"{}\",\"detail\":{}}}",
                            iteration,
                            json_escape(&parsed_tool.tool_id),
                            json_escape(&result.preview),
                            safe_json_fragment(&result.detail_json)
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                turn_controller.record_recovery_started(
                    &mut session,
                    "tool_contract_rejected",
                    iteration,
                )?;
                record_native_tool_batch_item(
                    &mut evidence_ledger,
                    &mut last_tool_batch,
                    provider_tool_call_id.clone(),
                    result.tool_id.clone(),
                    mediated.arguments_json.clone(),
                    result,
                    EvidenceClass::Error,
                );
                iteration_error_results += 1;
                tool_call_count += 1;
                continue;
            }
            turn_controller.record_tool_pending(
                &mut session,
                &ledger_tool_call_id,
                &tool_id,
                iteration,
            )?;
            emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
            if tool_id == "plan.enter" {
                let tool_call_id = format!("native_loop_v2_tool_{iteration}_{tool_index}");
                let plan_approval_id = format!("{tool_call_id}_plan_approval");
                record_tool_call_requested_preserving_provider_id(
                    &mut session,
                    &tool_call_id,
                    Some(&provider_tool_call_id),
                    &tool_id,
                )?;
                record_tool_call_completed_preserving_provider_id(
                    &mut session,
                    &tool_call_id,
                    Some(&provider_tool_call_id),
                    &tool_id,
                    true,
                )?;
                turn_controller.record_tool_completed(
                    &mut session,
                    &ledger_tool_call_id,
                    &tool_id,
                    true,
                )?;
                session
                    .record_runtime_event(
                        "plan.mode_entered",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"plan_approval_id\":\"{}\",\"tool_call_id\":\"{}\",\"plan_preview\":\"{}\"}}",
                            json_escape(&plan_approval_id),
                            json_escape(&tool_call_id),
                            json_escape(
                                &arguments
                                    .content
                                    .clone()
                                    .unwrap_or_else(|| "Plan approval requested by model.".to_string())
                            )
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                session
                    .request_plan_approval(plan_approval_id, arguments.content.clone())
                    .map_err(|error| format!("{error:?}"))?;
                turn_controller.record_ledger_update(&mut session, "plan_approval_pending")?;
                tool_call_count += 1;
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Blocked,
                    "plan_approval_pending",
                    "plan_approval",
                )?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                return Ok(loop_result(
                    NativeAgentLoopStatus::Blocked,
                    session,
                    tool_call_count,
                    model_call_count,
                ));
            }
            if tool_id == "plan.exit" {
                let tool_call_id = format!("native_loop_v2_tool_{iteration}_{tool_index}");
                record_tool_call_requested_preserving_provider_id(
                    &mut session,
                    &tool_call_id,
                    Some(&provider_tool_call_id),
                    &tool_id,
                )?;
                record_tool_call_completed_preserving_provider_id(
                    &mut session,
                    &tool_call_id,
                    Some(&provider_tool_call_id),
                    &tool_id,
                    true,
                )?;
                turn_controller.record_tool_completed(
                    &mut session,
                    &ledger_tool_call_id,
                    &tool_id,
                    true,
                )?;
                session
                    .record_runtime_event(
                        "plan.mode_exited",
                        researchcode_kernel::Actor::Runtime,
                        format!("{{\"tool_call_id\":\"{}\"}}", json_escape(&tool_call_id)),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                tool_call_count += 1;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                continue;
            }
            if tool_id == "ask_user" {
                let tool_call_id = format!("native_loop_v2_tool_{iteration}_{tool_index}");
                let question = arguments.content.clone().unwrap_or_else(|| {
                    "The agent needs clarification before continuing.".to_string()
                });
                let result = crate::tool_execution::ToolExecutionResult {
                    tool_call_id: tool_call_id.clone(),
                    tool_id: tool_id.clone(),
                    ok: true,
                    preview: format!(
                        "ask_user waiting for user: {}",
                        question.chars().take(160).collect::<String>()
                    ),
                    detail_json: format!(
                        "{{\"question\":\"{}\",\"status\":\"waiting_for_user\"}}",
                        json_escape(&question)
                    ),
                    exit_code: None,
                };
                record_tool_call_requested_preserving_provider_id(
                    &mut session,
                    &tool_call_id,
                    Some(&provider_tool_call_id),
                    &tool_id,
                )?;
                record_tool_call_completed_preserving_provider_id(
                    &mut session,
                    &tool_call_id,
                    Some(&provider_tool_call_id),
                    &tool_id,
                    true,
                )?;
                turn_controller.record_tool_completed(
                    &mut session,
                    &ledger_tool_call_id,
                    &tool_id,
                    true,
                )?;
                let artifact = write_tool_result_artifact(
                    &artifact_store,
                    &format!("native_loop_v2_tool_result_{iteration}_{tool_index}"),
                    &ToolResultRecord::new(
                        &tool_call_id,
                        &tool_id,
                        result.ok,
                        result.preview.clone(),
                        result.detail_json.clone(),
                    ),
                )
                .map_err(|error| error.to_string())?;
                session
                    .record_tool_result_artifact_with_provider_id(
                        &tool_call_id,
                        Some(provider_tool_call_id.clone()),
                        &tool_id,
                        artifact.artifact_id,
                        artifact.content_hash,
                        result.preview.clone(),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                session
                    .record_runtime_event(
                        "user.question_requested",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"tool_call_id\":\"{}\",\"question\":\"{}\"}}",
                            json_escape(&tool_call_id),
                            json_escape(&question)
                        ),
                    )
                    .and_then(|_| session.transition_to(AgentState::WaitingForUser))
                    .map_err(|error| format!("{error:?}"))?;
                remember_native_loop_incomplete(&mut session, "waiting for user clarification");
                tool_call_count += 1;
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Blocked,
                    "waiting_for_user",
                    "ask_user",
                )?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                return Ok(loop_result(
                    NativeAgentLoopStatus::Blocked,
                    session,
                    tool_call_count,
                    model_call_count,
                ));
            }
            if matches!(
                tool_id.as_str(),
                "file.write" | "file.edit" | "file.multi_edit"
            ) {
                let effective_permission_mode = kernel_services.permission_gate.mode();
                let result = match execute_permissioned_write_collect(
                    &mut session,
                    &artifact_store,
                    &mut kernel_services.permission_gate,
                    &request.workspace_root,
                    iteration * 10 + tool_index,
                    Some(&provider_tool_call_id),
                    &tool_id,
                    &arguments,
                    &request.prompt,
                    None,
                    &effective_permission_mode,
                    &request.provided_permission_decisions,
                    hook_dispatcher,
                )? {
                    PermissionedWriteOutcome::Executed(result) => result,
                    PermissionedWriteOutcome::Pending(pending_tool) => {
                        emit_new_session_events(
                            &session,
                            &mut emitted_event_count,
                            &mut event_sink,
                        );
                        native_loop_state.record_terminal(
                            &mut session,
                            NativeAgentLoopStatus::Blocked,
                            "pending_permission",
                            "permission",
                        )?;
                        emit_new_session_events(
                            &session,
                            &mut emitted_event_count,
                            &mut event_sink,
                        );
                        return Ok(loop_result_with_pending(
                            NativeAgentLoopStatus::Blocked,
                            session,
                            tool_call_count,
                            model_call_count,
                            Some(pending_tool),
                        ));
                    }
                };
                if !result.ok {
                    session
                        .record_runtime_event(
                            "agent.loop_recovery",
                            researchcode_kernel::Actor::Runtime,
                            format!(
                                "{{\"iteration\":{},\"tool_id\":\"{}\",\"reason\":\"write_tool_result_error\",\"preview\":\"{}\",\"detail\":{}}}",
                                iteration,
                                json_escape(&tool_id),
                                json_escape(&result.preview),
                                safe_json_fragment(&result.detail_json)
                            ),
                        )
                        .map_err(|error| format!("{error:?}"))?;
                    iteration_error_results += 1;
                } else {
                    iteration_ok_results += 1;
                }
                turn_controller.record_tool_completed(
                    &mut session,
                    &ledger_tool_call_id,
                    &tool_id,
                    result.ok,
                )?;
                let class = ledger_class_for_tool_result(&result);
                record_native_tool_batch_item(
                    &mut evidence_ledger,
                    &mut last_tool_batch,
                    provider_tool_call_id.clone(),
                    tool_id,
                    mediated.arguments_json.clone(),
                    result,
                    class,
                );
                tool_call_count += 1;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                continue;
            }
            if tool_id == "shell.command" {
                let effective_permission_mode = kernel_services.permission_gate.mode();
                let result = match execute_permissioned_command_collect(
                    &mut session,
                    &artifact_store,
                    &mut kernel_services.permission_gate,
                    &request.workspace_root,
                    iteration * 10 + tool_index,
                    Some(&provider_tool_call_id),
                    &arguments,
                    &effective_permission_mode,
                    &request.provided_permission_decisions,
                    hook_dispatcher,
                )? {
                    PermissionedCommandOutcome::Executed(result) => result,
                    PermissionedCommandOutcome::Pending(pending_tool) => {
                        emit_new_session_events(
                            &session,
                            &mut emitted_event_count,
                            &mut event_sink,
                        );
                        native_loop_state.record_terminal(
                            &mut session,
                            NativeAgentLoopStatus::Blocked,
                            "pending_permission",
                            "permission",
                        )?;
                        emit_new_session_events(
                            &session,
                            &mut emitted_event_count,
                            &mut event_sink,
                        );
                        return Ok(loop_result_with_pending(
                            NativeAgentLoopStatus::Blocked,
                            session,
                            tool_call_count,
                            model_call_count,
                            Some(pending_tool),
                        ));
                    }
                };
                if !result.ok {
                    session
                        .record_runtime_event(
                            "agent.loop_recovery",
                            researchcode_kernel::Actor::Runtime,
                            format!(
                                "{{\"iteration\":{},\"tool_id\":\"{}\",\"reason\":\"tool_result_error\",\"preview\":\"{}\",\"detail\":{}}}",
                                iteration,
                                json_escape(&tool_id),
                                json_escape(&result.preview),
                                safe_json_fragment(&result.detail_json)
                            ),
                        )
                        .map_err(|error| format!("{error:?}"))?;
                    iteration_error_results += 1;
                } else {
                    iteration_ok_results += 1;
                }
                turn_controller.record_tool_completed(
                    &mut session,
                    &ledger_tool_call_id,
                    &tool_id,
                    result.ok,
                )?;
                let class = ledger_class_for_tool_result(&result);
                record_native_tool_batch_item(
                    &mut evidence_ledger,
                    &mut last_tool_batch,
                    provider_tool_call_id.clone(),
                    tool_id,
                    mediated.arguments_json.clone(),
                    result,
                    class,
                );
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                continue;
            }
            let is_dup = turn_state.observation_cache.check_and_record_in_workspace(
                &tool_id,
                &arguments,
                &request.workspace_root,
            );
            let (result, is_dup_obs) = if let Some(cache_key) = is_dup {
                (
                    execute_duplicate_observation_collect(
                        &mut session,
                        &artifact_store,
                        iteration * 10 + tool_index,
                        Some(&provider_tool_call_id),
                        &tool_id,
                        &mediated.arguments_json,
                        &cache_key,
                    )?,
                    true,
                )
            } else {
                (
                    execute_read_only_collect(
                        &mut session,
                        &artifact_store,
                        &request.workspace_root,
                        iteration * 10 + tool_index,
                        Some(&provider_tool_call_id),
                        &tool_id,
                        &arguments,
                        hook_dispatcher,
                    )?,
                    false,
                )
            };
            if is_dup_obs {
                duplicate_suppressed_count += 1;
                turn_controller.record_tool_completed(
                    &mut session,
                    &ledger_tool_call_id,
                    &tool_id,
                    result.ok,
                )?;
                record_native_tool_batch_item(
                    &mut evidence_ledger,
                    &mut last_tool_batch,
                    provider_tool_call_id.clone(),
                    tool_id,
                    mediated.arguments_json.clone(),
                    result,
                    EvidenceClass::Recovery,
                );
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                continue;
            }
            if let Some(cache_key) = turn_state
                .observation_cache
                .check_and_record_weak_hint(&tool_id, &arguments)
            {
                session
                    .record_runtime_event(
                        "tool.weak_duplicate_observation_hint",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"tool_call_id\":{},\"tool_id\":{},\"cache_key\":{},\"suppressed\":false,\"next_action_hint\":\"This unsupported read-only observation used the same weak argument key before. Prefer new evidence unless repeating is necessary.\"}}",
                            json_string(&provider_tool_call_id),
                            json_string(&tool_id),
                            json_string(&cache_key)
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
            }
            if !result.ok {
                session
                    .record_runtime_event(
                        "agent.loop_recovery",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"iteration\":{},\"tool_id\":\"{}\",\"reason\":\"tool_result_error\",\"preview\":\"{}\",\"detail\":{}}}",
                            iteration,
                            json_escape(&tool_id),
                            json_escape(&result.preview),
                            safe_json_fragment(&result.detail_json)
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                iteration_error_results += 1;
            } else {
                iteration_ok_results += 1;
            }
            turn_controller.record_tool_completed(
                &mut session,
                &ledger_tool_call_id,
                &tool_id,
                result.ok,
            )?;
            tool_call_count += 1;
            let class = ledger_class_for_tool_result(&result);
            record_native_tool_batch_item(
                &mut evidence_ledger,
                &mut last_tool_batch,
                provider_tool_call_id.clone(),
                tool_id,
                mediated.arguments_json.clone(),
                result,
                class,
            );
            emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
        }
        let progress_duplicate_results = duplicate_suppressed_count;
        let progress_error_results = last_tool_batch
            .iter()
            .filter(|(_, _, _, result)| !result.ok)
            .count() as u32;
        let repeated_error_signatures = last_tool_batch
            .iter()
            .filter(|(_, _, _, result)| !result.ok)
            .filter_map(|(_, _, _, result)| {
                model_readable_error_signature(result)
                    .map(|signature| (result.tool_id.clone(), signature))
            })
            .collect::<Vec<_>>();
        let seen_tool_batches_snapshot = turn_state.seen_tool_batches.clone();
        match iteration_controller.observe_completed_tool_iteration(
            &mut session,
            &mut turn_state,
            &convergence_enforcer,
            &evidence_ledger,
            ToolIterationControlInput {
                iteration,
                ok_results: iteration_ok_results,
                error_results: iteration_error_results,
                duplicate_results: progress_duplicate_results,
                distinct_keys_before,
                progress_error_results,
                repeated_error_signatures,
                batch_signature: &batch_signature,
                seen_tool_batches: &seen_tool_batches_snapshot,
            },
        )? {
            ToolIterationControlAction::Continue => {}
            ToolIterationControlAction::SoftWarning => {
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
            }
            ToolIterationControlAction::Stop { reason } => {
                if (native_prompt_wants_file_generation(&request.prompt)
                    || native_prompt_wants_write_or_edit(&request.prompt))
                    && !last_tool_batch.iter().any(|(_, tool_id, _, result)| {
                        result.ok
                            && matches!(
                                tool_id.as_str(),
                                "file.write" | "file.edit" | "file.multi_edit" | "patch.apply"
                            )
                    })
                {
                    session
                        .record_runtime_event(
                            "agent.write_progress.blocked",
                            researchcode_kernel::Actor::Runtime,
                            format!(
                                "{{\"iteration\":{},\"reason\":\"write_task_stopped_without_state_change\",\"original_reason\":{}}}",
                                iteration,
                                json_string(reason)
                            ),
                        )
                        .map_err(|error| format!("{error:?}"))?;
                }
                stop_native_loop_with_structured_blocked(
                    &mut session,
                    &request.prompt,
                    &last_tool_batch,
                    reason,
                    "tool_iteration_stop",
                    &mut emitted_event_count,
                    &mut event_sink,
                )?;
                native_loop_state.record_terminal(
                    &mut session,
                    NativeAgentLoopStatus::Blocked,
                    reason,
                    "tool_iteration_stop",
                )?;
                emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
                return Ok(loop_result(
                    NativeAgentLoopStatus::Blocked,
                    session,
                    tool_call_count,
                    model_call_count,
                ));
            }
        }
    }
    if !last_tool_batch.is_empty() {
        iteration_controller
            .record_max_iterations_budget_reached(&mut session, turn_state.budget.max_iterations)?;
        stop_native_loop_with_structured_blocked(
            &mut session,
            &request.prompt,
            &last_tool_batch,
            "max_iterations",
            "turn_budget",
            &mut emitted_event_count,
            &mut event_sink,
        )?;
        native_loop_state.record_terminal(
            &mut session,
            NativeAgentLoopStatus::Blocked,
            "max_iterations",
            "turn_budget",
        )?;
        emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
        return Ok(loop_result(
            NativeAgentLoopStatus::Blocked,
            session,
            tool_call_count,
            model_call_count,
        ));
    }
    session
        .record_runtime_event(
            "agent.loop_incomplete",
            researchcode_kernel::Actor::Runtime,
            format!(
                "{{\"reason\":\"max_iterations\",\"max_iterations\":{},\"tool_calls\":{}}}",
                request.max_iterations, tool_call_count
            ),
        )
        .map_err(|error| format!("{error:?}"))?;
    session
        .diagnose_failure()
        .map_err(|error| format!("{error:?}"))?;
    native_loop_state.record_terminal(
        &mut session,
        NativeAgentLoopStatus::Failed,
        "max_iterations",
        "loop_incomplete",
    )?;
    emit_new_session_events(&session, &mut emitted_event_count, &mut event_sink);
    Ok(loop_result(
        NativeAgentLoopStatus::Failed,
        session,
        tool_call_count,
        model_call_count,
    ))
}

#[cfg(test)]
#[path = "native_agent_loop_tests.rs"]
mod native_agent_loop_tests;
