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
    ToolIterationControlInput, TurnBudget, TurnController, TurnRoute, TurnState,
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
use researchcode_kernel::tool::provider_tool_name_for_id;
use researchcode_kernel::{PermissionDecisionKind, PermissionRequestType};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

// Layer B: turn completion and structured-stop helpers.

use crate::native_agent_loop::native_agent_loop_util::{
    compact_inline, compact_text, json_escape, json_string,
};
use crate::native_agent_loop::NativeAgentToolExposure;

pub(in crate::native_agent_loop) fn compact_tool_evidence_summary(
    tool_batch: &NativeToolBatch,
    max_items: usize,
    detail_chars: usize,
) -> String {
    tool_batch
        .iter()
        .rev()
        .take(max_items)
        .map(|(provider_tool_call_id, tool_id, args_json, result)| {
            let (detail_preview, detail_truncated) =
                compact_text(&result.detail_json, detail_chars);
            format!(
                "- tool_call_id={} result_tool_call_id={} tool={} ok={} args={} preview={} detail_truncated={} detail_preview={}",
                compact_inline(provider_tool_call_id, 160),
                compact_inline(&result.tool_call_id, 160),
                tool_id,
                result.ok,
                compact_inline(args_json, 240),
                compact_inline(&result.preview, 360),
                detail_truncated,
                compact_inline(&detail_preview, detail_chars)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(in crate::native_agent_loop) fn native_loop_tool_execution_error_result(
    tool_call_id: &str,
    tool_id: &str,
    error: &crate::tool_execution::ToolExecutionError,
) -> crate::tool_execution::ToolExecutionResult {
    let error_text = format!("{error:?}");
    let next_action_hint = match error {
        crate::tool_execution::ToolExecutionError::MissingArgument(argument) => {
            format!("Retry {tool_id} with the required `{argument}` argument.")
        }
        crate::tool_execution::ToolExecutionError::PermissionRequired(_) => {
            format!(
                "{tool_id} requires an approval or governance path. Use plan.enter for planning, or wait for permission before writes/shell."
            )
        }
        crate::tool_execution::ToolExecutionError::NonReadOnlyTool(_) => {
            format!(
                "{tool_id} is not allowed in this read-only native loop turn. Use plan.enter/plan.write for planning, or request a write-capable turn."
            )
        }
        crate::tool_execution::ToolExecutionError::UnknownTool(_) => {
            "Use one of the advertised tools only; do not invent new tool names.".to_string()
        }
        crate::tool_execution::ToolExecutionError::SensitivePath(_)
        | crate::tool_execution::ToolExecutionError::PathEscapesWorkspace(_)
        | crate::tool_execution::ToolExecutionError::ValidationFailed(_) => {
            "Correct the path or arguments before retrying; do not repeat the same call."
                .to_string()
        }
        crate::tool_execution::ToolExecutionError::ToolFailed(_) => {
            "Use a different tool strategy or summarize the available evidence.".to_string()
        }
    };
    crate::tool_execution::ToolExecutionResult {
        tool_call_id: tool_call_id.to_string(),
        tool_id: tool_id.to_string(),
        ok: false,
        preview: format!("tool error {error_text}; recoverable=true"),
        detail_json: format!(
            "{{\"ok\":false,\"error_code\":\"tool_execution_error\",\"raw_error\":\"{}\",\"recoverable\":true,\"next_action_hint\":\"{}\"}}",
            json_escape(&error_text),
            json_escape(&next_action_hint)
        ),
        exit_code: None,
    }
}

pub(in crate::native_agent_loop) fn build_native_loop_tool_manifest(
    exposure: &NativeAgentToolExposure,
    route: &TurnRoute,
    endpoint: &NativeProviderEndpoint,
    workflow_state: &str,
) -> crate::tcml::BuiltToolManifest {
    build_tool_manifest_for_context(&ToolManifestBuildContext {
        family: endpoint.family.clone(),
        protocol: endpoint.protocol.clone(),
        exposure: native_loop_manifest_exposure(exposure, route),
        workflow_state: workflow_state.to_string(),
        permission_summary: "runtime_default".to_string(),
        task_contract_mode: "bounded_default".to_string(),
    })
}

pub(in crate::native_agent_loop) fn native_loop_manifest_exposure(
    exposure: &NativeAgentToolExposure,
    route: &TurnRoute,
) -> ToolManifestExposure {
    match native_agent_effective_tool_exposure_for_route(exposure, route) {
        NativeAgentToolExposure::ReadOnly => ToolManifestExposure::ReadOnly,
        NativeAgentToolExposure::FastAutoWrite => ToolManifestExposure::FastAutoWrite,
        NativeAgentToolExposure::CodeEdit => ToolManifestExposure::CodeEdit,
    }
}

pub(crate) fn native_agent_effective_tool_exposure_for_route(
    requested: &NativeAgentToolExposure,
    route: &TurnRoute,
) -> NativeAgentToolExposure {
    let routed = native_agent_tool_exposure_for_route(route);
    if native_tool_exposure_rank(requested) > native_tool_exposure_rank(&routed) {
        requested.clone()
    } else {
        routed
    }
}

pub(crate) fn native_agent_tool_exposure_for_route(route: &TurnRoute) -> NativeAgentToolExposure {
    match route {
        TurnRoute::ProjectStatus
        | TurnRoute::DirectAnswer
        | TurnRoute::ReadOnlyExplore
        | TurnRoute::Review => NativeAgentToolExposure::ReadOnly,
        TurnRoute::CodeEdit => NativeAgentToolExposure::CodeEdit,
        TurnRoute::DebugFailure | TurnRoute::RunTests | TurnRoute::LongHorizonTask => {
            NativeAgentToolExposure::FastAutoWrite
        }
    }
}

fn native_tool_exposure_rank(exposure: &NativeAgentToolExposure) -> u8 {
    match exposure {
        NativeAgentToolExposure::ReadOnly => 0,
        NativeAgentToolExposure::CodeEdit => 1,
        NativeAgentToolExposure::FastAutoWrite => 2,
    }
}

pub(in crate::native_agent_loop) fn native_loop_system_prompt(
    family: &NativeModelFamily,
    protocol: &str,
    exposure: &NativeAgentToolExposure,
    tools_json: Option<&str>,
    turn_context: Option<&str>,
) -> String {
    let write_policy = match exposure {
        NativeAgentToolExposure::ReadOnly => {
            "This turn is read-only. Do not request write, patch, shell, package, network, or destructive tools."
        }
        NativeAgentToolExposure::FastAutoWrite => {
            "FastAuto write tools may be available, but only use them for concrete requested file creation or edits inside the workspace. For explicit file creation/write requests, use the advertised file write tool with complete content instead of answering with prose or code blocks. Prefer patch-sized, complete writes with exact content. If the user asks for an approximate line count, the file content must contain real newline characters and should be formatted as readable multi-line source, not minified into one line. For a request like around 30 lines, target 24-42 physical lines and do not exceed 45 lines."
        }
        NativeAgentToolExposure::CodeEdit => {
            "This is an implementation/editing turn. Prefer existing Runtime Evidence Ledger entries first. If evidence is insufficient, use file.read or directory tools only for precise new targets, then move to file.write, file.edit, file.multi_edit, or patch.apply. Do not keep exploring once enough evidence exists to write or edit."
        }
    };
    let prompt = match family {
        NativeModelFamily::DeepSeek => format!(
            "You are ResearchCode Agent running in DeepSeek native tool mode. \
Use the provider-native tools when workspace evidence is needed; do not merely say that you will inspect files. \
For architecture, codebase, debugging, or project-analysis requests, use the advertised directory, search, git, and concrete file-read tools before the final answer. Do not repeat an already-observed root listing or file read; move to a narrower subdirectory/file or answer from evidence. \
Never replay reasoning_content as visible text. Keep final answers concise and evidence-based. \
Protocol contract: {} \
Tool-name contract: ONLY use tool names from the provided tool catalog. Do not invent alternate names such as read, execute_command, bash, exec, or search. \
If a prior tool call failed validation, reuse the exact canonical name and fix arguments only. {write_policy}",
            if protocol == "openai_compatible" {
                "Emit OpenAI-style tool_calls JSON only; never emit DSML/XML tags."
            } else {
                "Prefer native tool_use blocks; do not emit DSML/XML unless native tool blocks are unavailable."
            }
        ),
        NativeModelFamily::Qwen => format!(
            "You are ResearchCode Agent running in Qwen3.6-27B native tool mode. \
Use advertised OpenAI-style tool calls when evidence is needed; keep thinking metadata separate from visible text. \
For codebase analysis, use the advertised directory, search, git, and concrete file-read tools before answering. Do not repeat an already-observed root listing or file read; move to a narrower subdirectory/file or answer from evidence. \
Use small, precise, patch-sized edits. \
Protocol contract: {} \
Tool-name contract: ONLY use tool names from the provided tool catalog; do not invent aliases. {write_policy}",
            if protocol == "openai_compatible" {
                "Emit OpenAI-style tool_calls JSON only; do not emit DSML/XML tags."
            } else {
                "Use provider-native tool-call blocks only."
            }
        ),
    };
    if *family != NativeModelFamily::DeepSeek {
        return prompt;
    }
    let tool_catalog = tools_json.unwrap_or_default();
    let zones = crate::native_profile::deepseek::cache_prefix::CachePrefixPolicy::build_zones(
        &prompt,
        tool_catalog.lines().map(str::to_string).collect(),
        vec![
            ("protocol", protocol),
            (
                "tool_exposure",
                match exposure {
                    NativeAgentToolExposure::ReadOnly => "read_only",
                    NativeAgentToolExposure::FastAutoWrite => "fast_auto_write",
                    NativeAgentToolExposure::CodeEdit => "code_edit",
                },
            ),
        ],
        Vec::new(),
        turn_context.unwrap_or_default(),
    );
    crate::native_profile::deepseek::cache_prefix::deepseek_system_prompt(&zones)
}

pub(in crate::native_agent_loop) fn native_loop_prompt_with_turn_directives(
    prompt: &str,
    exposure: &NativeAgentToolExposure,
) -> String {
    let mut prompt_with_directives = prompt.to_string();
    if native_prompt_wants_tool_inventory(prompt) {
        prompt_with_directives.push_str(
            "\n\n# Runtime Tool Inventory Directive\nThe user is asking to verify available tools, not to chase a specific file. Use a small number of safe advertised read-only tools such as file.list_directory, file.list_tree, repo.map, search.ripgrep, or git.status. Do not guess hard-coded paths such as README.md unless a listing shows them. After the first useful tool observation, produce a concise final answer that names the exercised tools and lists write/shell/permission-gated tools as available but not executed unless explicitly approved.",
        );
    }
    if matches!(
        exposure,
        NativeAgentToolExposure::FastAutoWrite | NativeAgentToolExposure::CodeEdit
    ) {
        if let Some(write_directive) = native_loop_write_directive_for_prompt(prompt) {
            prompt_with_directives
                .push_str(&format!("\n\n# Runtime Write Directive\n{write_directive}"));
        }
    }
    prompt_with_directives
}

pub(in crate::native_agent_loop) fn native_loop_write_directive_for_prompt(
    prompt: &str,
) -> Option<&'static str> {
    if !native_prompt_wants_file_generation(prompt) {
        return None;
    }
    Some(
        "The user explicitly requested file creation or writing. Use the advertised write tool, not prose: Anthropic-compatible DeepSeek should call `file_write` with input `{ \"path\": \"generated_app.html\", \"content\": \"...complete file contents...\" }`; OpenAI-compatible providers should call the equivalent `file_write`/`file.write` function with both `path` and `content`. Use a relative path inside the workspace and complete non-truncated content. When the user asks for an approximate line count such as 30 lines, include actual newline characters and keep the source near that count: target 24-42 physical lines, never more than 45 lines. Do not minify HTML/CSS/JS into one line, and do not expand it into a long 60+ line file. You may inspect the workspace once to choose a safe path, but after a successful directory/listing observation the next state-changing tool call must be file write; do not keep listing directories, and do not satisfy the request with a prose code block.",
    )
}

pub(in crate::native_agent_loop) fn native_prompt_wants_file_generation(prompt: &str) -> bool {
    let lowered = native_prompt_user_intent(prompt).to_lowercase();
    let has_write_action = [
        "写入",
        "写进",
        "直接写",
        "保存",
        "新建",
        "创建",
        "生成",
        "实现",
        "create",
        "write",
        "save",
        "make",
    ]
    .iter()
    .any(|needle| lowered.contains(needle));
    let has_file_target = [
        "html",
        "网页",
        "页面",
        "小程序",
        "app",
        "文件夹",
        "文件",
        "file",
        ".html",
        ".css",
        ".js",
        "create file",
        "write file",
        "save file",
        "make file",
    ]
    .iter()
    .any(|needle| lowered.contains(needle));
    has_write_action && has_file_target
}

pub(in crate::native_agent_loop) fn native_prompt_is_long_running(prompt: &str) -> bool {
    let lowered = native_prompt_user_intent(prompt).to_lowercase();
    [
        "complete",
        "implement",
        "fix",
        "repair",
        "continue",
        "finish",
        "build",
        "完成",
        "修复",
        "实现",
        "继续",
        "编码",
        "开始写",
        "开写",
        "写",
        "创建",
        "修改",
        "编辑",
        "落地",
        "全部",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

pub(in crate::native_agent_loop) fn native_prompt_wants_write_or_edit(prompt: &str) -> bool {
    let lowered = native_prompt_user_intent(prompt).to_lowercase();
    [
        "write",
        "edit",
        "create",
        "modify",
        "implement",
        "continue implementation",
        "continue writing",
        "写",
        "开始写",
        "开写",
        "写入",
        "创建",
        "新建",
        "修改",
        "编辑",
        "实现",
        "编码",
        "落地",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

pub(in crate::native_agent_loop) fn native_prompt_wants_tool_inventory(prompt: &str) -> bool {
    let lowered = native_prompt_user_intent(prompt).to_lowercase();
    if native_prompt_wants_write_or_edit(prompt)
        || lowered.contains("file.write")
        || lowered.contains("file_write")
        || lowered.contains("plan.write")
        || lowered.contains("plan_write")
    {
        return false;
    }
    let mentions_tools = ["工具", "tool", "tools", "tooling"]
        .iter()
        .any(|needle| lowered.contains(needle));
    let asks_inventory_or_test = [
        "拥有",
        "可用",
        "能够",
        "哪些",
        "所有",
        "列出",
        "清单",
        "盘点",
        "available",
        "inventory",
        "catalog",
        "what tools",
        "which tools",
        "list tools",
        "tool list",
        "test tools",
        "try tools",
    ]
    .iter()
    .any(|needle| lowered.contains(needle));
    mentions_tools && asks_inventory_or_test
}

fn native_prompt_user_intent(prompt: &str) -> &str {
    prompt
        .split_once("\n\n# Runtime Context")
        .map(|(user_prompt, _)| user_prompt)
        .unwrap_or(prompt)
}

pub(in crate::native_agent_loop) fn validate_fast_auto_write_runtime_constraints(
    tool_call_id: &str,
    tool_id: &str,
    arguments: &ParsedToolArguments,
    prompt: &str,
) -> Option<crate::tool_execution::ToolExecutionResult> {
    if tool_id != "file.write" {
        return None;
    }
    let content = arguments.content.as_ref()?;
    let violation = validate_file_write_line_count(prompt, content)?;
    let policy = violation.policy;
    let actual_lines = violation.actual_lines;
    let path = arguments.path.as_deref().unwrap_or("");
    let next_action_hint = format!(
        "Retry file.write with the same relative path and complete content between {} and {} physical lines. Use real newline characters; do not list/read again.",
        policy.min, policy.max
    );
    Some(crate::tool_execution::ToolExecutionResult {
        tool_call_id: tool_call_id.to_string(),
        tool_id: tool_id.to_string(),
        ok: false,
        preview: format!(
            "runtime_validation_failed line_count_out_of_range requested={} actual={} accepted_range={}-{} recoverable=true",
            policy.target, actual_lines, policy.min, policy.max
        ),
        detail_json: format!(
            "{{\"ok\":false,\"error_code\":\"line_count_out_of_range\",\"tool_id\":\"{}\",\"path\":\"{}\",\"requested_lines\":{},\"actual_lines\":{},\"min_lines\":{},\"max_lines\":{},\"recoverable\":true,\"next_action_hint\":\"{}\"}}",
            json_escape(tool_id),
            json_escape(path),
            policy.target,
            actual_lines,
            policy.min,
            policy.max,
            json_escape(&next_action_hint)
        ),
        exit_code: None,
    })
}

pub(in crate::native_agent_loop) fn native_loop_continuation_hint(
    prompt: &str,
    tool_batch: &NativeToolBatch,
    exposure: &NativeAgentToolExposure,
) -> String {
    let write_capable_exposure = matches!(
        exposure,
        NativeAgentToolExposure::FastAutoWrite | NativeAgentToolExposure::CodeEdit
    );
    if matches!(
        exposure,
        NativeAgentToolExposure::FastAutoWrite | NativeAgentToolExposure::CodeEdit
    ) && native_prompt_wants_file_generation(prompt)
    {
        let write_failed = tool_batch
            .iter()
            .any(|(_, tool_id, _, result)| tool_id == "file.write" && !result.ok);
        if write_failed {
            if let Some(policy) = requested_line_count_policy(prompt) {
                return format!(
                    "The previous file_write/file.write call failed validation. The next model action must be exactly one corrected write tool call with the same relative `path` and complete non-empty multi-line `content` between {} and {} physical lines for the user's requested {} lines. Use real newline characters, do not minify, do not list or read again, and do not answer with prose until the write succeeds.",
                    policy.min, policy.max, policy.target
                );
            }
            return "The previous file_write/file.write call failed validation. The next model action must be exactly one corrected write tool call with a relative `path` and complete non-empty multi-line `content`; if a 30-line count was requested, target 24-42 physical lines and never exceed 45. Do not minify, do not write a 60+ line file, do not list or read again, and do not answer with prose until the write succeeds.".to_string();
        }
        let observed_workspace = tool_batch.iter().any(|(_, tool_id, _, result)| {
            result.ok
                && matches!(
                    tool_id.as_str(),
                    "file.list_directory" | "file.list_tree" | "repo.map" | "git.status"
                )
        });
        if observed_workspace {
            return "Continue from the compact tool evidence. The workspace has already been observed for this explicit file creation request. The next model action must be a file_write/file.write tool call with both `path` and complete multi-line `content`; if the user requested around 30 lines, target 24-42 physical lines and never exceed 45. Do not repeat directory or tree tools.".to_string();
        }
    }
    if write_capable_exposure
        && native_prompt_wants_write_or_edit(prompt)
        && !native_prompt_wants_tool_inventory(prompt)
    {
        let write_attempted = tool_batch.iter().any(|(_, tool_id, _, _)| {
            matches!(
                tool_id.as_str(),
                "file.write" | "file.edit" | "file.multi_edit" | "patch.apply"
            )
        });
        let observed_workspace = tool_batch.iter().any(|(_, tool_id, _, result)| {
            result.ok
                && matches!(
                    tool_id.as_str(),
                    "file.read"
                        | "file.list_directory"
                        | "file.list_tree"
                        | "repo.map"
                        | "search.ripgrep"
                        | "git.status"
                )
        });
        if observed_workspace && !write_attempted {
            return "Continue from the compact tool evidence. The user asked to proceed with implementation, and enough workspace evidence has been collected. The next model action must be file.write, file.edit, file.multi_edit, or patch.apply against concrete relative paths. If the planned file does not exist, create it under the actual observed project tree. Do not call more read/list/search tools unless a write/edit failed with a precise missing-field error.".to_string();
        }
    }
    if native_prompt_wants_tool_inventory(prompt) {
        let has_read_only_observation = tool_batch.iter().any(|(_, tool_id, _, result)| {
            result.ok
                && matches!(
                    tool_id.as_str(),
                    "file.list_directory"
                        | "file.list_tree"
                        | "repo.map"
                        | "search.ripgrep"
                        | "git.status"
                        | "file.read"
                )
        });
        let has_recoverable_path_failure = tool_batch.iter().any(|(_, tool_id, _, result)| {
            tool_id == "file.read" && !result.ok && result.detail_json.contains("path_not_found")
        });
        if has_read_only_observation || has_recoverable_path_failure {
            return "This is a tool-inventory/test request. You now have enough runtime evidence to answer. Produce the final answer now: list the read-only tools you exercised or observed, explain that write/shell/patch/plan tools are available but permission-gated, and do not call more tools or guess hard-coded paths such as README.md.".to_string();
        }
    }
    "Continue from the compact tool evidence. Do not repeat identical tool calls. Use one or more precise tools if more evidence is required; otherwise produce the final answer.".to_string()
}

pub(in crate::native_agent_loop) fn sanitize_http_failure_preview(value: &str) -> String {
    value
        .replace('\n', " ")
        .replace('\r', " ")
        .split_whitespace()
        .take(80)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(in crate::native_agent_loop) fn is_tool_budget_refusal_text(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    let cn_budget = value.contains("工具调用次数")
        || value.contains("工具调用已达上限")
        || value.contains("工具调用达到上限")
        || value.contains("调用次数已达上限");
    let cn_refusal = value.contains("无法继续")
        || value.contains("不能继续")
        || value.contains("无法读取")
        || value.contains("不能读取");
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
    (cn_budget && cn_refusal) || (en_budget && en_refusal)
}

pub(in crate::native_agent_loop) fn provider_tool_name_for_deepseek(tool_id: &str) -> String {
    provider_tool_name_for_id(tool_id)
}

pub(in crate::native_agent_loop) type NativeToolBatch = Vec<(
    String,
    String,
    String,
    crate::tool_execution::ToolExecutionResult,
)>;

pub(in crate::native_agent_loop) fn record_native_tool_batch_item(
    ledger: &mut EvidenceLedger,
    batch: &mut NativeToolBatch,
    provider_tool_call_id: String,
    tool_id: String,
    arguments_json: String,
    result: crate::tool_execution::ToolExecutionResult,
    classification: EvidenceClass,
) {
    ledger.push(
        provider_tool_call_id.clone(),
        tool_id.clone(),
        arguments_json.clone(),
        result.clone(),
        classification,
    );
    if classification != EvidenceClass::Suppressed {
        batch.push((provider_tool_call_id, tool_id, arguments_json, result));
    }
}

pub(in crate::native_agent_loop) fn replace_native_tool_batch_from_legacy(
    ledger: &mut EvidenceLedger,
    batch: &mut NativeToolBatch,
    items: NativeToolBatch,
) {
    ledger.replace_from_legacy(items.clone());
    *batch = items;
}

pub(in crate::native_agent_loop) fn continuation_view_for_batch(
    ledger: &EvidenceLedger,
    fallback_batch: &NativeToolBatch,
) -> ContinuationView {
    let view = ledger.view_for_continuation();
    if view.is_empty() && !fallback_batch.is_empty() {
        ContinuationView::from_legacy_batch(fallback_batch.clone())
    } else {
        view
    }
}

pub(in crate::native_agent_loop) fn ledger_class_for_tool_result(
    result: &crate::tool_execution::ToolExecutionResult,
) -> EvidenceClass {
    if result.ok {
        EvidenceClass::NewEvidence
    } else {
        EvidenceClass::Error
    }
}

pub(in crate::native_agent_loop) fn model_readable_error_signature(
    result: &crate::tool_execution::ToolExecutionResult,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(&result.detail_json).ok()?;
    let error_code = value
        .get("error_code")
        .and_then(|value| value.as_str())
        .map(str::to_string)?;
    if matches!(
        error_code.as_str(),
        "path_not_found" | "path_escapes_workspace" | "sensitive_path"
    ) {
        if let Some(path) = value.get("path").and_then(|value| value.as_str()) {
            return Some(format!("{error_code}:{path}"));
        }
    }
    Some(error_code)
}
