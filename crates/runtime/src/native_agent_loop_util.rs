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

use crate::native_agent_loop::{
    NativeAgentLoopResult, NativeAgentLoopStatus, PendingNativeToolExecution,
};

pub(in crate::native_agent_loop) fn provider_openai_tool_call_id(tool_use_id: &str) -> String {
    if tool_use_id.starts_with("call_") {
        return tool_use_id.to_string();
    }
    let sanitized = tool_use_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("call_{sanitized}")
}

pub(in crate::native_agent_loop) fn native_loop_provider_label(
    family: &NativeModelFamily,
) -> &'static str {
    match family {
        NativeModelFamily::DeepSeek => "deepseek",
        NativeModelFamily::Qwen => "qwen",
    }
}

pub(in crate::native_agent_loop) fn native_loop_continuation_preview(
    family: &NativeModelFamily,
) -> &'static str {
    match family {
        NativeModelFamily::DeepSeek => "native loop v2 anthropic tool_result continuation",
        NativeModelFamily::Qwen => "native loop v2 qwen tool message continuation",
    }
}

pub(in crate::native_agent_loop) fn native_loop_evidence_continuation_preview(
    family: &NativeModelFamily,
) -> &'static str {
    match family {
        NativeModelFamily::DeepSeek => "native loop v2 compact evidence continuation",
        NativeModelFamily::Qwen => "native loop v2 qwen compact evidence continuation",
    }
}

#[cfg(test)]
pub(in crate::native_agent_loop) fn native_loop_permission_gate(
    artifact_store: &ArtifactStore,
    workspace_root: &Path,
    mode: PermissionMode,
    session_id: &str,
) -> PermissionGate {
    PermissionGate::new(
        Arc::new(PermissionRuleStore::new(native_permission_policy_path(
            artifact_store,
        ))),
        PermissionRuleSet::default(),
        mode,
        workspace_root.to_string_lossy(),
        session_id.to_string(),
    )
}

#[cfg(test)]
pub(in crate::native_agent_loop) fn native_permission_policy_path(
    artifact_store: &ArtifactStore,
) -> PathBuf {
    artifact_store
        .root()
        .parent()
        .map(|path| path.join("permission_policy.tsv"))
        .unwrap_or_else(|| artifact_store.root().join("permission_policy.tsv"))
}

pub(in crate::native_agent_loop) fn structured_tool_result_content(
    result: &crate::tool_execution::ToolExecutionResult,
) -> String {
    let (detail_preview, detail_truncated) = compact_text(&result.detail_json, 6_000);
    format!(
        "{{\"ok\":{},\"tool_id\":\"{}\",\"preview\":\"{}\",\"detail_preview\":\"{}\",\"detail_truncated\":{},\"artifact_ref\":\"artifact_{}\"}}",
        result.ok,
        json_escape(&result.tool_id),
        json_escape(&compact_inline(&result.preview, 1_000)),
        json_escape(&detail_preview),
        detail_truncated,
        json_escape(&result.tool_call_id)
    )
}

pub(in crate::native_agent_loop) fn compact_text(value: &str, max_chars: usize) -> (String, bool) {
    if value.chars().count() <= max_chars {
        return (value.to_string(), false);
    }
    let mut output = value.chars().take(max_chars).collect::<String>();
    output.push_str("\n[truncated]");
    (output, true)
}

pub(in crate::native_agent_loop) fn compact_inline(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    compact_text(&normalized, max_chars).0
}

