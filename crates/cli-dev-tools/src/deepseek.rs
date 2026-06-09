#![allow(unused_imports, dead_code)]

use crate::agent_smokes::*;
use crate::core_smokes::*;
use crate::fixtures::*;
use crate::helpers::*;
use crate::live_model::*;
use crate::prelude::*;
use crate::qwen_tools::*;
use crate::runtime_smokes::*;
pub(crate) fn deepseek_sidecar_live_cli(output_path: Option<PathBuf>) -> Result<(), String> {
    let live_enabled = env::var("RESEARCHCODE_ENABLE_LIVE_PROVIDER").unwrap_or_default() == "1";
    let network_approved = env::var("RESEARCHCODE_ALLOW_NETWORK").unwrap_or_default() == "1";
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = env::temp_dir().join(format!("researchcode-deepseek-sidecar-live-{nonce}"));
    let store = ArtifactStore::new(root.join("artifacts"));
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = live_enabled;
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
        task_summary: "Optional live DeepSeek sidecar smoke".to_string(),
        requires_tools: false,
        context_tokens_estimate: 2_000,
    })?;
    let live_messages = native_live_smoke_messages(
        NativeModelFamily::DeepSeek,
        ModelRole::Planner,
        &plan,
        "Optional live DeepSeek sidecar smoke: inspect project context and reply with a concise next-step summary.",
    );
    let mut session = AgentSession::new("proj", "sess_deepseek_sidecar_live", "task")
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .map_err(|error| format!("{error:?}"))?;
    let result = run_live_model_http_once(
        &mut session,
        &store,
        &PythonSidecarLiveHttpTransport::default_workspace_sidecar(),
        LiveModelHttpRunRequest {
            execution: LiveModelExecutionRequest {
                call_id: "deepseek_sidecar_live_call_1".to_string(),
                role: "planner".to_string(),
                endpoint: endpoint.clone(),
                messages: live_messages,
                max_tokens: 64,
                stream: true,
                tools_json: None,
                live_calls_enabled: live_enabled,
                network_approved,
            },
            stream_id: "deepseek_sidecar_live_stream_1",
            role: ModelRole::Planner,
            plan: &plan,
            request_preview: "optional live DeepSeek sidecar smoke",
            transcript_id: "deepseek_sidecar_live_transcript_1",
        },
    );
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(output_path, session.export_events_jsonl()).map_err(|error| error.to_string())?;
    }
    match result {
        Ok(result) => match result.status {
            LiveModelHttpRunStatus::Blocked => {
                let gate = result.gate.as_ref().map(gate_to_str).unwrap_or("unknown");
                println!(
                    "deepseek sidecar live skipped gate={} events={}",
                    gate,
                    session.event_count()
                );
                let _ = fs::remove_dir_all(root);
                Ok(())
            }
            LiveModelHttpRunStatus::HttpFailed => Err(format!(
                "deepseek sidecar live HTTP failed status={:?} preview={}",
                result.http_status_code,
                result.http_error_preview.unwrap_or_default()
            )),
            LiveModelHttpRunStatus::Completed => {
                let jsonl = session.export_events_jsonl();
                if jsonl.contains("sk-") || jsonl.contains("api_key") || jsonl.contains(".env") {
                    return Err("deepseek sidecar live leaked sensitive event content".to_string());
                }
                let response = result
                    .response
                    .as_ref()
                    .ok_or_else(|| "missing live sidecar response".to_string())?;
                println!(
                    "deepseek sidecar live completed events={} hash={} tokens={}/{} cache={}/{}",
                    session.event_count(),
                    response.content_hash,
                    response.prompt_tokens,
                    response.completion_tokens,
                    response.prompt_cache_hit_tokens,
                    response.prompt_cache_miss_tokens
                );
                let _ = fs::remove_dir_all(root);
                Ok(())
            }
        },
        Err(error)
            if error.contains("network_not_enabled") || error.contains("missing_api_key") =>
        {
            println!("deepseek sidecar live skipped: {error}");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn deepseek_stream_visible_cli(prompt: String) -> Result<(), String> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    deepseek_stream_visible_to_writer(prompt, &mut writer)
}

pub(crate) fn deepseek_stream_visible_to_writer<W: Write>(
    prompt: String,
    writer: &mut W,
) -> Result<(), String> {
    let live_enabled = env::var("RESEARCHCODE_ENABLE_LIVE_PROVIDER").unwrap_or_default() == "1";
    let network_approved = env::var("RESEARCHCODE_ALLOW_NETWORK").unwrap_or_default() == "1";
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = live_enabled;
    let gate = evaluate_native_live_call_gate(&endpoint, live_enabled, network_approved);
    if gate != researchcode_runtime::native_provider::NativeLiveCallGate::Allowed {
        return Err(format!(
            "deepseek stream blocked gate={}. Set RESEARCHCODE_ENABLE_LIVE_PROVIDER=1 and RESEARCHCODE_ALLOW_NETWORK=1, and provide DEEPSEEK_API_KEY.",
            gate_to_str(&gate)
        ));
    }
    let max_tokens = deepseek_tui_max_tokens_for_task(&prompt);
    let request = build_deepseek_anthropic_request(
        &endpoint,
        &[
            ModelRequestMessage {
                role: "system".to_string(),
                content: "You are ResearchCode DeepSeek native mode. Reply with visible user-facing text only. Keep DeepSeek thinking separate from visible output.".to_string(),
                cache_control_ttl: None,
            },
            ModelRequestMessage {
                role: "user".to_string(),
                content: prompt,
                cache_control_ttl: None,
            },
        ],
        max_tokens,
        true,
    )?;
    deepseek_stream_prepared_to_writer(&request, writer)
}

pub(crate) fn deepseek_stream_tool_visible_cli() -> Result<(), String> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let live_enabled = env::var("RESEARCHCODE_ENABLE_LIVE_PROVIDER").unwrap_or_default() == "1";
    let network_approved = env::var("RESEARCHCODE_ALLOW_NETWORK").unwrap_or_default() == "1";
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = live_enabled;
    let gate = evaluate_native_live_call_gate(&endpoint, live_enabled, network_approved);
    if gate != researchcode_runtime::native_provider::NativeLiveCallGate::Allowed {
        return Err(format!(
            "deepseek stream tool blocked gate={}. Set RESEARCHCODE_ENABLE_LIVE_PROVIDER=1 and RESEARCHCODE_ALLOW_NETWORK=1, and provide DEEPSEEK_API_KEY.",
            gate_to_str(&gate)
        ));
    }
    let request = build_deepseek_anthropic_request_with_tools(
        &endpoint,
        &[
            ModelRequestMessage {
                role: "system".to_string(),
                content: "You are ResearchCode DeepSeek native mode. Use the provided tool when the user asks to read a file. Do not invent file contents.".to_string(),
                cache_control_ttl: None,
            },
            ModelRequestMessage {
                role: "user".to_string(),
                content: "Use file.read to inspect README.md, then stop after emitting the tool call.".to_string(),
                cache_control_ttl: None,
            },
        ],
        1024,
        true,
        &native_readonly_provider_tool_schema_json(),
    )?;
    deepseek_stream_prepared_to_writer(&request, &mut writer)
}

