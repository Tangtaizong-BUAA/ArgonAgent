#![allow(unused_imports)]
// === native_agent_loop family imports (per docs/architecture/native_agent_loop_module_api.md §5) ===
use crate::agent_kernel::permission_gate::{classify_command_with_reasons, CommandDecision};
use crate::agent_kernel::permission_gate::{
    DefaultTool, FileEditTool, FileWriteTool, PatchApplyTool, ShellCommandTool,
};
use crate::agent_kernel::ContextSpineState;
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

use crate::native_agent_loop::native_agent_loop_prompt::{
    compact_tool_evidence_summary, native_loop_system_prompt,
    native_loop_write_directive_for_prompt, provider_tool_name_for_deepseek, NativeToolBatch,
};
use crate::native_agent_loop::native_agent_loop_util::{
    compact_text, provider_openai_tool_call_id, structured_tool_result_content,
};
use crate::native_agent_loop::NativeAgentToolExposure;

const TOOL_RESULT_ERROR_META_KEY: &str = "__researchcode_tool_result_meta";

pub(in crate::native_agent_loop) fn build_native_tool_evidence_continuation_request(
    endpoint: &NativeProviderEndpoint,
    prompt: &str,
    continuation_view: &ContinuationView,
    max_tokens: u64,
    tools_json: &str,
    exposure: &NativeAgentToolExposure,
    extra_hint: &str,
) -> Result<PreparedModelHttpRequest, String> {
    // Fallback-only path: the normal continuation route must use provider
    // tool_result/tool_use replay so tool_call_id binding stays structured.
    // Plain evidence is retained for protocol recovery when a provider rejects
    // structured tool_result replay or when compaction must shed oversized
    // provider-native transcript blocks.
    let tool_batch = continuation_view.current_legacy_batch();
    if tool_batch.is_empty() {
        return Err("tool evidence continuation requires a non-empty tool batch".to_string());
    }
    let messages = vec![
        ModelRequestMessage {
            role: "system".to_string(),
            content: format!(
                "{}\n\nContinuation contract: previous tool results are supplied below as compact runtime evidence, not provider tool_result blocks. Treat them as already executed. Do not repeat identical tool calls. Continue the task with the advertised tool catalog, or produce the final answer when the evidence is sufficient. If you need a tool, use the provider-native tool_calls channel only; never emit DSML/XML markup in visible text.",
                native_loop_system_prompt(&endpoint.family, &endpoint.protocol, exposure, Some(tools_json), None)
            ),
            cache_control_ttl: None,
        },
        ModelRequestMessage {
            role: "user".to_string(),
            content: native_tool_evidence_continuation_prompt(
                prompt,
                continuation_view,
                extra_hint,
            ),
            cache_control_ttl: None,
        },
    ];
    match endpoint.family {
        NativeModelFamily::DeepSeek if endpoint.protocol == "openai_compatible" => {
            build_deepseek_openai_request_with_tools(
                endpoint, &messages, max_tokens, true, tools_json,
            )
        }
        NativeModelFamily::DeepSeek => build_deepseek_anthropic_request_with_tools(
            endpoint, &messages, max_tokens, true, tools_json,
        ),
        NativeModelFamily::Qwen => {
            build_qwen_openai_request_with_tools(endpoint, &messages, max_tokens, true, tools_json)
        }
    }
}

pub(in crate::native_agent_loop) fn compacted_prompt_for_model(
    original_prompt: &str,
    summary: &CompactionSummary,
    spine: Option<&ContextSpineState>,
    preserved_messages: &[String],
) -> String {
    let task = original_prompt
        .split("# Runtime Context")
        .next()
        .unwrap_or(original_prompt)
        .trim();
    let spine_text = spine
        .map(ContextSpineState::to_markdown)
        .unwrap_or_default();
    let mut prompt = format!(
        "{}\n\n# Compacted Runtime Context\n{}{}",
        compact_text(task, 8_000).0,
        spine_text,
        summary.to_markdown()
    );
    if !preserved_messages.is_empty() {
        prompt.push_str("\n# Recent Turns (preserved verbatim)\n");
        for (index, message) in preserved_messages.iter().enumerate() {
            let compacted = compact_text(message, 4_000).0;
            prompt.push_str(&format!("\n## Turn {}\n{}\n", index + 1, compacted));
        }
    }
    prompt
}