pub(in crate::native_agent_loop) fn native_loop_synthetic_tool_error(
    iteration: usize,
    tool_index: usize,
    tool_id: &str,
    arguments_json: &str,
    user_prompt: &str,
) -> crate::tool_execution::ToolExecutionResult {
    let next_action_hint = native_loop_recovery_hint(tool_id, arguments_json, user_prompt);
    crate::tool_execution::ToolExecutionResult {
        tool_call_id: format!("native_loop_v2_recovery_{iteration}_{tool_index}"),
        tool_id: tool_id.to_string(),
        ok: false,
        preview: format!(
            "loop_guard recovery: repeated {tool_id}; switch strategy instead of repeating"
        ),
        detail_json: format!(
            "{{\"ok\":false,\"error_code\":\"repeated_tool_batch\",\"recoverable\":true,\"tool_id\":\"{}\",\"arguments_preview\":\"{}\",\"next_action_hint\":\"{}\"}}",
            json_escape(tool_id),
            json_escape(&arguments_json.chars().take(500).collect::<String>()),
            json_escape(&next_action_hint)
        ),
        exit_code: None,
    }
}

pub(in crate::native_agent_loop) fn native_loop_recovery_hint(
    tool_id: &str,
    arguments_json: &str,
    user_prompt: &str,
) -> String {
    let lower_prompt = user_prompt.to_lowercase();
    let lower_args = arguments_json.to_lowercase();
    let looks_like_creation_task = [
        "html",
        "网页",
        "页面",
        "小程序",
        "app",
        "创建",
        "写",
        "实现",
        "继续完成",
    ]
    .iter()
    .any(|needle| lower_prompt.contains(needle));
    if tool_id == "file.read"
        && (lower_args.contains("\".\"")
            || lower_args.contains("path_is_directory")
            || lower_args.contains("未命名文件夹")
            || lower_args.contains("directory"))
    {
        return "The path is a directory. Use file.list_directory or file.list_tree on that root, then read concrete files only. Do not repeat the same file read on that directory again.".to_string();
    }
    if looks_like_creation_task {
        return "If repo/search found no existing target file, stop scanning and either produce a concise final answer or use the write-capable outer tool loop with complete, non-truncated file content. Do not repeat the same read/search batch.".to_string();
    }
    "Use a different tool or different arguments, summarize what is known, or ask for clarification. Do not repeat the same tool batch.".to_string()
}

pub(in crate::native_agent_loop) fn suggested_manifest_tool(
    allowed_tools: &BTreeSet<String>,
    requested_tool_id: &str,
) -> Option<String> {
    if requested_tool_id.contains("read") {
        return if allowed_tools.contains("file.read") {
            Some("file.read".to_string())
        } else {
            None
        };
    }
    if allowed_tools.contains("file.list_directory")
        && (requested_tool_id.contains("list")
            || requested_tool_id.contains("dir")
            || requested_tool_id.contains("shell")
            || requested_tool_id.contains("command"))
    {
        return Some("file.list_directory".to_string());
    }
    if allowed_tools.contains("file.list_tree")
        && (requested_tool_id.contains("tree") || requested_tool_id.contains("dir"))
    {
        return Some("file.list_tree".to_string());
    }
    if allowed_tools.contains("file.read") && requested_tool_id.contains("read") {
        return Some("file.read".to_string());
    }
    if allowed_tools.contains("repo.map")
        && (requested_tool_id.contains("list")
            || requested_tool_id.contains("dir")
            || requested_tool_id.contains("tree"))
    {
        return Some("repo.map".to_string());
    }
    if allowed_tools.contains("search.ripgrep")
        && (requested_tool_id.contains("search") || requested_tool_id.contains("grep"))
    {
        return Some("search.ripgrep".to_string());
    }
    if allowed_tools.contains("git.status") && requested_tool_id.contains("git") {
        return Some("git.status".to_string());
    }
    if allowed_tools.contains("plan.write") {
        return Some("plan.write".to_string());
    }
    None
}

pub(in crate::native_agent_loop) fn remember_native_loop_incomplete(
    session: &mut AgentSession,
    reason: &str,
) {
    if let Err(e) = session.record_runtime_event(
        "agent.continuation_summary",
        researchcode_kernel::Actor::Runtime,
        format!(
            "{{\"status\":\"incomplete\",\"reason\":\"{}\",\"next_action\":\"continue same session with prior tool results and path corrections\"}}",
            json_escape(reason)
        ),
    ) {
        eprintln!("WARNING: remember_native_loop_incomplete failed to record event for reason={reason}: {e:?}");
    }
}

