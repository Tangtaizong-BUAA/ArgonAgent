//! Recorded native agent loop.
//!
//! This module is the first model-output-driven executor loop. It still uses
//! recorded provider bodies, but it exercises the same runtime contracts that a
//! live DeepSeek/Qwen executor must use: model response recording, native parser
//! gates, tool scheduling, read-before-write patch validation, permission
//! events, command execution, artifacts, and final review.

use crate::artifact::ArtifactStore;
use crate::live_http_transport::{
    run_live_model_http_once, LiveModelHttpRunRequest, RecordedLiveHttpTransport,
};
use crate::live_model_executor::LiveModelExecutionRequest;
use crate::live_model_request::ModelRequestMessage;
use crate::model_adapter::{
    DeepSeekNativeAdapter, ModelAdapter, ModelAdapterRequest, ModelRole, PlannedModelCall,
    QwenNativeAdapter,
};
use crate::native_provider::NativeProviderEndpoint;
use crate::parser::{
    classify_deepseek_output, classify_qwen_output, ParsedToolIntent, ParserAction,
};
use crate::patch::{stable_text_hash, validate_patch, PatchCheck, PatchValidation};
use crate::provider_response_adapter::{
    record_native_provider_response, NativeProviderResponseInput, NativeProviderStreamKind,
};
use crate::session::AgentSession;
use crate::state::AgentState;
use crate::tcml::{
    normalize_tool_id, parse_first_tool_call, parse_tool_arguments, ParsedToolArguments,
};
use crate::tool_dispatcher::{schedule_tool_calls, ScheduledToolCall};
use crate::tool_execution::{
    execute_tool, ToolExecutionArgs, ToolExecutionMode, ToolExecutionRequest,
};
use crate::tool_result::{write_tool_result_artifact, ToolResultRecord};
use researchcode_kernel::model::{NativeModelFamily, NativeModelProfile, OptimizationLevel};
use researchcode_kernel::{PermissionDecisionKind, PermissionRequestType};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedNativeProvider {
    DeepSeek,
    Qwen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedAgentStep {
    pub provider: RecordedNativeProvider,
    pub role: ModelRole,
    pub raw_response: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedAgentLoopConfig {
    pub project_id: String,
    pub session_id: String,
    pub task_id: String,
    pub cleanup: bool,
}

impl Default for RecordedAgentLoopConfig {
    fn default() -> Self {
        Self {
            project_id: "proj".to_string(),
            session_id: "sess_recorded_agent_loop".to_string(),
            task_id: "task".to_string(),
            cleanup: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedAgentLoopResult {
    pub final_state: AgentState,
    pub event_count: usize,
    pub final_file_hash: String,
    pub command_exit_code: i32,
    pub event_jsonl: String,
}

pub fn run_recorded_agent_loop_fixture(
    config: &RecordedAgentLoopConfig,
) -> Result<RecordedAgentLoopResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-recorded-agent-loop-{nonce}"));
    let src = root.join("src");
    fs::create_dir_all(&src).map_err(|error| error.to_string())?;
    let target = src.join("parser.ts");
    fs::write(&target, "export const retry_count = 3;\n").map_err(|error| error.to_string())?;
    let artifact_store = ArtifactStore::new(root.join("artifacts"));
    let mut session = AgentSession::new(&config.project_id, &config.session_id, &config.task_id)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .map_err(|error| format!("{error:?}"))?;

    let steps = vec![
        RecordedAgentStep {
            provider: RecordedNativeProvider::DeepSeek,
            role: ModelRole::Planner,
            raw_response:
                r#"{"tool_calls":[{"name":"file.read","arguments":{"path":"src/parser.ts"}}]}"#
                    .to_string(),
        },
        RecordedAgentStep {
            provider: RecordedNativeProvider::Qwen,
            role: ModelRole::Executor,
            raw_response: r#"{"model":"Qwen/Qwen3.6-27B","tool_calls":[{"name":"patch.propose","arguments":{"path":"src/parser.ts","old_string":"retry_count = 3","new_string":"retry_count = 5"}}]}"#.to_string(),
        },
        RecordedAgentStep {
            provider: RecordedNativeProvider::DeepSeek,
            role: ModelRole::Reviewer,
            raw_response:
                r#"{"tool_calls":[{"name":"shell.command","arguments":{"command":"find . -maxdepth 0"}}]}"#
                    .to_string(),
        },
    ];

    let mut command_exit_code = -1;
    for (index, step) in steps.iter().enumerate() {
        let plan = planned_call_for_step(step)?;
        record_step_model_response(&mut session, &artifact_store, index, step, &plan)?;
        let parsed = classify_step(step);
        execute_parsed_step(
            &mut session,
            &artifact_store,
            &root,
            index,
            &step.raw_response,
            parsed,
            &mut command_exit_code,
        )?;
    }

    session
        .start_review()
        .and_then(|_| session.complete_after_review())
        .map_err(|error| format!("{error:?}"))?;
    let final_text = fs::read_to_string(&target).map_err(|error| error.to_string())?;
    if final_text != "export const retry_count = 5;\n" {
        return Err(format!("recorded loop final file mismatch: {final_text}"));
    }
    let result = RecordedAgentLoopResult {
        final_state: session.state(),
        event_count: session.event_count(),
        final_file_hash: stable_text_hash(&final_text),
        command_exit_code,
        event_jsonl: session.export_events_jsonl(),
    };
    if config.cleanup {
        let _ = fs::remove_dir_all(&root);
    }
    Ok(result)
}

pub fn run_live_transport_agent_loop_fixture(
    config: &RecordedAgentLoopConfig,
) -> Result<RecordedAgentLoopResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-live-transport-agent-loop-{nonce}"));
    let src = root.join("src");
    fs::create_dir_all(&src).map_err(|error| error.to_string())?;
    let target = src.join("parser.ts");
    fs::write(&target, "export const retry_count = 3;\n").map_err(|error| error.to_string())?;
    let artifact_store = ArtifactStore::new(root.join("artifacts"));
    let mut session = AgentSession::new(&config.project_id, &config.session_id, &config.task_id)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .map_err(|error| format!("{error:?}"))?;

    let steps = vec![
        LiveTransportAgentStep {
            provider: RecordedNativeProvider::DeepSeek,
            role: ModelRole::Planner,
            response_body: r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"name":"file.read","arguments":"{\"path\":\"src/parser.ts\"}"}}]}}]}
