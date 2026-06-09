#![allow(unused_imports, dead_code)]

use crate::agent_smokes::*;
use crate::core_smokes::*;
use crate::deepseek::*;
use crate::fixtures::*;
use crate::helpers::*;
use crate::prelude::*;
use crate::qwen_tools::*;
use crate::runtime_smokes::*;
pub(crate) fn live_model_response_record_cli(output_path: Option<PathBuf>) -> Result<(), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = env::temp_dir().join(format!("researchcode-live-response-cli-{nonce}"));
    let store = ArtifactStore::new(root.join("artifacts"));
    let mut deepseek_endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
    deepseek_endpoint.live_calls_enabled_by_default = true;
    deepseek_endpoint.api_key_env = "PATH".to_string();
    let deepseek_adapter = DeepSeekNativeAdapter::new(
        researchcode_kernel::model::NativeModelProfile {
            profile_id: "deepseek-v4-native".to_string(),
            family: NativeModelFamily::DeepSeek,
            optimization_level: OptimizationLevel::Native,
        },
        "deepseek-v4-flash",
    )?;
    let deepseek_plan = deepseek_adapter.plan_call(&ModelAdapterRequest {
        role: ModelRole::Planner,
        task_summary: "Record a prepared live response".to_string(),
        requires_tools: false,
        context_tokens_estimate: 2_000,
    })?;
    let mut qwen_endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
    qwen_endpoint.live_calls_enabled_by_default = true;
    qwen_endpoint.api_key_env = "PATH".to_string();
    qwen_endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
    let qwen_adapter = QwenNativeAdapter::new(
        researchcode_kernel::model::NativeModelProfile {
            profile_id: "qwen3-6-27b-native".to_string(),
            family: NativeModelFamily::Qwen,
            optimization_level: OptimizationLevel::Native,
        },
        "Qwen/Qwen3.6-27B",
    )?;
    let qwen_plan = qwen_adapter.plan_call(&ModelAdapterRequest {
        role: ModelRole::Executor,
        task_summary: "Record a prepared live response".to_string(),
        requires_tools: false,
        context_tokens_estimate: 2_000,
    })?;
    let mut session = AgentSession::new("proj", "sess_live_response_record", "task")
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .map_err(|error| format!("{error:?}"))?;
    let prepared = prepare_live_model_execution(
        &mut session,
        &LiveModelExecutionRequest {
            call_id: "live_call_cli_1".to_string(),
            role: "planner".to_string(),
            endpoint: deepseek_endpoint.clone(),
            messages: vec![ModelRequestMessage {
                role: "user".to_string(),
                content: "Plan this task".to_string(),
                cache_control_ttl: None,
            }],
            max_tokens: 1024,
            stream: false,
            tools_json: None,
            live_calls_enabled: true,
            network_approved: true,
        },
    )
    .map_err(|error| format!("{error:?}"))?;
    if !matches!(
        prepared.status,
        researchcode_runtime::live_model_executor::LiveModelExecutionStatus::Prepared
    ) {
        return Err(format!(
            "expected prepared live call, got {:?}",
            prepared.status
        ));
    }
    let deepseek_result = record_live_model_response(
        &mut session,
        &store,
        LiveModelResponseRecordRequest {
            call_id: "live_call_cli_1",
            stream_id: "live_stream_cli_1",
            endpoint: &deepseek_endpoint,
            role: ModelRole::Planner,
            plan: &deepseek_plan,
            request_preview: "request preview",
            transcript_id: "live_transcript_cli_1",
            response_body: r#"{"model":"deepseek-v4-flash","content":[{"type":"text","text":"Visible live response"}],"reasoning_content":"Need sk-testsecret from .env","usage":{"input_tokens":12,"output_tokens":6,"reasoning_tokens":4,"cache_read_input_tokens":9,"cache_creation_input_tokens":3}}"#,
        },
    )?;
    let qwen_prepared = prepare_live_model_execution(
        &mut session,
        &LiveModelExecutionRequest {
            call_id: "live_call_cli_2".to_string(),
            role: "executor".to_string(),
            endpoint: qwen_endpoint.clone(),
            messages: vec![ModelRequestMessage {
                role: "user".to_string(),
                content: "Patch this task".to_string(),
                cache_control_ttl: None,
            }],
            max_tokens: 1024,
            stream: false,
            tools_json: None,
            live_calls_enabled: true,
            network_approved: true,
        },
    )
    .map_err(|error| format!("{error:?}"))?;
    if !matches!(
        qwen_prepared.status,
        researchcode_runtime::live_model_executor::LiveModelExecutionStatus::Prepared
    ) {
        return Err(format!(
            "expected prepared qwen live call, got {:?}",
            qwen_prepared.status
        ));
    }
    let qwen_result = record_live_model_response(
        &mut session,
        &store,
        LiveModelResponseRecordRequest {
            call_id: "live_call_cli_2",
            stream_id: "live_stream_cli_2",
            endpoint: &qwen_endpoint,
            role: ModelRole::Executor,
            plan: &qwen_plan,
            request_preview: "qwen request preview",
            transcript_id: "live_transcript_cli_2",
            response_body: r#"{"model":"Qwen/Qwen3.6-27B","choices":[{"message":{"reasoning_content":"Need sk-qwensecret from .env","content":"Visible Qwen live response"}}],"usage":{"prompt_tokens":22,"completion_tokens":8,"reasoning_tokens":5}}"#,
        },
    )?;
    let jsonl = session.export_events_jsonl();
    let call_started_count = jsonl
        .matches("\"event_type\":\"model.call_started\"")
        .count();
    if call_started_count != 2 {
        return Err(format!(
            "expected two model.call_started events, got {call_started_count}"
        ));
    }
    if jsonl.contains("sk-testsecret") || jsonl.contains("sk-qwensecret") || jsonl.contains(".env")
    {
        return Err("live response record leaked sensitive response content".to_string());
    }
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(output_path, &jsonl).map_err(|error| error.to_string())?;
    }
    println!(
        "live response record events={} deepseek={} qwen={} tokens={}/{} qwen_tokens={}/{} cache={}/{}",
        session.event_count(),
        deepseek_result.content_hash,
        qwen_result.content_hash,
        deepseek_result.prompt_tokens,
        deepseek_result.completion_tokens,
        qwen_result.prompt_tokens,
        qwen_result.completion_tokens,
        deepseek_result.prompt_cache_hit_tokens,
        deepseek_result.prompt_cache_miss_tokens
    );
    let _ = fs::remove_dir_all(root);
    Ok(())
}