pub(in crate::native_agent_loop) fn build_native_compacted_initial_request(
    endpoint: &NativeProviderEndpoint,
    prompt: &str,
    summary: &CompactionSummary,
    spine: Option<&ContextSpineState>,
    preserved_messages: &[String],
    max_tokens: u64,
    tools_json: &str,
    exposure: &NativeAgentToolExposure,
) -> Result<PreparedModelHttpRequest, String> {
    let compacted_prompt = compacted_prompt_for_model(prompt, summary, spine, preserved_messages);
    let messages = vec![
        ModelRequestMessage {
            role: "system".to_string(),
            content: format!(
                "{}\n\nCompaction contract: the user task and runtime evidence have been compacted. Use the compacted context as authoritative; do not require the omitted raw transcript unless a precise file read/search is necessary.",
                native_loop_system_prompt(&endpoint.family, &endpoint.protocol, exposure, Some(tools_json), Some("compacted runtime context is supplied in the user message"))
            ),
            cache_control_ttl: None,
        },
        ModelRequestMessage {
            role: "user".to_string(),
            content: compacted_prompt,
            cache_control_ttl: None,
        },
    ];
    match endpoint.family {
        NativeModelFamily::DeepSeek if endpoint.protocol == "openai_compatible" => {
            build_deepseek_openai_request_with_tools(
                endpoint, &messages, max_tokens, true, tools_json,
            )
        }
        NativeModelFamily::DeepSeek => build_deepseek_anthropic_request_with_tools(
            endpoint, &messages, max_tokens, true, tools_json,
        ),
        NativeModelFamily::Qwen => {
            build_qwen_openai_request_with_tools(endpoint, &messages, max_tokens, true, tools_json)
        }
    }
}

fn native_tool_evidence_continuation_prompt(
    prompt: &str,
    continuation_view: &ContinuationView,
    extra_hint: &str,
) -> String {
    let current_batch = continuation_view.current_legacy_batch();
    let write_directive = native_loop_write_directive_for_prompt(prompt)
        .map(|directive| format!("\n\n# Runtime Write Directive\n{directive}"))
        .unwrap_or_default();
    let history_digest = continuation_view.history_digest_text();
    let history_section = if history_digest.trim().is_empty() {
        String::new()
    } else {
        format!("\n\n# Prior Evidence Digest\n{history_digest}")
    };
    let suppression_section = if continuation_view.suppressed_count == 0 {
        String::new()
    } else {
        format!(
            "\n\n# Suppressed Duplicate Observations\n{} duplicate observation(s) were suppressed this turn and are intentionally omitted from provider tool_result replay.",
            continuation_view.suppressed_count
        )
    };
    format!(
        "{prompt}{write_directive}\n\n# Already Executed Tool Evidence\n{}{history_section}{suppression_section}\n\n# Continuation Instruction\n{extra_hint}",
        compact_tool_evidence_summary(&current_batch, 8, 800)
    )
}

pub(in crate::native_agent_loop) fn deepseek_reasoning_replay_for_tool_continuation(
    reasoning_content: Option<&str>,
) -> String {
    let Some(reasoning) = reasoning_content
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return "[reasoning_content omitted by provider; placeholder retained so DeepSeek thinking tool replay remains well-formed]".to_string();
    };
    let max_chars = 48_000usize;
    if reasoning.chars().count() <= max_chars {
        reasoning.to_string()
    } else {
        let prefix = reasoning.chars().take(max_chars).collect::<String>();
        format!(
            "{prefix}\n[reasoning_content truncated by ResearchCode to stay within the 12K-token replay budget]"
        )
    }
}