data: {"usage":{"prompt_tokens":32,"completion_tokens":16,"reasoning_tokens":0,"prompt_cache_hit_tokens":8,"prompt_cache_miss_tokens":2}}
data: [DONE]"#.to_string(),
        },
        LiveTransportAgentStep {
            provider: RecordedNativeProvider::Qwen,
            role: ModelRole::Executor,
            response_body: r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"tool_calls":[{"function":{"name":"patch.propose","arguments":"{\"path\":\"src/parser.ts\",\"old_string\":\"retry_count = 3\",\"new_string\":\"retry_count = 5\"}"}}]}}]}
data: {"usage":{"prompt_tokens":32,"completion_tokens":16,"total_tokens":48}}
data: [DONE]"#.to_string(),
        },
        LiveTransportAgentStep {
            provider: RecordedNativeProvider::DeepSeek,
            role: ModelRole::Reviewer,
            response_body: r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"name":"shell.command","arguments":"{\"command\":\"find . -maxdepth 0\"}"}}]}}]}
data: {"usage":{"prompt_tokens":32,"completion_tokens":16,"reasoning_tokens":0,"prompt_cache_hit_tokens":8,"prompt_cache_miss_tokens":2}}
data: [DONE]"#.to_string(),
        },
    ];

    let mut command_exit_code = -1;
    for (index, step) in steps.iter().enumerate() {
        let plan = planned_call_for_step(&RecordedAgentStep {
            provider: step.provider.clone(),
            role: step.role.clone(),
            raw_response: String::new(),
        })?;
        let endpoint = live_endpoint_for_step(step);
        let response = run_live_model_http_once(
            &mut session,
            &artifact_store,
            &RecordedLiveHttpTransport {
                status_code: 200,
                body: step.response_body.clone(),
            },
            LiveModelHttpRunRequest {
                execution: LiveModelExecutionRequest {
                    call_id: format!("live_loop_call_{index}"),
                    role: model_role_to_str(&step.role).to_string(),
                    endpoint,
                    messages: vec![ModelRequestMessage {
                        role: "user".to_string(),
                        content: "Continue the agent loop".to_string(),
                        cache_control_ttl: None,
                    }],
                    max_tokens: 1024,
                    stream: true,
                    tools_json: None,
                    live_calls_enabled: true,
                    network_approved: true,
                },
                stream_id: &format!("live_loop_stream_{index}"),
                role: step.role.clone(),
                plan: &plan,
                request_preview: "live transport agent loop request",
                transcript_id: &format!("live_loop_transcript_{index}"),
            },
        )?;
        let visible_content = response
            .response
            .as_ref()
            .ok_or_else(|| format!("live transport loop step {index} produced no response"))?
            .visible_content_preview
            .clone();
        let parsed = match step.provider {
            RecordedNativeProvider::DeepSeek => classify_deepseek_output(&visible_content),
            RecordedNativeProvider::Qwen => classify_qwen_output(&visible_content),
        };
        execute_parsed_step(
            &mut session,
            &artifact_store,
            &root,
            index,
            &visible_content,
            parsed,
            &mut command_exit_code,
        )?;
    }

    session
        .start_review()
        .and_then(|_| session.complete_after_review())
        .map_err(|error| format!("{error:?}"))?;
    let final_text = fs::read_to_string(&target).map_err(|error| error.to_string())?;
    if final_text != "export const retry_count = 5;\n" {
        return Err(format!(
            "live transport loop final file mismatch: {final_text}"
        ));
    }
    let result = RecordedAgentLoopResult {
        final_state: session.state(),
        event_count: session.event_count(),
        final_file_hash: stable_text_hash(&final_text),
        command_exit_code,
        event_jsonl: session.export_events_jsonl(),
    };
    if config.cleanup {
        let _ = fs::remove_dir_all(&root);
    }
    Ok(result)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveTransportAgentStep {
    provider: RecordedNativeProvider,
    role: ModelRole,
    response_body: String,
}

