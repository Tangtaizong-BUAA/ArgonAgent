#![allow(unused_imports, dead_code)]

use crate::agent_smokes::*;
use crate::core_smokes::*;
use crate::deepseek::*;
use crate::fixtures::*;
use crate::helpers::*;
use crate::live_model::*;
use crate::prelude::*;
use crate::runtime_smokes::*;
pub(crate) fn qwen_tool_result_continuation_smoke() -> Result<(), String> {
    let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
    endpoint.base_url = env::var("QWEN_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8000/v1/chat/completions".to_string());
    let request = build_qwen_openai_tool_result_request(
        &endpoint,
        "You are ResearchCode Qwen3.6-27B native mode. Keep thinking metadata separate from visible output.",
        "Use file_read to inspect README.md, then summarize the result.",
        "call_researchcode_1",
        "file_read",
        "{\"path\":\"README.md\",\"max_bytes\":256}",
        "README.md: ResearchCode native agent fixture",
        256,
        true,
    )?;
    if !request.body_json.contains("\"tool_calls\"")
        || !request.body_json.contains("\"role\":\"tool\"")
        || !request
            .body_json
            .contains("\"tool_call_id\":\"call_researchcode_1\"")
        || request.body_json.contains("sk-")
        || request.body_json.contains("api_key")
    {
        return Err("Qwen tool_result continuation request shape failed".to_string());
    }
    println!(
        "qwen tool_result continuation smoke body_bytes={} stream={}",
        request.body_json.len(),
        request.stream
    );
    Ok(())
}