pub(crate) fn live_http_transport_smoke() -> Result<(), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = env::temp_dir().join(format!("researchcode-live-http-cli-{nonce}"));
    let store = ArtifactStore::new(root.join("artifacts"));
    let mut deepseek_endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
    deepseek_endpoint.live_calls_enabled_by_default = true;
    deepseek_endpoint.api_key_env = "PATH".to_string();
    let deepseek_adapter = DeepSeekNativeAdapter::new(
        researchcode_kernel::model::NativeModelProfile {
            profile_id: "deepseek-v4-native".to_string(),
            family: NativeModelFamily::DeepSeek,
            optimization_level: OptimizationLevel::Native,
        },
        "deepseek-v4-flash",
    )?;
    let deepseek_plan = deepseek_adapter.plan_call(&ModelAdapterRequest {
        role: ModelRole::Planner,
        task_summary: "Run through injectable HTTP transport".to_string(),
        requires_tools: false,
        context_tokens_estimate: 2_000,
    })?;
    let mut qwen_endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
    qwen_endpoint.live_calls_enabled_by_default = true;
    qwen_endpoint.api_key_env = "PATH".to_string();
    qwen_endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
    let qwen_adapter = QwenNativeAdapter::new(
        researchcode_kernel::model::NativeModelProfile {
            profile_id: "qwen3-6-27b-native".to_string(),
            family: NativeModelFamily::Qwen,
            optimization_level: OptimizationLevel::Native,
        },
        "Qwen/Qwen3.6-27B",
    )?;
    let qwen_plan = qwen_adapter.plan_call(&ModelAdapterRequest {
        role: ModelRole::Executor,
        task_summary: "Run through injectable HTTP transport".to_string(),
        requires_tools: false,
        context_tokens_estimate: 2_000,
    })?;
    let mut session = AgentSession::new("proj", "sess_live_http_transport", "task")
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .map_err(|error| format!("{error:?}"))?;
    let deepseek = run_live_model_http_once(
        &mut session,
        &store,
        &RecordedLiveHttpTransport {
            status_code: 200,
            body: r#"{"model":"deepseek-v4-flash","content":[{"type":"text","text":"Visible DeepSeek HTTP response"}],"reasoning_content":"Need sk-httpsecret from .env","usage":{"input_tokens":14,"output_tokens":6,"reasoning_tokens":4,"cache_read_input_tokens":10,"cache_creation_input_tokens":4}}"#.to_string(),
        },
        LiveModelHttpRunRequest {
            execution: LiveModelExecutionRequest {
                call_id: "http_call_cli_1".to_string(),
                role: "planner".to_string(),
                endpoint: deepseek_endpoint,
                messages: vec![ModelRequestMessage {
                    role: "user".to_string(),
                    content: "Plan".to_string(),
                    cache_control_ttl: None,
                }],
                max_tokens: TUI_LIVE_DEEPSEEK_CHAT_MAX_TOKENS,
                stream: false,
                tools_json: None,
                    live_calls_enabled: true,
                network_approved: true,
            },
            stream_id: "http_stream_cli_1",
            role: ModelRole::Planner,
            plan: &deepseek_plan,
            request_preview: "request preview",
            transcript_id: "http_transcript_cli_1",
        },
    )?;
    let qwen = run_live_model_http_once(
        &mut session,
        &store,
        &RecordedLiveHttpTransport {
            status_code: 200,
            body: r#"{"model":"Qwen/Qwen3.6-27B","choices":[{"message":{"reasoning_content":"Need sk-qwenhttpsecret from .env","content":"Visible Qwen HTTP response"}}],"usage":{"prompt_tokens":20,"completion_tokens":8,"reasoning_tokens":5}}"#.to_string(),
        },
        LiveModelHttpRunRequest {
            execution: LiveModelExecutionRequest {
                call_id: "http_call_cli_2".to_string(),
                role: "executor".to_string(),
                endpoint: qwen_endpoint,
                messages: vec![ModelRequestMessage {
                    role: "user".to_string(),
                    content: "Patch".to_string(),
                    cache_control_ttl: None,
                }],
                max_tokens: 1024,
                stream: false,
                tools_json: None,
                    live_calls_enabled: true,
                network_approved: true,
            },
            stream_id: "http_stream_cli_2",
            role: ModelRole::Executor,
            plan: &qwen_plan,
            request_preview: "qwen request preview",
            transcript_id: "http_transcript_cli_2",
        },
    )?;
    if deepseek.status != LiveModelHttpRunStatus::Completed
        || qwen.status != LiveModelHttpRunStatus::Completed
    {
        return Err("recorded live HTTP transport did not complete both native calls".to_string());
    }
    let jsonl = session.export_events_jsonl();
    if jsonl.contains("sk-httpsecret")
        || jsonl.contains("sk-qwenhttpsecret")
        || jsonl.contains(".env")
    {
        return Err("live HTTP transport leaked sensitive response content".to_string());
    }
    println!(
        "live http transport events={} deepseek_status={:?} qwen_status={:?}",
        session.event_count(),
        deepseek.status,
        qwen.status
    );
    let _ = fs::remove_dir_all(root);
    Ok(())
}