pub(in crate::native_agent_loop) fn build_native_tool_result_continuation_request(
    endpoint: &NativeProviderEndpoint,
    prompt: &str,
    continuation_view: &ContinuationView,
    max_tokens: u64,
    tools_json: &str,
    deepseek_reasoning_content: Option<&str>,
) -> Result<PreparedModelHttpRequest, String> {
    let continuation_messages = continuation_messages_for_provider_replay(
        endpoint,
        prompt,
        continuation_view,
        deepseek_reasoning_content,
    );
    if continuation_messages.len() <= 1 {
        return Err("tool_result continuation requires a non-empty tool batch".to_string());
    }
    match endpoint.family {
        NativeModelFamily::DeepSeek if endpoint.protocol == "openai_compatible" => {
            let tool_calls = deepseek_openai_tool_calls_from_messages(&continuation_messages);
            let tool_results = deepseek_openai_tool_results_from_messages(&continuation_messages);
            let reasoning_replay =
                deepseek_reasoning_replay_for_tool_continuation(deepseek_reasoning_content);
            build_deepseek_openai_multi_tool_result_request_with_reasoning(
                endpoint,
                &native_loop_system_prompt(
                    &endpoint.family,
                    &endpoint.protocol,
                    &NativeAgentToolExposure::FastAutoWrite,
                    Some(tools_json),
                    None,
                ),
                prompt,
                &tool_calls,
                &tool_results,
                max_tokens,
                true,
                Some(reasoning_replay.as_str()),
            )
        }
        NativeModelFamily::DeepSeek => {
            let tool_uses = deepseek_tool_uses_from_messages(&continuation_messages);
            let tool_results = deepseek_tool_results_from_messages(&continuation_messages);
            build_deepseek_anthropic_multi_tool_result_request_with_thinking(
                endpoint,
                &native_loop_system_prompt(
                    &endpoint.family,
                    &endpoint.protocol,
                    &NativeAgentToolExposure::FastAutoWrite,
                    Some(tools_json),
                    None,
                ),
                prompt,
                &tool_uses,
                &tool_results,
                max_tokens,
                true,
                tools_json,
                None,
                None,
            )
        }
        NativeModelFamily::Qwen => {
            let tool_calls = qwen_tool_calls_from_messages(&continuation_messages);
            let tool_results = qwen_tool_results_from_messages(&continuation_messages);
            build_qwen_openai_multi_tool_result_request(
                endpoint,
                "ResearchCode Qwen3.6-27B native loop v2 tool_result continuation. Keep thinking metadata separate from visible output. Use the advertised tool schema only.",
                prompt,
                &tool_calls,
                &tool_results,
                max_tokens,
                true,
            )
        }
    }
}

fn continuation_messages_for_provider_replay(
    endpoint: &NativeProviderEndpoint,
    prompt: &str,
    continuation_view: &ContinuationView,
    deepseek_reasoning_content: Option<&str>,
) -> Vec<ConversationMessage> {
    let openai_style_ids =
        endpoint.family == NativeModelFamily::Qwen || endpoint.protocol == "openai_compatible";
    let tool_calls = continuation_view
        .current_legacy_batch()
        .into_iter()
        .map(|(tool_use_id, tool_name, tool_input_json, result)| {
            let id = if openai_style_ids {
                provider_openai_tool_call_id(&tool_use_id)
            } else {
                tool_use_id
            };
            let tool_id = match endpoint.family {
                NativeModelFamily::DeepSeek if endpoint.protocol != "openai_compatible" => {
                    provider_tool_name_for_deepseek(&tool_name)
                }
                _ => provider_tool_name_for_id(&tool_name),
            };
            (
                ConversationToolCall {
                    id: id.clone(),
                    tool_id,
                    arguments_json: tool_input_json,
                },
                (id, structured_tool_result_content(&result), !result.ok),
            )
        })
        .collect::<Vec<_>>();
    let result_error_flags = tool_calls
        .iter()
        .map(|(_, (_, _, is_error))| *is_error)
        .collect::<Vec<_>>();
    let calls = tool_calls
        .iter()
        .map(|(call, _)| call.clone())
        .collect::<Vec<_>>();
    let results = tool_calls
        .into_iter()
        .map(|(_, (id, content, _))| (id, content))
        .collect::<Vec<_>>();
    let mut messages = conversation_messages_from_tool_result_continuation(
        prompt,
        calls,
        results,
        deepseek_reasoning_content.map(|value| value.to_string()),
    );
    for (message, is_error) in messages
        .iter_mut()
        .filter(|message| message.role == "tool")
        .zip(result_error_flags)
    {
        if is_error {
            let content = message.content.get_or_insert_with(String::new);
            content.push_str(&format!(
                "\n{{\"{TOOL_RESULT_ERROR_META_KEY}\":{{\"is_error\":true}}}}"
            ));
        }
    }
    messages
}