pub(crate) fn qwen_sidecar_live_cli(output_path: Option<PathBuf>) -> Result<(), String> {
    let live_enabled = env::var("RESEARCHCODE_ENABLE_LIVE_PROVIDER").unwrap_or_default() == "1";
    let network_approved = env::var("RESEARCHCODE_ALLOW_NETWORK").unwrap_or_default() == "1";
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = env::temp_dir().join(format!("researchcode-qwen-sidecar-live-{nonce}"));
    let store = ArtifactStore::new(root.join("artifacts"));
    let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
    endpoint.live_calls_enabled_by_default = live_enabled;
    if let Ok(base_url) = env::var("QWEN_BASE_URL") {
        if !base_url.trim().is_empty() {
            endpoint.base_url = base_url;
        }
    }
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
        task_summary: "Optional live Qwen sidecar smoke".to_string(),
        requires_tools: false,
        context_tokens_estimate: 2_000,
    })?;
    let live_messages = native_live_smoke_messages(
        NativeModelFamily::Qwen,
        ModelRole::Executor,
        &plan,
        "Optional live Qwen sidecar smoke: inspect project context and reply with a concise next-step summary.",
    );
    let mut session = AgentSession::new("proj", "sess_qwen_sidecar_live", "task")
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
                call_id: "qwen_sidecar_live_call_1".to_string(),
                role: "executor".to_string(),
                endpoint,
                messages: live_messages,
                max_tokens: 64,
                stream: true,
                tools_json: None,
                live_calls_enabled: live_enabled,
                network_approved,
            },
            stream_id: "qwen_sidecar_live_stream_1",
            role: ModelRole::Executor,
            plan: &plan,
            request_preview: "optional live Qwen sidecar smoke",
            transcript_id: "qwen_sidecar_live_transcript_1",
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
                    "qwen sidecar live skipped gate={} events={}",
                    gate,
                    session.event_count()
                );
                let _ = fs::remove_dir_all(root);
                Ok(())
            }
            LiveModelHttpRunStatus::HttpFailed => Err(format!(
                "qwen sidecar live HTTP failed status={:?} preview={}",
                result.http_status_code,
                result.http_error_preview.unwrap_or_default()
            )),
            LiveModelHttpRunStatus::Completed => {
                let jsonl = session.export_events_jsonl();
                if jsonl.contains("sk-") || jsonl.contains("api_key") || jsonl.contains(".env") {
                    return Err("qwen sidecar live leaked sensitive event content".to_string());
                }
                let response = result
                    .response
                    .as_ref()
                    .ok_or_else(|| "missing live sidecar response".to_string())?;
                println!(
                    "qwen sidecar live completed events={} hash={} tokens={}/{}",
                    session.event_count(),
                    response.content_hash,
                    response.prompt_tokens,
                    response.completion_tokens
                );
                let _ = fs::remove_dir_all(root);
                Ok(())
            }
        },
        Err(error)
            if error.contains("network_not_enabled") || error.contains("missing_api_key") =>
        {
            println!("qwen sidecar live skipped: {error}");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn native_live_smoke_messages(
    family: NativeModelFamily,
    role: ModelRole,
    plan: &researchcode_runtime::model_adapter::PlannedModelCall,
    task: &str,
) -> Vec<ModelRequestMessage> {
    let model_family = match family {
        NativeModelFamily::DeepSeek => "deepseek",
        NativeModelFamily::Qwen => "qwen",
    };
    let mut builder = ContextBundleBuilder::new("live_sidecar_context", model_family, 16_000);
    builder.add_user_task(task);
    if let Ok(repo_map) = build_repo_map(&RepoMapRequest {
        root: PathBuf::from("."),
        max_files: 40,
        max_depth: 3,
    }) {
        builder.add_repo_map(&repo_map);
    }
    let context = builder.build();
    let prompt = assemble_native_prompt(NativePromptRequest {
        family,
        role,
        plan,
        context: &context,
        tools: &core_tool_specs(),
    });
    native_prompt_messages(&prompt)
}

pub(crate) fn parser_action_to_str(action: ParserAction) -> &'static str {
    match action {
        ParserAction::Execute => "execute",
        ParserAction::RepairThenExecute => "repair_then_execute",
        ParserAction::Retry => "retry",
        ParserAction::Deny => "deny",
        ParserAction::NoTool => "no_tool",
        ParserAction::PermissionRequiredThenDenyByPolicy => {
            "permission_required_then_deny_by_policy"
        }
        ParserAction::PermissionRequiredPackageInstall => "permission_required_package_install",
        ParserAction::BlockNativeSession => "block_native_session",
        ParserAction::ExecuteWithReasoningSanitizer => "execute_with_reasoning_sanitizer",
        ParserAction::ExecuteWithReasoningRedaction => "execute_with_reasoning_redaction",
        ParserAction::ExecuteOnlyAfterFileReadHash => "execute_only_after_file_read_hash",
        ParserAction::PatchValidatorMustRejectAmbiguousMatch => {
            "patch_validator_must_reject_ambiguous_match"
        }
    }
}