pub(in crate::native_agent_loop) fn record_native_model_http_failure_event(
    session: &mut AgentSession,
    call_id: &str,
    family: &NativeModelFamily,
    http_status: u16,
    failure_preview: &str,
    reason: &str,
    action: &str,
) -> Result<(), String> {
    session
        .record_model_call_completed(
            call_id,
            native_loop_provider_label(family),
            false,
            format!("{call_id}_http_failure"),
            stable_text_hash(failure_preview),
        )
        .and_then(|_| {
            session.record_runtime_event(
                "model.http_failure_recovered",
                researchcode_kernel::Actor::Runtime,
                format!(
                    "{{\"call_id\":{},\"status\":{},\"reason\":{},\"preview\":{},\"action\":{}}}",
                    json_string(call_id),
                    http_status,
                    json_string(reason),
                    json_string(failure_preview),
                    json_string(action)
                ),
            )
        })
        .map_err(|error| format!("{error:?}"))
}

pub(in crate::native_agent_loop) fn json_escape(value: &str) -> String {
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

pub(in crate::native_agent_loop) fn safe_json_fragment(value: &str) -> String {
    let trimmed = value.trim();
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        if is_balanced_json_fragment(trimmed) {
            return trimmed.to_string();
        }
    }
    format!("{{\"raw\": \"{}\"}}", json_escape(trimmed))
}

pub(in crate::native_agent_loop) fn tool_contract_failure_signature(
    mediated: &crate::tcml::MediatedToolCall,
    error: &ModelReadableToolError,
) -> String {
    let mut parts = vec![
        format!("requested={}", mediated.requested_tool_id),
        format!("resolved={}", mediated.tool_id),
        format!("error={}", error.error_code),
    ];
    let missing_fields = missing_required_fields_from_tool_error(error);
    if !missing_fields.is_empty() {
        parts.push(format!("missing={}", missing_fields.join(",")));
    } else {
        parts.push(format!(
            "message={}",
            stable_text_hash(&error.short_message)
        ));
    }
    parts.join("|")
}

pub(in crate::native_agent_loop) fn missing_required_fields_from_tool_error(
    error: &ModelReadableToolError,
) -> Vec<String> {
    let mut fields = Vec::new();
    let marker = "missing required field ";
    let mut rest = error.short_message.as_str();
    while let Some(index) = rest.find(marker) {
        let after = &rest[index + marker.len()..];
        let field = after
            .split(|character: char| {
                character == ','
                    || character == ';'
                    || character == '.'
                    || character.is_whitespace()
            })
            .next()
            .unwrap_or("")
            .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '_');
        if !field.is_empty() && !fields.iter().any(|candidate| candidate == field) {
            fields.push(field.to_string());
        }
        rest = after;
    }
    fields.sort();
    fields
}

pub(in crate::native_agent_loop) fn escalated_model_readable_tool_error(
    mediated: &crate::tcml::MediatedToolCall,
    error: &ModelReadableToolError,
) -> ModelReadableToolError {
    let mut escalated = error.clone();
    escalated.retryable = false;
    let missing_fields = missing_required_fields_from_tool_error(error);
    let file_edit_missing_exact_fields = mediated.tool_id == "file.edit"
        && ["new_string", "old_string"]
            .iter()
            .all(|field| missing_fields.iter().any(|candidate| candidate == field));
    let guidance = if file_edit_missing_exact_fields {
        escalated.suggested_replacement = Some("file.write".to_string());
        "Repeated invalid file.edit call. Stop retrying file.edit with missing old_string/new_string. Read the target file first to obtain exact old_string, or use file.write when creating or replacing a whole file."
    } else {
        "Repeated identical tool contract failure. Stop retrying the same arguments. Re-read the tool schema and send one corrected tool call with all required fields."
    };
    if !escalated.short_message.contains(guidance) {
        escalated.short_message = format!("{} {guidance}", escalated.short_message);
    }
    escalated
}