pub(crate) fn deepseek_tool_loop_fixture_smoke() -> Result<(), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = env::temp_dir().join(format!("researchcode-deepseek-tool-loop-{nonce}"));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    fs::write(root.join("README.md"), "DeepSeek tool loop fixture\n")
        .map_err(|error| error.to_string())?;
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"file_read","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"README.md\",\"max_bytes\":256}"}}
data: {"usage":{"input_tokens":24,"output_tokens":9,"reasoning_tokens":3,"cache_read_input_tokens":10,"cache_creation_input_tokens":2}}
data: {"type":"message_stop"}
"#
        .to_string(),
    }, LiveHttpResponse {
        status_code: 200,
        body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Read README.md and verified the fixture."}}
data: {"type":"message_stop"}
"#
        .to_string(),
    }]);
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
    endpoint.live_calls_enabled_by_default = true;
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: "sess_deepseek_tool_loop_fixture".to_string(),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: root.clone(),
        artifact_root: root.join("artifacts"),
        endpoint,
        prompt: "Use file_read to inspect README.md.".to_string(),
        max_tokens: 256,
        max_iterations: 2,
        max_tool_calls: 1,
        tool_exposure: NativeAgentToolExposure::ReadOnly,
        permission_mode: PermissionMode::Default,
        provided_permission_decisions: Vec::new(),
        deepseek_adaptation: None,
        error_recovery: None,
        hook_dispatcher: None,
        concurrent_tool_execution: false,
    };
    let result = AgentKernel::for_request(&request).run_turn(&transport, request, None)?;
    if result.status != NativeAgentLoopStatus::Completed {
        let _ = fs::remove_dir_all(&root);
        return Err(format!(
            "deepseek tool loop fixture expected completed, got {:?}",
            result.status
        ));
    }
    if result.tool_call_count != 1 || !result.event_jsonl.contains("\"tool_id\":\"file.read\"") {
        let _ = fs::remove_dir_all(&root);
        return Err("deepseek tool loop fixture did not execute mapped file.read".to_string());
    }
    if result.event_jsonl.contains("sk-") || result.event_jsonl.contains("api_key") {
        let _ = fs::remove_dir_all(&root);
        return Err("deepseek tool loop fixture leaked secret-like content".to_string());
    }
    let _ = fs::remove_dir_all(&root);
    println!(
        "deepseek tool loop fixture smoke status={:?} events={} tools={} models={}",
        result.status, result.event_count, result.tool_call_count, result.model_call_count
    );
    Ok(())
}