pub(crate) fn tool_execution_smoke() -> Result<(), String> {
    let repo_map = execute_tool(&ToolExecutionRequest {
        workspace_root: PathBuf::from("."),
        tool_call_id: "cli_repo_map".to_string(),
        tool_id: "repo.map".to_string(),
        mode: ToolExecutionMode::ReadOnlyPreview,
        args: ToolExecutionArgs {
            max_files: Some(40),
            max_depth: Some(3),
            ..ToolExecutionArgs::default()
        },
    })
    .map_err(|error| format!("{error:?}"))?;
    let search = execute_tool(&ToolExecutionRequest {
        workspace_root: PathBuf::from("."),
        tool_call_id: "cli_search".to_string(),
        tool_id: "search.ripgrep".to_string(),
        mode: ToolExecutionMode::ReadOnlyPreview,
        args: ToolExecutionArgs {
            root: Some("crates".to_string()),
            pattern: Some("ToolSpec".to_string()),
            max_results: Some(8),
            ..ToolExecutionArgs::default()
        },
    })
    .map_err(|error| format!("{error:?}"))?;
    let denied = execute_tool_preview(&ToolExecutionRequest {
        workspace_root: PathBuf::from("."),
        tool_call_id: "cli_shell".to_string(),
        tool_id: "shell.command".to_string(),
        mode: ToolExecutionMode::ReadOnlyPreview,
        args: ToolExecutionArgs::default(),
    });
    let shell_allowed = execute_tool(&ToolExecutionRequest {
        workspace_root: PathBuf::from("."),
        tool_call_id: "cli_shell_allowed".to_string(),
        tool_id: "shell.command".to_string(),
        mode: ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        args: ToolExecutionArgs {
            command: Some("find . -maxdepth 0".to_string()),
            ..ToolExecutionArgs::default()
        },
    })
    .map_err(|error| format!("{error:?}"))?;
    let patch_root = std::env::temp_dir().join(format!(
        "researchcode-cli-tool-exec-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    fs::create_dir_all(patch_root.join("src")).map_err(|error| error.to_string())?;
    let patch_path = patch_root.join("src/lib.rs");
    fs::write(&patch_path, "pub const RETRY: u8 = 3;\n").map_err(|error| error.to_string())?;
    let stale_patch = execute_tool(&ToolExecutionRequest {
        workspace_root: patch_root.clone(),
        tool_call_id: "cli_patch_stale".to_string(),
        tool_id: "patch.apply".to_string(),
        mode: ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        args: ToolExecutionArgs {
            path: Some("src/lib.rs".to_string()),
            old_string: Some("RETRY: u8 = 3".to_string()),
            new_string: Some("RETRY: u8 = 5".to_string()),
            base_hash: Some("stale_hash".to_string()),
            ..ToolExecutionArgs::default()
        },
    });
    let base_hash = stable_text_hash("pub const RETRY: u8 = 3;\n");
    let patch = execute_tool(&ToolExecutionRequest {
        workspace_root: patch_root.clone(),
        tool_call_id: "cli_patch".to_string(),
        tool_id: "patch.apply".to_string(),
        mode: ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        args: ToolExecutionArgs {
            path: Some("src/lib.rs".to_string()),
            old_string: Some("RETRY: u8 = 3".to_string()),
            new_string: Some("RETRY: u8 = 5".to_string()),
            base_hash: Some(base_hash),
            ..ToolExecutionArgs::default()
        },
    })
    .map_err(|error| format!("{error:?}"))?;
    let patched_text = fs::read_to_string(&patch_path).map_err(|error| error.to_string())?;
    let _ = fs::remove_dir_all(&patch_root);
    let research_output_dir = std::env::temp_dir().join(format!(
        "researchcode-cli-tool-exec-research-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    let research = execute_tool(&ToolExecutionRequest {
        workspace_root: PathBuf::from("."),
        tool_call_id: "cli_research".to_string(),
        tool_id: "research.csv_profile".to_string(),
        mode: ToolExecutionMode::ReadOnlyPreview,
        args: ToolExecutionArgs {
            input_csv: Some("eval/fixtures/research/csv-quality-small/input.csv".to_string()),
            job_id: Some("cli_tool_execution_research".to_string()),
            output_dir: Some(research_output_dir.to_string_lossy().to_string()),
            ..ToolExecutionArgs::default()
        },
    })
    .map_err(|error| format!("{error:?}"))?;
    let _ = fs::remove_dir_all(&research_output_dir);
    println!(
        "tool execution repo_map={} search={} shell_denied={} shell_allowed={} stale_patch_denied={} patch_applied={} research_csv_profile={}",
        repo_map.ok,
        search.ok,
        denied.is_err(),
        shell_allowed.ok && shell_allowed.exit_code == Some(0),
        stale_patch.is_err(),
        patch.ok && patched_text.contains("RETRY: u8 = 5"),
        research.ok && research.preview.contains("artifacts=5")
    );
    Ok(())
}

pub(crate) fn fast_auto_policy_smoke() -> Result<(), String> {
    let root = env::temp_dir().join(format!(
        "researchcode-fast-auto-policy-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).map_err(|error| error.to_string())?;
    fs::write(root.join("README.md"), "FastAuto policy smoke\n")
        .map_err(|error| error.to_string())?;
    fs::write(root.join(".env"), "DEEPSEEK_API_KEY=sk-test\n")
        .map_err(|error| error.to_string())?;
    fs::write(root.join("src/lib.rs"), "pub const RETRY: u8 = 3;\n")
        .map_err(|error| error.to_string())?;
    let facade = RuntimeFacade::new(&root, root.join(".researchcode"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::FastAuto,
    )?;
    let read = facade.execute_session_tool(
        &handle.session_id,
        "fast_auto_read",
        "file.read",
        ToolExecutionArgs {
            path: Some("README.md".to_string()),
            max_bytes: Some(256),
            ..ToolExecutionArgs::default()
        },
    )?;
    let secret_read = facade.execute_session_tool(
        &handle.session_id,
        "fast_auto_secret_read",
        "file.read",
        ToolExecutionArgs {
            path: Some(".env".to_string()),
            max_bytes: Some(256),
            ..ToolExecutionArgs::default()
        },
    );
    let safe_command = facade.execute_session_tool(
        &handle.session_id,
        "fast_auto_find",
        "shell.command",
        ToolExecutionArgs {
            command: Some("find . -maxdepth 0".to_string()),
            ..ToolExecutionArgs::default()
        },
    )?;
    let package_install = facade.execute_session_tool(
        &handle.session_id,
        "fast_auto_package",
        "shell.command",
        ToolExecutionArgs {
            command: Some("npm install left-pad".to_string()),
            ..ToolExecutionArgs::default()
        },
    )?;
    let stale_patch = facade.execute_session_tool(
        &handle.session_id,
        "fast_auto_stale_patch",
        "patch.apply",
        ToolExecutionArgs {
            path: Some("src/lib.rs".to_string()),
            old_string: Some("RETRY: u8 = 3".to_string()),
            new_string: Some("RETRY: u8 = 5".to_string()),
            base_hash: Some("stale_hash".to_string()),
            ..ToolExecutionArgs::default()
        },
    );
    let current = fs::read_to_string(root.join("src/lib.rs")).map_err(|error| error.to_string())?;
    let good_patch = facade.execute_session_tool(
        &handle.session_id,
        "fast_auto_good_patch",
        "patch.apply",
        ToolExecutionArgs {
            path: Some("src/lib.rs".to_string()),
            old_string: Some("RETRY: u8 = 3".to_string()),
            new_string: Some("RETRY: u8 = 5".to_string()),
            base_hash: Some(stable_text_hash(&current)),
            ..ToolExecutionArgs::default()
        },
    )?;
    let event_path = root.join("events.jsonl");
    facade.export_events(&handle.session_id, &event_path)?;
    let jsonl = fs::read_to_string(&event_path).map_err(|error| error.to_string())?;
    let patched = fs::read_to_string(root.join("src/lib.rs")).map_err(|error| error.to_string())?;
    let secret_blocked = match secret_read {
        Err(_) => true,
        Ok(FacadeToolOutcome::Executed(result)) => {
            !result.ok && result.detail_json.contains("sensitive")
        }
        Ok(FacadeToolOutcome::BlockedByPolicy(_)) => true,
        Ok(_) => false,
    };
    let ok = matches!(read, FacadeToolOutcome::Executed(_))
        && secret_blocked
        && matches!(safe_command, FacadeToolOutcome::Executed(_))
        && matches!(package_install, FacadeToolOutcome::BlockedByPolicy(_))
        && stale_patch.is_err()
        && matches!(good_patch, FacadeToolOutcome::Executed(_))
        && patched.contains("RETRY: u8 = 5")
        && jsonl.contains("\"event_type\":\"permission.decided\"")
        && jsonl.contains("\"event_type\":\"patch.applied\"")
        && !jsonl.contains("sk-test");
    let _ = fs::remove_dir_all(&root);
    if !ok {
        return Err("fast auto policy smoke failed".to_string());
    }
    println!("fast auto policy smoke passed");
    Ok(())
}