fn planned_call_for_step(step: &RecordedAgentStep) -> Result<PlannedModelCall, String> {
    match step.provider {
        RecordedNativeProvider::DeepSeek => DeepSeekNativeAdapter::new(
            NativeModelProfile {
                profile_id: "deepseek-v4-native".to_string(),
                family: NativeModelFamily::DeepSeek,
                optimization_level: OptimizationLevel::Native,
            },
            "deepseek-v4-flash",
        )?
        .plan_call(&ModelAdapterRequest {
            role: step.role.clone(),
            task_summary: "recorded agent loop".to_string(),
            requires_tools: true,
            context_tokens_estimate: 2_000,
        }),
        RecordedNativeProvider::Qwen => QwenNativeAdapter::new(
            NativeModelProfile {
                profile_id: "qwen3-6-27b-native".to_string(),
                family: NativeModelFamily::Qwen,
                optimization_level: OptimizationLevel::Native,
            },
            "Qwen/Qwen3.6-27B",
        )?
        .plan_call(&ModelAdapterRequest {
            role: step.role.clone(),
            task_summary: "recorded agent loop".to_string(),
            requires_tools: true,
            context_tokens_estimate: 2_000,
        }),
    }
}

fn live_endpoint_for_step(step: &LiveTransportAgentStep) -> NativeProviderEndpoint {
    match step.provider {
        RecordedNativeProvider::DeepSeek => {
            let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
            endpoint.live_calls_enabled_by_default = true;
            endpoint.api_key_env = "PATH".to_string();
            endpoint
        }
        RecordedNativeProvider::Qwen => {
            let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
            endpoint.live_calls_enabled_by_default = true;
            endpoint.api_key_env = "PATH".to_string();
            endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
            endpoint
        }
    }
}