pub(crate) fn deepseek_tool_result_continuation_smoke() -> Result<(), String> {
    let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
    let request = build_deepseek_anthropic_tool_result_request(
        &endpoint,
        "You are ResearchCode DeepSeek native mode. Continue after tool_result with visible text only. Do not replay reasoning as a user message.",
        "Use file_read to inspect README.md, then summarize the result.",
        "toolu_researchcode_1",
        "file_read",
        "{\"path\":\"README.md\",\"max_bytes\":256}",
        "README.md: ResearchCode native agent fixture",
        256,
        true,
        &native_readonly_provider_tool_schema_json(),
    )?;
    let has_raw_key_like_secret =
        request.body_json.contains("sk-") && !request.body_json.contains("task-governance");
    if !request.body_json.contains("\"type\":\"tool_result\"")
        || !request
            .body_json
            .contains("\"tool_use_id\":\"toolu_researchcode_1\"")
        || has_raw_key_like_secret
        || request.body_json.contains("api_key")
    {
        return Err(format!(
            "DeepSeek tool_result continuation request shape failed has_tool_result={} has_tool_use_id={} has_secret={} has_api_key_word={}",
            request.body_json.contains("\"type\":\"tool_result\""),
            request
                .body_json
                .contains("\"tool_use_id\":\"toolu_researchcode_1\""),
            has_raw_key_like_secret,
            request.body_json.contains("api_key")
        ));
    }
    println!(
        "deepseek tool_result continuation smoke body_bytes={} stream={}",
        request.body_json.len(),
        request.stream
    );
    Ok(())
}

pub(crate) fn deepseek_agent_live_cli(
    prompt: String,
    output_path: Option<PathBuf>,
    expect_tool_call: bool,
) -> Result<(), String> {
    let live_enabled = env::var("RESEARCHCODE_ENABLE_LIVE_PROVIDER").unwrap_or_default() == "1";
    let network_approved = env::var("RESEARCHCODE_ALLOW_NETWORK").unwrap_or_default() == "1";
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = live_enabled;
    let provider_ready = live_enabled
        && network_approved
        && env::var(&endpoint.api_key_env)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let artifact_root = env::temp_dir().join(format!("researchcode-deepseek-agent-live-{nonce}"));
    let interrupt = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let result = if provider_ready {
        RuntimeFacade::run_deepseek_agent_loop_request_with_interrupt(
            &PythonSidecarLiveHttpTransport::default_workspace_sidecar(),
            NativeAgentLoopV2Request {
                project_id: "proj".to_string(),
                session_id: format!("sess_deepseek_agent_live_v2_{nonce}"),
                task_id: "task".to_string(),
                turn_id: None,
                workspace_root: PathBuf::from("."),
                artifact_root: artifact_root.clone(),
                endpoint,
                prompt,
                max_tokens: 8_192,
                max_iterations: 16,
                max_tool_calls: 64,
                tool_exposure: NativeAgentToolExposure::ReadOnly,
                permission_mode: PermissionMode::Default,
                provided_permission_decisions: Vec::new(),
                deepseek_adaptation: None,
                error_recovery: None,
                hook_dispatcher: None,
                concurrent_tool_execution: false,
            },
            None,
            &*interrupt,
        )?
    } else {
        let request = NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: format!("sess_deepseek_agent_live_{nonce}"),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: PathBuf::from("."),
            artifact_root: artifact_root.clone(),
            endpoint,
            prompt,
            max_tokens: 8_192,
            max_iterations: 1,
            max_tool_calls: 1,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        };
        AgentKernel::for_request(&request).run_turn(
            &PythonSidecarLiveHttpTransport::default_workspace_sidecar(),
            request,
            None,
        )?
    };
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, &result.event_jsonl).map_err(|error| error.to_string())?;
        println!("deepseek agent live eventlog={}", path.display());
    }
    if expect_tool_call
        && provider_ready
        && result.status == NativeAgentLoopStatus::Completed
        && result.tool_call_count == 0
    {
        let _ = fs::remove_dir_all(&artifact_root);
        return Err("deepseek agent live completed but did not execute any tool call".to_string());
    }
    if result.event_jsonl.contains("sk-") || result.event_jsonl.contains("api_key") {
        let _ = fs::remove_dir_all(&artifact_root);
        return Err("deepseek agent live leaked secret-like event content".to_string());
    }
    println!(
        "deepseek agent live status={:?} provider_ready={} events={} models={} tools={}",
        result.status,
        provider_ready,
        result.event_count,
        result.model_call_count,
        result.tool_call_count
    );
    let _ = fs::remove_dir_all(&artifact_root);
    Ok(())
}