fn split_tool_result_error_meta(content: &str) -> (String, bool) {
    let trimmed_end = content.trim_end();
    let Some((body, maybe_meta)) = trimmed_end.rsplit_once('\n') else {
        return (content.to_string(), false);
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(maybe_meta.trim()) else {
        return (content.to_string(), false);
    };
    let is_error = value
        .get(TOOL_RESULT_ERROR_META_KEY)
        .and_then(|meta| meta.get("is_error"))
        .and_then(|flag| flag.as_bool())
        .unwrap_or(false);
    if is_error {
        (body.to_string(), true)
    } else {
        (content.to_string(), false)
    }
}

fn assistant_tool_calls(
    messages: &[ConversationMessage],
) -> impl Iterator<Item = &ConversationToolCall> {
    messages
        .iter()
        .filter(|message| message.role == "assistant")
        .flat_map(|message| message.tool_calls.iter())
}

fn tool_messages(messages: &[ConversationMessage]) -> impl Iterator<Item = &ConversationMessage> {
    messages.iter().filter(|message| message.role == "tool")
}

fn deepseek_openai_tool_calls_from_messages(
    messages: &[ConversationMessage],
) -> Vec<DeepSeekOpenAiToolCallBlock> {
    assistant_tool_calls(messages)
        .map(|call| DeepSeekOpenAiToolCallBlock {
            id: call.id.clone(),
            name: call.tool_id.clone(),
            arguments_json: call.arguments_json.clone(),
        })
        .collect()
}

fn deepseek_openai_tool_results_from_messages(
    messages: &[ConversationMessage],
) -> Vec<DeepSeekOpenAiToolResultBlock> {
    tool_messages(messages)
        .map(|message| DeepSeekOpenAiToolResultBlock {
            tool_call_id: message.tool_call_id.clone().unwrap_or_default(),
            content: message.content.clone().unwrap_or_default(),
        })
        .collect()
}

fn deepseek_tool_uses_from_messages(
    messages: &[ConversationMessage],
) -> Vec<DeepSeekAnthropicToolUseBlock> {
    assistant_tool_calls(messages)
        .map(|call| DeepSeekAnthropicToolUseBlock {
            id: call.id.clone(),
            name: call.tool_id.clone(),
            input_json: call.arguments_json.clone(),
        })
        .collect()
}

fn deepseek_tool_results_from_messages(
    messages: &[ConversationMessage],
) -> Vec<DeepSeekAnthropicToolResultBlock> {
    tool_messages(messages)
        .map(|message| {
            let content = message.content.clone().unwrap_or_default();
            let (content, is_error) = split_tool_result_error_meta(&content);
            DeepSeekAnthropicToolResultBlock {
                tool_use_id: message.tool_call_id.clone().unwrap_or_default(),
                is_error,
                content,
            }
        })
        .collect()
}

fn qwen_tool_calls_from_messages(messages: &[ConversationMessage]) -> Vec<QwenOpenAiToolCallBlock> {
    assistant_tool_calls(messages)
        .map(|call| QwenOpenAiToolCallBlock {
            id: call.id.clone(),
            name: call.tool_id.clone(),
            arguments_json: call.arguments_json.clone(),
        })
        .collect()
}

fn qwen_tool_results_from_messages(
    messages: &[ConversationMessage],
) -> Vec<QwenOpenAiToolResultBlock> {
    tool_messages(messages)
        .map(|message| QwenOpenAiToolResultBlock {
            tool_call_id: message.tool_call_id.clone().unwrap_or_default(),
            content: message.content.clone().unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_tool_result_error_flag_uses_private_tail_meta_only() {
        let messages = vec![ConversationMessage {
            role: "tool".to_string(),
            content: Some(format!(
                "stdout legitimately contains {{\"is_error\":true}}\n{{\"{TOOL_RESULT_ERROR_META_KEY}\":{{\"is_error\":true}}}}"
            )),
            tool_call_id: Some("toolu_1".to_string()),
            tool_calls: Vec::new(),
            reasoning_preview: None,
        }];

        let results = deepseek_tool_results_from_messages(&messages);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_error);
        assert_eq!(
            results[0].content,
            "stdout legitimately contains {\"is_error\":true}"
        );
    }

    #[test]
    fn deepseek_tool_result_does_not_false_positive_on_plain_is_error_json() {
        let messages = vec![ConversationMessage {
            role: "tool".to_string(),
            content: Some("stdout legitimately contains {\"is_error\":true}".to_string()),
            tool_call_id: Some("toolu_1".to_string()),
            tool_calls: Vec::new(),
            reasoning_preview: None,
        }];

        let results = deepseek_tool_results_from_messages(&messages);
        assert_eq!(results.len(), 1);
        assert!(!results[0].is_error);
        assert_eq!(
            results[0].content,
            "stdout legitimately contains {\"is_error\":true}"
        );
    }
}