fn model_role_to_str(role: &ModelRole) -> &'static str {
    match role {
        ModelRole::Planner => "planner",
        ModelRole::Executor => "executor",
        ModelRole::Reviewer => "reviewer",
        ModelRole::Researcher => "researcher",
        ModelRole::Summarizer => "summarizer",
    }
}

fn record_step_model_response(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    index: usize,
    step: &RecordedAgentStep,
    plan: &PlannedModelCall,
) -> Result<(), String> {
    let provider = match step.provider {
        RecordedNativeProvider::DeepSeek => NativeProviderStreamKind::DeepSeek,
        RecordedNativeProvider::Qwen => NativeProviderStreamKind::Qwen,
    };
    record_native_provider_response(
        session,
        artifact_store,
        NativeProviderResponseInput {
            provider,
            call_id: &format!("recorded_loop_call_{index}"),
            stream_id: &format!("recorded_loop_stream_{index}"),
            role: step.role.clone(),
            plan,
            request_preview: "recorded agent loop request",
            transcript_id: &format!("recorded_loop_transcript_{index}"),
            live: false,
            visible_content: &step.raw_response,
            hidden_reasoning_sanitized: None,
            prompt_tokens: 32,
            completion_tokens: 16,
            reasoning_tokens: 0,
            prompt_cache_hit_tokens: 0,
            prompt_cache_miss_tokens: 0,
        },
    )?;
    Ok(())
}

fn classify_step(step: &RecordedAgentStep) -> ParsedToolIntent {
    match step.provider {
        RecordedNativeProvider::DeepSeek => classify_deepseek_output(&step.raw_response),
        RecordedNativeProvider::Qwen => classify_qwen_output(&step.raw_response),
    }
}

fn execute_parsed_step(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    root: &Path,
    index: usize,
    raw: &str,
    parsed: ParsedToolIntent,
    command_exit_code: &mut i32,
) -> Result<(), String> {
    match parsed.action {
        ParserAction::Execute
        | ParserAction::RepairThenExecute
        | ParserAction::ExecuteWithReasoningSanitizer
        | ParserAction::ExecuteWithReasoningRedaction => {}
        other => {
            return Err(format!(
                "recorded loop step {index} cannot execute action {other:?}"
            ))
        }
    }
    let tool_call = parse_first_tool_call(raw)
        .ok_or_else(|| format!("recorded loop step {index} produced no structured tool call"))?;
    let tool_id = normalize_tool_id(&tool_call.tool_id);
    if let Some(policy_tool_id) = parsed.tool_id {
        if policy_tool_id != tool_call.tool_id {
            return Err(format!(
                "recorded loop step {index} policy/parser tool mismatch: {policy_tool_id} != {}",
                tool_call.tool_id
            ));
        }
    }
    let arguments = parse_tool_arguments(&tool_call.arguments_json);
    schedule_tool_calls(vec![ScheduledToolCall {
        tool_call_id: format!("recorded_loop_tool_{index}"),
        tool_id: tool_id.clone(),
    }])
    .map_err(|error| format!("{error:?}"))?;
    match tool_id.as_str() {
        "file.read" | "file.list_directory" | "file.list_tree" | "search.ripgrep" | "repo.map" => {
            execute_read_only_tool(session, artifact_store, root, index, &tool_id, &arguments)
        }
        "patch.apply" => execute_patch(session, artifact_store, root, index, &arguments),
        "shell.command" => {
            *command_exit_code = execute_command(session, artifact_store, root, index, &arguments)?;
            Ok(())
        }
        other => Err(format!("unsupported recorded loop tool {other}")),
    }
}