pub(crate) fn deepseek_stream_prepared_to_writer<W: Write>(
    request: &PreparedModelHttpRequest,
    writer: &mut W,
) -> Result<(), String> {
    let sidecar_input = sidecar_stream_visible_input_json(request);
    let mut child = ProcessCommand::new("python3")
        .arg(workspace_provider_sidecar_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("stream sidecar spawn failed: {error}"))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "stream sidecar stdin unavailable".to_string())?;
        stdin
            .write_all(sidecar_input.as_bytes())
            .map_err(|error| format!("stream sidecar stdin write failed: {error}"))?;
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stream sidecar stdout unavailable".to_string())?;
    let mut visible_chars = 0usize;
    let mut reasoning_events = 0usize;
    let mut tool_events = 0usize;
    let mut input_tokens = None;
    let mut output_tokens = None;
    let reader = io::BufReader::new(stdout);
    writeln!(writer, "[DeepSeek visible stream]").map_err(|error| error.to_string())?;
    for line_result in reader.lines() {
        let line = line_result.map_err(|error| format!("stream sidecar read failed: {error}"))?;
        match extract_json_string_field_cli(&line, "event").as_deref() {
            Some("text") => {
                if let Some(delta) = extract_json_string_field_cli(&line, "delta") {
                    visible_chars += delta.chars().count();
                    write!(writer, "{delta}").map_err(|error| error.to_string())?;
                    writer.flush().map_err(|error| error.to_string())?;
                }
            }
            Some("reasoning_sanitized") => {
                reasoning_events += 1;
            }
            Some("tool_call") => {
                tool_events += 1;
                let name = extract_json_string_field_cli(&line, "name")
                    .unwrap_or_else(|| "unknown".to_string());
                writeln!(writer, "\n[tool_call name={}]", name)
                    .map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
            }
            Some("tool_arguments_delta") => {
                tool_events += 1;
            }
            Some("usage") => {
                input_tokens = extract_json_u64_field_cli(&line, "input_tokens");
                output_tokens = extract_json_u64_field_cli(&line, "output_tokens");
            }
            Some("http_error") => {
                let status = extract_json_u64_field_cli(&line, "status_code").unwrap_or(0);
                let preview = extract_json_string_field_cli(&line, "preview")
                    .map(|value| format!(" {}", truncate_for_panel(&value, 160)))
                    .unwrap_or_default();
                writeln!(
                    writer,
                    "╭─ ModelStreamPanel\n│ deepseek stream HTTP failed status={}{}\n╰",
                    status, preview
                )
                .map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
                let _ = child.wait();
                return Err(format!("deepseek stream HTTP failed status={status}"));
            }
            Some("skipped") => {
                let reason = extract_json_string_field_cli(&line, "reason")
                    .unwrap_or_else(|| "unknown".to_string());
                writeln!(
                    writer,
                    "╭─ ModelStreamPanel\n│ deepseek stream skipped: {}\n╰",
                    reason
                )
                .map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
                let _ = child.wait();
                return Err(format!("deepseek stream skipped: {reason}"));
            }
            _ => {}
        }
    }
    let status = child
        .wait()
        .map_err(|error| format!("stream sidecar wait failed: {error}"))?;
    if !status.success() {
        return Err(format!("deepseek stream sidecar exited with {status}"));
    }
    writeln!(
        writer,
        "\n[done visible_chars={} reasoning_events={} tool_events={} tokens={}/{}]",
        visible_chars,
        reasoning_events,
        tool_events,
        input_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        output_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}