pub(crate) fn provider_sidecar_smoke() -> Result<(), String> {
    let request = PreparedModelHttpRequest {
        method: "POST".to_string(),
        url: "https://api.deepseek.com/anthropic".to_string(),
        authorization_env: "RESEARCHCODE_TEST_MISSING_API_KEY".to_string(),
        body_json: "{\"model\":\"deepseek-v4-flash\",\"max_tokens\":16,\"stream\":false,\"messages\":[{\"role\":\"user\",\"content\":\"Reply OK\"}]}".to_string(),
        stream: false,
    };
    let transport = PythonSidecarLiveHttpTransport::default_workspace_sidecar();
    match transport.send(&request) {
        Ok(response) => {
            println!(
                "provider sidecar live status={} bytes={}",
                response.status_code,
                response.body.len()
            );
            Ok(())
        }
        Err(error)
            if error.contains("network_not_enabled") || error.contains("missing_api_key") =>
        {
            println!("provider sidecar skipped: {error}");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn provider_health_smoke() -> Result<(), String> {
    let requests = [
        (
            "deepseek",
            PreparedModelHttpRequest {
                method: "POST".to_string(),
                url: "https://api.deepseek.com/anthropic".to_string(),
                authorization_env: "DEEPSEEK_API_KEY".to_string(),
                body_json: "{\"model\":\"deepseek-v4-flash\",\"max_tokens\":16,\"stream\":false,\"messages\":[{\"role\":\"user\",\"content\":\"Reply OK\"}]}".to_string(),
                stream: false,
            },
        ),
        (
            "qwen",
            PreparedModelHttpRequest {
                method: "POST".to_string(),
                url: env::var("QWEN_BASE_URL").unwrap_or_else(|_| "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string()),
                authorization_env: "QWEN_API_KEY".to_string(),
                body_json: "{\"model\":\"Qwen/Qwen3.6-27B\",\"max_tokens\":16,\"stream\":false,\"messages\":[{\"role\":\"user\",\"content\":\"Reply OK\"}]}".to_string(),
                stream: false,
            },
        ),
    ];
    let transport = PythonSidecarLiveHttpTransport::default_workspace_sidecar();
    for (label, request) in requests {
        let report = transport.health_check(&request)?;
        let status = match report.status {
            ProviderSidecarHealthStatus::Skipped => "skipped",
            ProviderSidecarHealthStatus::Healthy => "healthy",
            ProviderSidecarHealthStatus::Unhealthy => "unhealthy",
        };
        println!(
            "provider health label={} status={} reason={} http_status={:?} target={}",
            label,
            status,
            report.reason.unwrap_or_else(|| "none".to_string()),
            report.http_status_code,
            report.target_kind.unwrap_or_else(|| "unknown".to_string())
        );
    }
    Ok(())
}

pub(crate) fn compatible_provider_request_smoke() -> Result<(), String> {
    let provider = CompatibleProviderConfig {
        provider_id: "sample-compatible".to_string(),
        schema_version: "v0".to_string(),
        display_name: "Sample Compatible".to_string(),
        protocol: "openai_compatible".to_string(),
        base_url: "http://127.0.0.1:8000/v1".to_string(),
        api_key_env: Some("SAMPLE_API_KEY".to_string()),
        actual_model_name: "sample-model".to_string(),
        display_model_name: "Sample Model".to_string(),
        model_alias: Some("sample".to_string()),
        capability_hints: ProviderCapabilityHints {
            supports_streaming: true,
            supports_tools: false,
            max_context_tokens: 32_000,
        },
        request_transform_id: Some("openai_chat_default_v0".to_string()),
        response_transform_id: Some("openai_chat_default_v0".to_string()),
        health_check: ProviderHealthCheck::default(),
        enabled_by_default: false,
        optimization_level: OptimizationLevel::Compatible,
    };
    let request = build_compatible_provider_request(&CompatibleProviderRequest {
        provider,
        messages: vec![ModelRequestMessage {
            role: "user".to_string(),
            content: "Reply OK".to_string(),
            cache_control_ttl: None,
        }],
        max_tokens: 32,
        stream: true,
    })?;
    if request.body_json.contains("deepseek")
        || request.body_json.contains("Qwen")
        || request.body_json.contains("native")
    {
        return Err("compatible provider request leaked native profile assumptions".to_string());
    }
    let response = normalize_compatible_provider_response(
        "openai_compatible",
        r#"{"choices":[{"message":{"content":"OK"}}],"usage":{"prompt_tokens":8,"completion_tokens":2}}"#,
    )?;
    println!(
        "compatible provider request method={} url={} auth_env={} stream={} body_chars={} response_parser={} response_tokens={}/{}",
        request.method,
        request.url,
        request.authorization_env,
        request.stream,
        request.body_json.len(),
        response.parser_profile,
        response.prompt_tokens,
        response.completion_tokens
    );
    Ok(())
}

pub(crate) fn native_prompt_smoke(family: &str) -> Result<(), String> {
    let mut builder = ContextBundleBuilder::new("prompt_smoke_bundle", family, 16_000);
    builder.add_user_task("Inspect the repo and propose the next safe implementation step.");
    if let Ok(repo_map) = build_repo_map(&RepoMapRequest {
        root: PathBuf::from("."),
        max_files: 40,
        max_depth: 3,
    }) {
        builder.add_repo_map(&repo_map);
    }
    let context = builder.build();
    match family {
        "deepseek" => {
            let adapter = DeepSeekNativeAdapter::new(
                researchcode_kernel::model::NativeModelProfile {
                    profile_id: "deepseek-v4-native".to_string(),
                    family: NativeModelFamily::DeepSeek,
                    optimization_level: OptimizationLevel::Native,
                },
                "deepseek-v4-flash",
            )?;
            let plan = adapter.plan_call(&ModelAdapterRequest {
                role: ModelRole::Planner,
                task_summary: "Assemble native prompt".to_string(),
                requires_tools: true,
                context_tokens_estimate: context.token_estimate(),
            })?;
            let prompt = assemble_native_prompt(NativePromptRequest {
                family: NativeModelFamily::DeepSeek,
                role: ModelRole::Planner,
                plan: &plan,
                context: &context,
                tools: &core_tool_specs(),
            });
            let messages = native_prompt_messages(&prompt);
            println!(
                "native prompt family=deepseek level={:?} estimated_tokens={} warnings={} system_chars={} user_chars={} messages={} parser={}",
                prompt.context_budget.scaffold_level,
                prompt.estimated_input_tokens,
                prompt.budget_warnings.len(),
                prompt.system_prompt.len(),
                prompt.user_prompt.len(),
                messages.len(),
                plan.parser_profile
            );
            Ok(())
        }
        "qwen" => {
            let adapter = QwenNativeAdapter::new(
                researchcode_kernel::model::NativeModelProfile {
                    profile_id: "qwen3-6-27b-native".to_string(),
                    family: NativeModelFamily::Qwen,
                    optimization_level: OptimizationLevel::Native,
                },
                "Qwen/Qwen3.6-27B",
            )?;
            let plan = adapter.plan_call(&ModelAdapterRequest {
                role: ModelRole::Executor,
                task_summary: "Assemble native prompt".to_string(),
                requires_tools: true,
                context_tokens_estimate: context.token_estimate(),
            })?;
            let prompt = assemble_native_prompt(NativePromptRequest {
                family: NativeModelFamily::Qwen,
                role: ModelRole::Executor,
                plan: &plan,
                context: &context,
                tools: &core_tool_specs(),
            });
            let messages = native_prompt_messages(&prompt);
            println!(
                "native prompt family=qwen level={:?} estimated_tokens={} warnings={} system_chars={} user_chars={} messages={} parser={}",
                prompt.context_budget.scaffold_level,
                prompt.estimated_input_tokens,
                prompt.budget_warnings.len(),
                prompt.system_prompt.len(),
                prompt.user_prompt.len(),
                messages.len(),
                plan.parser_profile
            );
            Ok(())
        }
        other => Err(format!("unknown native prompt family: {other}")),
    }
}