fn execute_read_only_tool(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    root: &Path,
    index: usize,
    tool_id: &str,
    arguments: &ParsedToolArguments,
) -> Result<(), String> {
    let tool_call_id = format!("recorded_loop_tool_{index}");
    let execution_args = recorded_tool_args(tool_id, arguments);
    session
        .record_tool_call_requested(&tool_call_id, tool_id)
        .map_err(|error| format!("{error:?}"))?;
    let result = execute_tool(&ToolExecutionRequest {
        workspace_root: root.to_path_buf(),
        tool_call_id: tool_call_id.clone(),
        tool_id: tool_id.to_string(),
        mode: ToolExecutionMode::ReadOnlyPreview,
        args: execution_args,
    })
    .map_err(|error| format!("{error:?}"))?;
    session
        .record_tool_call_completed(&tool_call_id, tool_id, true)
        .map_err(|error| format!("{error:?}"))?;
    let artifact = write_tool_result_artifact(
        artifact_store,
        &format!("recorded_loop_read_only_{index}"),
        &ToolResultRecord::new(
            &tool_call_id,
            tool_id,
            true,
            result.preview.clone(),
            result.detail_json,
        ),
    )
    .map_err(|error| error.to_string())?;
    session
        .record_tool_result_artifact(
            &tool_call_id,
            tool_id,
            artifact.artifact_id,
            artifact.content_hash,
            result.preview,
        )
        .map_err(|error| format!("{error:?}"))
}

fn recorded_tool_args(tool_id: &str, arguments: &ParsedToolArguments) -> ToolExecutionArgs {
    match tool_id {
        "file.read" => ToolExecutionArgs {
            path: Some(
                arguments
                    .path
                    .clone()
                    .unwrap_or_else(|| "src/parser.ts".to_string()),
            ),
            max_bytes: Some(4096),
            ..ToolExecutionArgs::default()
        },
        "file.list_directory" => ToolExecutionArgs {
            path: arguments
                .path
                .clone()
                .or_else(|| arguments.root.clone())
                .or_else(|| Some(".".to_string())),
            max_results: Some(120),
            include_hidden: Some(false),
            ..ToolExecutionArgs::default()
        },
        "file.list_tree" => ToolExecutionArgs {
            path: arguments
                .path
                .clone()
                .or_else(|| arguments.root.clone())
                .or_else(|| Some(".".to_string())),
            max_results: Some(200),
            max_depth: Some(3),
            ..ToolExecutionArgs::default()
        },
        "search.ripgrep" => ToolExecutionArgs {
            root: arguments.root.clone().or_else(|| Some(".".to_string())),
            pattern: arguments
                .pattern
                .clone()
                .or_else(|| arguments.query.clone())
                .or_else(|| Some("retry_count".to_string())),
            max_results: Some(20),
            ..ToolExecutionArgs::default()
        },
        "repo.map" => ToolExecutionArgs {
            root: arguments
                .root
                .clone()
                .or_else(|| arguments.path.clone())
                .or_else(|| Some(".".to_string())),
            max_files: Some(80),
            max_depth: Some(4),
            ..ToolExecutionArgs::default()
        },
        _ => ToolExecutionArgs::default(),
    }
}