pub(in crate::native_agent_loop) fn is_balanced_json_fragment(input: &str) -> bool {
    let mut in_string = false;
    let mut escaped = false;
    let mut stack = Vec::new();
    for ch in input.chars() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' => stack.push(ch),
            '}' => {
                if stack.pop() != Some('{') {
                    return false;
                }
            }
            ']' => {
                if stack.pop() != Some('[') {
                    return false;
                }
            }
            _ => {}
        }
    }
    !in_string && stack.is_empty()
}

pub(in crate::native_agent_loop) fn planned_call_for_endpoint(
    endpoint: &NativeProviderEndpoint,
    role: ModelRole,
    task_summary: String,
    requires_tools: bool,
) -> Result<PlannedModelCall, String> {
    match endpoint.family {
        NativeModelFamily::DeepSeek => DeepSeekNativeAdapter::new(
            NativeModelProfile {
                profile_id: "deepseek-v4-native".to_string(),
                family: NativeModelFamily::DeepSeek,
                optimization_level: OptimizationLevel::Native,
            },
            &endpoint.actual_model_name,
        )?
        .plan_call(&ModelAdapterRequest {
            role,
            task_summary,
            requires_tools,
            context_tokens_estimate: 2_000,
        }),
        NativeModelFamily::Qwen => QwenNativeAdapter::new(
            NativeModelProfile {
                profile_id: "qwen3-6-27b-native".to_string(),
                family: NativeModelFamily::Qwen,
                optimization_level: OptimizationLevel::Native,
            },
            &endpoint.actual_model_name,
        )?
        .plan_call(&ModelAdapterRequest {
            role,
            task_summary,
            requires_tools,
            context_tokens_estimate: 2_000,
        }),
    }
}

pub(in crate::native_agent_loop) fn resolve_workspace_path(
    root: &Path,
    value: &str,
) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let input = PathBuf::from(value);
    let candidate = if input.is_absolute() {
        input
    } else {
        root.join(input)
    };
    let resolved = candidate
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !resolved.starts_with(&root) {
        return Err("path escapes workspace".to_string());
    }
    Ok(resolved)
}

pub(in crate::native_agent_loop) fn resolve_workspace_write_path(
    root: &Path,
    value: &str,
) -> Result<PathBuf, String> {
    if value.trim().is_empty() {
        return Err("path is required".to_string());
    }
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let input = PathBuf::from(value);
    let candidate = if input.is_absolute() {
        input
    } else {
        root.join(input)
    };
    let parent = candidate
        .parent()
        .ok_or_else(|| "path escapes workspace".to_string())?;
    let resolved_parent = if parent.exists() {
        parent.canonicalize().map_err(|error| error.to_string())?
    } else {
        let mut ancestor = parent.to_path_buf();
        while !ancestor.exists() {
            ancestor = ancestor
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root.clone());
        }
        let resolved_ancestor = ancestor.canonicalize().map_err(|error| error.to_string())?;
        if !resolved_ancestor.starts_with(&root) {
            return Err("path escapes workspace".to_string());
        }
        let remainder = parent.strip_prefix(&ancestor).unwrap_or(Path::new(""));
        resolved_ancestor.join(remainder)
    };
    if !resolved_parent.starts_with(&root) {
        return Err("path escapes workspace".to_string());
    }
    let normalized = candidate
        .components()
        .fold(PathBuf::new(), |mut acc, comp| {
            match comp {
                std::path::Component::ParentDir => {
                    acc.pop();
                }
                std::path::Component::CurDir => {}
                other => acc.push(other.as_os_str()),
            }
            acc
        });
    if !normalized.starts_with(&root) {
        return Err("path escapes workspace".to_string());
    }
    if normalized.exists() {
        if let Ok(meta) = normalized.symlink_metadata() {
            if meta.file_type().is_symlink() {
                return Err("path escapes workspace".to_string());
            }
        }
    }
    Ok(normalized)
}