fn execute_patch(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    root: &Path,
    index: usize,
    arguments: &ParsedToolArguments,
) -> Result<(), String> {
    let tool_call_id = format!("recorded_loop_tool_{index}");
    let patch_id = format!("recorded_loop_patch_{index}");
    let path = root.join(
        arguments
            .path
            .clone()
            .unwrap_or_else(|| "src/parser.ts".to_string()),
    );
    let old_string = arguments
        .old_string
        .clone()
        .unwrap_or_else(|| "retry_count = 3".to_string());
    let new_string = arguments
        .new_string
        .clone()
        .unwrap_or_else(|| "retry_count = 5".to_string());
    let current_text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let base_hash = stable_text_hash(&current_text);
    let validation = validate_patch(PatchCheck {
        path: &path.to_string_lossy(),
        current_text: Some(&current_text),
        current_hash: Some(&base_hash),
        old_string: &old_string,
        base_hash: &base_hash,
    });
    session
        .record_tool_call_requested(&tool_call_id, "patch.apply")
        .and_then(|_| session.record_patch_proposal_created(&patch_id, path.to_string_lossy()))
        .and_then(|_| session.record_patch_proposal_validated(&patch_id, validation.clone()))
        .map_err(|error| format!("{error:?}"))?;
    if validation != PatchValidation::Pass {
        session
            .record_tool_call_completed(&tool_call_id, "patch.apply", false)
            .map_err(|error| format!("{error:?}"))?;
        return Err(format!(
            "recorded loop patch validation failed: {validation:?}"
        ));
    }
    session
        .request_permission(
            &format!("recorded_loop_patch_perm_{index}"),
            PermissionRequestType::FileWrite,
            None,
        )
        .and_then(|_| session.decide_permission(PermissionDecisionKind::AllowOnce))
        .map_err(|error| format!("{error:?}"))?;
    let result = execute_tool(&ToolExecutionRequest {
        workspace_root: root.to_path_buf(),
        tool_call_id: tool_call_id.clone(),
        tool_id: "patch.apply".to_string(),
        mode: ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        args: ToolExecutionArgs {
            path: arguments
                .path
                .clone()
                .or_else(|| Some("src/parser.ts".to_string())),
            old_string: Some(old_string),
            new_string: Some(new_string),
            base_hash: Some(base_hash),
            ..ToolExecutionArgs::default()
        },
    })
    .map_err(|error| format!("{error:?}"))?;
    session
        .record_patch_applied(&patch_id, path.to_string_lossy())
        .and_then(|_| session.record_tool_call_completed(&tool_call_id, "patch.apply", result.ok))
        .map_err(|error| format!("{error:?}"))?;
    let artifact = write_tool_result_artifact(
        artifact_store,
        &format!("recorded_loop_patch_{index}"),
        &ToolResultRecord::new(
            &tool_call_id,
            "patch.apply",
            true,
            result.preview.clone(),
            result.detail_json,
        ),
    )
    .map_err(|error| error.to_string())?;
    session
        .record_tool_result_artifact(
            &tool_call_id,
            "patch.apply",
            artifact.artifact_id,
            artifact.content_hash,
            result.preview,
        )
        .map_err(|error| format!("{error:?}"))
}

fn execute_command(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    root: &Path,
    index: usize,
    arguments: &ParsedToolArguments,
) -> Result<i32, String> {
    let tool_call_id = format!("recorded_loop_tool_{index}");
    let command = arguments
        .command
        .clone()
        .unwrap_or_else(|| "find . -maxdepth 0".to_string());
    session
        .record_tool_call_requested(&tool_call_id, "shell.command")
        .and_then(|_| {
            session.request_permission(
                &format!("recorded_loop_command_perm_{index}"),
                PermissionRequestType::Command,
                None,
            )
        })
        .and_then(|_| session.decide_permission(PermissionDecisionKind::AllowOnce))
        .map_err(|error| format!("{error:?}"))?;
    let result = execute_tool(&ToolExecutionRequest {
        workspace_root: root.to_path_buf(),
        tool_call_id: tool_call_id.clone(),
        tool_id: "shell.command".to_string(),
        mode: ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        args: ToolExecutionArgs {
            command: Some(command),
            ..ToolExecutionArgs::default()
        },
    })
    .map_err(|error| format!("{error:?}"))?;
    let artifact = write_tool_result_artifact(
        artifact_store,
        &format!("recorded_loop_command_{index}"),
        &ToolResultRecord::new(
            &tool_call_id,
            "shell.command",
            result.ok,
            result.preview.clone(),
            result.detail_json,
        ),
    )
    .map_err(|error| error.to_string())?;
    session
        .record_tool_call_completed(&tool_call_id, "shell.command", result.ok)
        .and_then(|_| {
            session.record_tool_result_artifact(
                &tool_call_id,
                "shell.command",
                artifact.artifact_id,
                artifact.content_hash,
                result.preview,
            )
        })
        .map_err(|error| format!("{error:?}"))?;
    Ok(result.exit_code.unwrap_or(-1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_agent_loop_runs_model_output_to_tools() {
        let result = run_recorded_agent_loop_fixture(&RecordedAgentLoopConfig::default()).unwrap();
        assert_eq!(result.final_state, AgentState::Completed);
        assert_eq!(result.command_exit_code, 0);
        let jsonl = result.event_jsonl;
        assert!(jsonl.contains("\"event_type\":\"model.call_started\""));
        assert!(jsonl.contains("\"event_type\":\"tool.call_requested\""));
        assert!(jsonl.contains("\"event_type\":\"patch.applied\""));
        assert!(jsonl.contains("\"event_type\":\"model.call_completed\""));
        assert!(!jsonl.contains("sk-"));
        assert!(!jsonl.contains(".env"));
    }

    #[test]
    fn recorded_agent_loop_can_execute_repo_map_tool() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-recorded-repo-map-{nonce}"));
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        fs::write(src.join("lib.rs"), "pub fn demo() {}\n").unwrap();
        let artifact_store = ArtifactStore::new(root.join("artifacts"));
        let mut session = AgentSession::new("proj", "sess_repo_map", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        let mut command_exit_code = -1;
        execute_parsed_step(
            &mut session,
            &artifact_store,
            &root,
            0,
            r#"{"tool_calls":[{"name":"repo.map","arguments":{"root":"."}}]}"#,
            ParsedToolIntent {
                action: ParserAction::Execute,
                tool_id: Some("repo.map".to_string()),
            },
            &mut command_exit_code,
        )
        .unwrap();
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"tool_id\":\"repo.map\""));
        assert!(jsonl.contains("repo map files="));
        assert_eq!(command_exit_code, -1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recorded_agent_loop_can_execute_search_tool() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-recorded-search-{nonce}"));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/parser.ts"),
            "export const retry_count = 3;\n",
        )
        .unwrap();
        let artifact_store = ArtifactStore::new(root.join("artifacts"));
        let mut session = AgentSession::new("proj", "sess_search", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        let mut command_exit_code = -1;
        execute_parsed_step(
            &mut session,
            &artifact_store,
            &root,
            0,
            r#"{"tool_calls":[{"name":"search.ripgrep","arguments":{"pattern":"retry_count","root":"."}}]}"#,
            ParsedToolIntent {
                action: ParserAction::Execute,
                tool_id: Some("search.ripgrep".to_string()),
            },
            &mut command_exit_code,
        )
        .unwrap();
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"tool_id\":\"search.ripgrep\""));
        assert!(jsonl.contains("matches"));
        assert!(!jsonl.contains("sk-"));
        assert!(!jsonl.contains(".env"));
        assert_eq!(command_exit_code, -1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn live_transport_agent_loop_runs_response_to_tools() {
        let result =
            run_live_transport_agent_loop_fixture(&RecordedAgentLoopConfig::default()).unwrap();
        assert_eq!(result.final_state, AgentState::Completed);
        assert_eq!(result.command_exit_code, 0);
        let jsonl = result.event_jsonl;
        assert!(jsonl.contains("\"live\":true"));
        assert!(jsonl.contains("\"event_type\":\"model.call_started\""));
        assert!(jsonl.contains("\"event_type\":\"tool.call_requested\""));
        assert!(jsonl.contains("\"event_type\":\"patch.applied\""));
        assert!(jsonl.contains("\"event_type\":\"model.call_completed\""));
        assert_eq!(
            jsonl
                .matches("\"event_type\":\"model.call_started\"")
                .count(),
            3
        );
        assert!(!jsonl.contains("sk-"));
        assert!(!jsonl.contains(".env"));
    }
}