pub(in crate::native_agent_loop) fn loop_result(
    status: NativeAgentLoopStatus,
    session: AgentSession,
    tool_call_count: usize,
    model_call_count: usize,
) -> NativeAgentLoopResult {
    loop_result_with_pending(status, session, tool_call_count, model_call_count, None)
}

pub(in crate::native_agent_loop) fn loop_result_with_pending(
    status: NativeAgentLoopStatus,
    session: AgentSession,
    tool_call_count: usize,
    model_call_count: usize,
    pending_tool: Option<PendingNativeToolExecution>,
) -> NativeAgentLoopResult {
    let event_jsonl = session.export_events_jsonl();
    let usage = aggregate_model_usage_from_jsonl(&event_jsonl);
    NativeAgentLoopResult {
        status,
        final_state: session.state(),
        event_count: session.event_count(),
        tool_call_count,
        model_call_count,
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        prompt_cache_hit_tokens: usage.prompt_cache_hit_tokens,
        prompt_cache_miss_tokens: usage.prompt_cache_miss_tokens,
        event_jsonl,
        pending_tool,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::native_agent_loop) struct ModelUsageTotals {
    prompt_tokens: u64,
    completion_tokens: u64,
    reasoning_tokens: u64,
    prompt_cache_hit_tokens: u64,
    prompt_cache_miss_tokens: u64,
}

pub(in crate::native_agent_loop) fn aggregate_model_usage_from_jsonl(
    jsonl: &str,
) -> ModelUsageTotals {
    let mut totals = ModelUsageTotals::default();
    for line in jsonl.lines() {
        if !line.contains("\"event_type\":\"model.stream_completed\"") {
            continue;
        }
        totals.prompt_tokens += extract_json_u64_local(line, "prompt_tokens").unwrap_or(0);
        totals.completion_tokens += extract_json_u64_local(line, "completion_tokens").unwrap_or(0);
        totals.reasoning_tokens += extract_json_u64_local(line, "reasoning_tokens").unwrap_or(0);
        totals.prompt_cache_hit_tokens +=
            extract_json_u64_local(line, "prompt_cache_hit_tokens").unwrap_or(0);
        totals.prompt_cache_miss_tokens +=
            extract_json_u64_local(line, "prompt_cache_miss_tokens").unwrap_or(0);
    }
    totals
}

pub(in crate::native_agent_loop) fn extract_json_u64_local(input: &str, key: &str) -> Option<u64> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let tail = input[start..].trim_start();
    let number = tail
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    number.parse().ok()
}

pub(in crate::native_agent_loop) fn live_deepseek_endpoint() -> NativeProviderEndpoint {
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
    endpoint.live_calls_enabled_by_default = true;
    endpoint.api_key_env = "PATH".to_string();
    endpoint
}

pub(in crate::native_agent_loop) fn live_qwen_endpoint() -> NativeProviderEndpoint {
    let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
    endpoint.live_calls_enabled_by_default = true;
    endpoint.api_key_env = "PATH".to_string();
    endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
    endpoint
}

pub(in crate::native_agent_loop) fn pending_tool_to_json(
    pending: &PendingNativeToolExecution,
) -> String {
    format!(
        "{{\"step_index\":{},\"tool_call_id\":{},\"tool_id\":{},\"permission_id\":{},\"request_type\":{},\"patch_id\":{},\"args\":{}}}\n",
        pending.step_index,
        json_string(&pending.tool_call_id),
        json_string(&pending.tool_id),
        json_string(&pending.permission_id),
        json_string(permission_request_type_to_str(&pending.request_type)),
        json_string(pending.patch_id.as_deref().unwrap_or("")),
        tool_execution_args_to_json(&pending.args)
    )
}

pub(in crate::native_agent_loop) fn pending_tool_from_json(
    input: &str,
) -> Result<PendingNativeToolExecution, String> {
    let step_index = extract_json_usize(input, "step_index")
        .ok_or_else(|| "pending tool missing step_index".to_string())?;
    let request_type = extract_json_string(input, "request_type")
        .and_then(|value| PermissionRequestType::parse(&value))
        .ok_or_else(|| "pending tool missing valid request_type".to_string())?;
    let patch_id = extract_json_string(input, "patch_id").filter(|value| !value.is_empty());
    Ok(PendingNativeToolExecution {
        step_index,
        tool_call_id: extract_json_string(input, "tool_call_id")
            .ok_or_else(|| "pending tool missing tool_call_id".to_string())?,
        tool_id: extract_json_string(input, "tool_id")
            .ok_or_else(|| "pending tool missing tool_id".to_string())?,
        permission_id: extract_json_string(input, "permission_id")
            .ok_or_else(|| "pending tool missing permission_id".to_string())?,
        request_type,
        patch_id,
        args: tool_execution_args_from_json(input),
    })
}

pub(in crate::native_agent_loop) fn tool_execution_args_to_json(
    args: &ToolExecutionArgs,
) -> String {
    format!(
        "{{\"path\":{},\"root\":{},\"include_hidden\":{},\"pattern\":{},\"query\":{},\"command\":{},\"content\":{},\"old_string\":{},\"new_string\":{},\"base_hash\":{},\"replace_all\":{},\"offset\":{},\"limit\":{},\"max_bytes\":{},\"max_results\":{},\"max_files\":{},\"max_depth\":{},\"edits\":{},\"input_csv\":{},\"job_id\":{},\"output_dir\":{}}}",
        json_optional_string(args.path.as_deref()),
        json_optional_string(args.root.as_deref()),
        json_optional_bool(args.include_hidden),
        json_optional_string(args.pattern.as_deref()),
        json_optional_string(args.query.as_deref()),
        json_optional_string(args.command.as_deref()),
        json_optional_string(args.content.as_deref()),
        json_optional_string(args.old_string.as_deref()),
        json_optional_string(args.new_string.as_deref()),
        json_optional_string(args.base_hash.as_deref()),
        json_optional_bool(args.replace_all),
        json_optional_usize(args.offset),
        json_optional_usize(args.limit),
        json_optional_usize(args.max_bytes),
        json_optional_usize(args.max_results),
        json_optional_usize(args.max_files),
        json_optional_usize(args.max_depth),
        args.edits_json.as_deref().unwrap_or("null"),
        json_optional_string(args.input_csv.as_deref()),
        json_optional_string(args.job_id.as_deref()),
        json_optional_string(args.output_dir.as_deref())
    )
}

pub(in crate::native_agent_loop) fn tool_execution_args_from_json(
    input: &str,
) -> ToolExecutionArgs {
    ToolExecutionArgs {
        path: extract_json_string(input, "path"),
        root: extract_json_string(input, "root"),
        include_hidden: extract_json_bool(input, "include_hidden"),
        pattern: extract_json_string(input, "pattern"),
        query: extract_json_string(input, "query"),
        command: extract_json_string(input, "command"),
        content: extract_json_string(input, "content"),
        old_string: extract_json_string(input, "old_string"),
        new_string: extract_json_string(input, "new_string"),
        base_hash: extract_json_string(input, "base_hash"),
        replace_all: extract_json_bool(input, "replace_all"),
        offset: extract_json_usize(input, "offset"),
        limit: extract_json_usize(input, "limit"),
        max_bytes: extract_json_usize(input, "max_bytes"),
        max_results: extract_json_usize(input, "max_results"),
        max_files: extract_json_usize(input, "max_files"),
        max_depth: extract_json_usize(input, "max_depth"),
        edits_json: extract_json_value(input, "edits"),
        input_csv: extract_json_string(input, "input_csv"),
        job_id: extract_json_string(input, "job_id"),
        output_dir: extract_json_string(input, "output_dir"),
        ..ToolExecutionArgs::default()
    }
}

pub(in crate::native_agent_loop) fn permission_request_type_to_str(
    value: &PermissionRequestType,
) -> &'static str {
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

pub(in crate::native_agent_loop) fn extract_json_usize(input: &str, key: &str) -> Option<usize> {
    let marker = format!("\"{key}\"");
    let key_start = input.find(&marker)? + marker.len();
    let after_key = input[key_start..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    let digits: String = after_colon
        .chars()
        .take_while(|char| char.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

pub(in crate::native_agent_loop) fn json_optional_string(value: Option<&str>) -> String {
    match value {
        Some(value) => json_string(value),
        None => "null".to_string(),
    }
}

pub(in crate::native_agent_loop) fn json_optional_bool(value: Option<bool>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

pub(in crate::native_agent_loop) fn json_optional_usize(value: Option<usize>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

pub(in crate::native_agent_loop) fn json_string(value: &str) -> String {
    let mut escaped = String::new();
    for char in value.chars() {
        match char {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    format!("\"{escaped}\"")
}

pub(in crate::native_agent_loop) fn native_loop_user_prompt_for_event(prompt: &str) -> &str {
    let prompt = prompt
        .split_once("\n\n# Runtime Context\n")
        .map(|(user_prompt, _)| user_prompt)
        .unwrap_or(prompt)
        .trim();
    if let Some((hint, user_prompt)) = prompt.rsplit_once("\n\nUser: ") {
        if hint.contains("simple social opener") {
            return user_prompt.trim();
        }
    }
    prompt
}

/// Detect a "transition statement / preamble-only" assistant message that the model
/// emits when it intends to continue but forgot to actually call a tool.
///
/// Symptoms we want to catch (real samples observed in production):
/// - "好的，让我查看当前项目的完整状态，确认已完成的步骤和下一步工作。"
/// - "好的，所有 Features 目录都是空的，需要创建完整的功能文件。让我从基础服务层开始逐个创建。"
/// - "Let me check the codebase first..."
/// - "I'll start by reading the README."
///
/// We deliberately keep this conservative: a long, substantive visible response
/// is unlikely to be a transition, even if it contains a continuation marker.
/// False negatives are tolerable (the model will retry next turn); false
/// positives are worse (they add a round-trip on legitimate short final
/// answers).
pub(in crate::native_agent_loop) fn visible_text_looks_like_transition_statement(
    text: &str,
) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let char_count = trimmed.chars().count();
    // Real final answers (especially after tool work) tend to be substantive.
    // Anything > 600 chars is almost certainly not a "let me X" preamble.
    if char_count > 600 {
        return false;
    }
    let lowered = trimmed.to_lowercase();
    // Negative filter: phrases that explicitly frame the current response AS
    // the final answer, not a prelude to one. These suppress transition
    // detection even if a positive marker would otherwise fire (e.g. the
    // model emits "I will provide the complete HTML in the final answer
    // instead of stopping." — that whole sentence IS the final answer).
    let final_answer_signals = [
        "final answer",
        "in conclusion",
        "to summarize",
        "summary:",
        "summary：",
        "instead of stopping",
        "instead of aborting",
        "as the answer",
        "答案是",
        "结论是",
        "最终答案",
        "总结：",
        "综上",
    ];
    if final_answer_signals
        .iter()
        .any(|signal| lowered.contains(signal))
    {
        return false;
    }
    // Strong Chinese transition markers. These are highly specific to preamble
    // patterns the model emits when it INTENDS to call a tool next but forgets.
    let chinese_markers = [
        "让我",
        "我来",
        "我去",
        "我将",
        "现在我",
        "接下来",
        "下一步",
        "让我们",
        "首先让",
    ];
    if chinese_markers
        .iter()
        .any(|marker| trimmed.contains(marker))
    {
        return true;
    }
    // Strong English transition markers. We deliberately exclude the looser
    // "i'll" / "i will" / "going to" forms — those appear too often in real
    // final answers ("I'll note that...", "going to mention...").
    let english_markers = [
        "let me ",
        "let's ",
        "next, i",
        "now i'll",
        "now let",
        "first, let",
        "first let me",
        "moving on,",
        "to start,",
    ];
    if english_markers
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        return true;
    }
    // Short content ending with open-ended punctuation suggests continuation.
    if char_count <= 200
        && (trimmed.ends_with("...")
            || trimmed.ends_with("…")
            || trimmed.ends_with(':')
            || trimmed.ends_with('：'))
    {
        return true;
    }
    false
}

#[cfg(test)]
mod transition_detect_tests {
    use super::visible_text_looks_like_transition_statement;

    #[test]
    fn detects_chinese_let_me_preamble() {
        assert!(visible_text_looks_like_transition_statement(
            "好的，让我查看当前项目的完整状态，确认已完成的步骤和下一步工作。"
        ));
    }

    #[test]
    fn detects_chinese_continue_creation_preamble() {
        assert!(visible_text_looks_like_transition_statement(
            "好的，所有 Features 目录都是空的，需要创建完整的功能文件。让我从基础服务层开始逐个创建。"
        ));
    }

    #[test]
    fn detects_english_let_me_preamble() {
        assert!(visible_text_looks_like_transition_statement(
            "Let me check the codebase structure first."
        ));
    }

    #[test]
    fn detects_short_trailing_ellipsis() {
        assert!(visible_text_looks_like_transition_statement(
            "Working on it..."
        ));
    }

    #[test]
    fn detects_trailing_colon_list_intro() {
        assert!(visible_text_looks_like_transition_statement(
            "下一步将完成以下："
        ));
    }

    #[test]
    fn rejects_substantive_final_answer() {
        let answer = "The README explains that this project is a DeepSeek-first agent kernel. \
                      Key components include an AgentKernel facade, a TCML pipeline, and a \
                      permission gate. The runtime is implemented in Rust and exposes a \
                      RuntimeFacade to the desktop client. No further action is required.";
        assert!(!visible_text_looks_like_transition_statement(answer));
    }

    #[test]
    fn rejects_short_complete_sentence() {
        assert!(!visible_text_looks_like_transition_statement(
            "The answer is 4."
        ));
    }

    #[test]
    fn rejects_empty_string() {
        assert!(!visible_text_looks_like_transition_statement(""));
        assert!(!visible_text_looks_like_transition_statement("   \n  "));
    }

    #[test]
    fn rejects_long_response_even_with_marker() {
        // Long, substantive response that happens to contain "let me" near the end.
        let mut long = String::new();
        for _ in 0..40 {
            long.push_str("Implementation detail. ");
        }
        long.push_str("Let me note this remains intentional.");
        assert!(!visible_text_looks_like_transition_statement(&long));
    }

    #[test]
    fn rejects_response_framing_itself_as_final_answer() {
        // Real example from converts_tool_error_to_tool_result fixture:
        // model commits to providing content "in the final answer" -- this IS
        // the final answer, not a prelude to one. Must NOT be flagged as
        // transition even though it contains "i will".
        let answer = "The write tool was not executed because this turn is \
                      read-only; I will provide the complete HTML content \
                      in the final answer instead of stopping.";
        assert!(!visible_text_looks_like_transition_statement(answer));
    }

    #[test]
    fn rejects_short_completion_acknowledgements() {
        // Common short final answers after tool work that must NOT trigger
        // the transition fallback (these are in production test fixtures).
        for sample in [
            "已写入 clock.html。",
            "Created demo.html.",
            "First turn done.",
            "Pairable call id.",
            "Live sink done.",
            "Recovered after retry.",
            "Child inspected README.",
            "Task dispatch child complete.",
        ] {
            assert!(
                !visible_text_looks_like_transition_statement(sample),
                "false positive on completion acknowledgement: {sample:?}",
            );
        }
    }

    #[test]
    fn loose_i_will_no_longer_triggers() {
        // Regression guard: previously "i will " was a positive marker which
        // produced false positives. With the tightened markers, a bare
        // "I will" form must NOT trigger detection.
        assert!(!visible_text_looks_like_transition_statement(
            "I will note that this is the result."
        ));
        assert!(!visible_text_looks_like_transition_statement(
            "I'll mention the README contains a demo description."
        ));
    }
}
