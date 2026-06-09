#![allow(unused_imports, dead_code)]

use crate::agent_smokes::*;
use crate::core_smokes::*;
use crate::deepseek::*;
use crate::helpers::*;
use crate::live_model::*;
use crate::prelude::*;
use crate::qwen_tools::*;
use crate::runtime_smokes::*;
pub(crate) fn recorded_agent_loop_cli(output_path: Option<PathBuf>) -> Result<(), String> {
    let result = run_recorded_agent_loop_fixture(&RecordedAgentLoopConfig::default())?;
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(output_path, &result.event_jsonl).map_err(|error| error.to_string())?;
    }
    println!(
        "recorded agent loop completed state={:?} events={} file_hash={} command_exit={}",
        result.final_state, result.event_count, result.final_file_hash, result.command_exit_code
    );
    Ok(())
}

pub(crate) fn live_transport_agent_loop_cli(output_path: Option<PathBuf>) -> Result<(), String> {
    let result = run_live_transport_agent_loop_fixture(&RecordedAgentLoopConfig::default())?;
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(output_path, &result.event_jsonl).map_err(|error| error.to_string())?;
    }
    println!(
        "live transport agent loop completed state={:?} events={} file_hash={} command_exit={}",
        result.final_state, result.event_count, result.final_file_hash, result.command_exit_code
    );
    Ok(())
}

pub(crate) fn native_agent_loop_cli(output_path: Option<PathBuf>) -> Result<(), String> {
    let result = run_scripted_native_agent_loop_fixture()?;
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(output_path, &result.loop_result.event_jsonl)
            .map_err(|error| error.to_string())?;
    }
    println!(
        "native agent loop completed state={:?} events={} models={} tools={} file_hash={}",
        result.loop_result.final_state,
        result.loop_result.event_count,
        result.loop_result.model_call_count,
        result.loop_result.tool_call_count,
        result.final_file_hash
    );
    Ok(())
}

pub(crate) fn native_agent_loop_v2_cli(output_path: Option<PathBuf>) -> Result<(), String> {
    let result = run_scripted_native_agent_loop_v2_continuation_fixture()?;
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(output_path, &result.event_jsonl).map_err(|error| error.to_string())?;
    }
    println!(
        "native agent loop v2 completed state={:?} events={} models={} tools={}",
        result.final_state, result.event_count, result.model_call_count, result.tool_call_count
    );
    Ok(())
}

pub(crate) fn native_agent_loop_blocked_cli(output_path: Option<PathBuf>) -> Result<(), String> {
    let result = run_scripted_native_agent_loop_external_block_fixture()?;
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(output_path, &result.event_jsonl).map_err(|error| error.to_string())?;
    }
    println!(
        "native agent loop blocked status={:?} state={:?} events={} models={} tools={}",
        result.status,
        result.final_state,
        result.event_count,
        result.model_call_count,
        result.tool_call_count
    );
    Ok(())
}

pub(crate) fn native_agent_loop_resume_cli(output_path: Option<PathBuf>) -> Result<(), String> {
    let result = run_scripted_native_agent_loop_provided_permission_fixture()?;
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(output_path, &result.loop_result.event_jsonl)
            .map_err(|error| error.to_string())?;
    }
    println!(
        "native agent loop resume completed state={:?} events={} models={} tools={} file_hash={}",
        result.loop_result.final_state,
        result.loop_result.event_count,
        result.loop_result.model_call_count,
        result.loop_result.tool_call_count,
        result.final_file_hash
    );
    Ok(())
}

pub(crate) fn native_agent_loop_external_resume_cli(
    output_path: Option<PathBuf>,
) -> Result<(), String> {
    let result = run_scripted_native_agent_loop_external_resume_fixture()?;
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(output_path, &result.loop_result.event_jsonl)
            .map_err(|error| error.to_string())?;
    }
    println!(
        "native agent loop external resume completed state={:?} events={} models={} tools={} file_hash={}",
        result.loop_result.final_state,
        result.loop_result.event_count,
        result.loop_result.model_call_count,
        result.loop_result.tool_call_count,
        result.final_file_hash
    );
    Ok(())
}

pub(crate) fn native_agent_loop_export_pending_package_cli(
    package_dir: PathBuf,
) -> Result<(), String> {
    let package = write_scripted_native_agent_loop_external_decision_package(&package_dir)?;
    println!(
        "native pending package status={:?} events={} package={} pending={}",
        package.blocked_result.status,
        package.blocked_result.event_count,
        package.package_dir.display(),
        package.pending_tool_path.display()
    );
    Ok(())
}

pub(crate) fn native_agent_loop_resume_pending_package_cli(
    package_dir: PathBuf,
    decision: &str,
) -> Result<(), String> {
    let decision = match decision {
        "allow_once" => PermissionDecisionKind::AllowOnce,
        "deny" => PermissionDecisionKind::Deny,
        other => return Err(format!("unsupported decision {other}: use allow_once|deny")),
    };
    let result =
        resume_scripted_native_agent_loop_external_decision_package(&package_dir, decision)?;
    println!(
        "native pending package resume status={:?} state={:?} events={} models={} tools={} eventlog={} file_hash={}",
        result.loop_result.status,
        result.loop_result.final_state,
        result.loop_result.event_count,
        result.loop_result.model_call_count,
        result.loop_result.tool_call_count,
        result.event_log_path.display(),
        result.final_file_hash
    );
    Ok(())
}

pub(crate) fn native_agent_loop_sidecar_live_cli(
    family: &str,
    output_path: PathBuf,
) -> Result<(), String> {
    let live_requested = env::var("RESEARCHCODE_ENABLE_LIVE_PROVIDER").unwrap_or_default() == "1";
    let network_approved = env::var("RESEARCHCODE_ALLOW_NETWORK").unwrap_or_default() == "1";
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let artifact_root = env::temp_dir().join(format!("researchcode-native-sidecar-loop-{nonce}"));
    let mut endpoint = match family {
        "deepseek" => NativeProviderEndpoint::deepseek_v4_flash_openai(),
        "qwen" => {
            let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
            if let Ok(base_url) = env::var("QWEN_BASE_URL") {
                if !base_url.trim().is_empty() {
                    endpoint.base_url = base_url;
                }
            }
            endpoint
        }
        other => return Err(format!("unknown native family {other}: use deepseek|qwen")),
    };
    let provider_ready = live_requested
        && network_approved
        && env::var(&endpoint.api_key_env)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    endpoint.live_calls_enabled_by_default = provider_ready;
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: format!("sess_native_sidecar_live_{family}"),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: PathBuf::from("."),
        artifact_root: artifact_root.clone(),
        endpoint: endpoint.clone(),
        prompt: "Read the supplied task context and return one concise safe next-step summary. Do not call tools.".to_string(),
        max_tokens: 96,
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
    let result = AgentKernel::for_request(&request).run_turn(
        &PythonSidecarLiveHttpTransport::default_workspace_sidecar(),
        request,
        None,
    )?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&output_path, &result.event_jsonl).map_err(|error| error.to_string())?;
    let status = match result.status {
        NativeAgentLoopStatus::Completed => "completed",
        NativeAgentLoopStatus::Blocked => "blocked",
        NativeAgentLoopStatus::Failed => "failed",
        NativeAgentLoopStatus::Interrupted => "interrupted",
    };
    println!(
        "native sidecar live loop family={} status={} provider_ready={} events={} models={} tools={} eventlog={}",
        family,
        status,
        provider_ready,
        result.event_count,
        result.model_call_count,
        result.tool_call_count,
        output_path.display()
    );
    let _ = fs::remove_dir_all(artifact_root);
    Ok(())
}

pub(crate) fn native_agent_loop_sidecar_live_pending_package_cli(
    family: &str,
    package_dir: PathBuf,
) -> Result<(), String> {
    let live_requested = env::var("RESEARCHCODE_ENABLE_LIVE_PROVIDER").unwrap_or_default() == "1";
    let network_approved = env::var("RESEARCHCODE_ALLOW_NETWORK").unwrap_or_default() == "1";
    let mut endpoint = match family {
        "deepseek" => NativeProviderEndpoint::deepseek_v4_flash_openai(),
        "qwen" => {
            let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
            if let Ok(base_url) = env::var("QWEN_BASE_URL") {
                if !base_url.trim().is_empty() {
                    endpoint.base_url = base_url;
                }
            }
            endpoint
        }
        other => return Err(format!("unknown native family {other}: use deepseek|qwen")),
    };
    let provider_ready = live_requested
        && network_approved
        && env::var(&endpoint.api_key_env)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    endpoint.live_calls_enabled_by_default = provider_ready;
    if !provider_ready {
        return Err(format!(
            "live provider not ready for {family}: set RESEARCHCODE_ENABLE_LIVE_PROVIDER=1, RESEARCHCODE_ALLOW_NETWORK=1, and provide {}{}",
            endpoint.api_key_env,
            if family == "qwen" { " plus QWEN_BASE_URL" } else { "" }
        ));
    }
    let workspace_root = package_dir.join("workspace");
    let artifact_root = package_dir.join("artifacts");
    fs::create_dir_all(&workspace_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&artifact_root).map_err(|error| error.to_string())?;
    let prompt = "Call exactly one tool now: shell_command with command \"pwd\". Do not answer in text. This is a permission-boundary test; the runtime must stop for external approval before executing the command.".to_string();
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: format!("sess_native_sidecar_live_pending_{family}"),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: workspace_root.clone(),
        artifact_root: artifact_root.clone(),
        endpoint: endpoint.clone(),
        prompt,
        max_tokens: 192,
        max_iterations: 1,
        max_tool_calls: 1,
        tool_exposure: NativeAgentToolExposure::FastAutoWrite,
        permission_mode: PermissionMode::Default,
        provided_permission_decisions: Vec::new(),
        deepseek_adaptation: None,
        error_recovery: None,
        hook_dispatcher: None,
        concurrent_tool_execution: false,
    };
    let result = AgentKernel::for_request(&request).run_turn(
        &PythonSidecarLiveHttpTransport::default_workspace_sidecar(),
        request,
        None,
    )?;
    let package = write_native_agent_loop_external_decision_package(
        &package_dir,
        workspace_root,
        artifact_root,
        result,
        "live_provider_blocked_tool_call",
    )?;
    println!(
        "native sidecar live pending package family={} provider_ready={} status={:?} events={} package={} pending={}",
        family,
        provider_ready,
        package.blocked_result.status,
        package.blocked_result.event_count,
        package.package_dir.display(),
        package.pending_tool_path.display()
    );
    Ok(())
}

pub(crate) fn recorded_research_loop_cli(output_path: Option<PathBuf>) -> Result<(), String> {
    let result = run_recorded_research_loop_fixture(&RecordedResearchLoopConfig::default())?;
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(output_path, &result.event_jsonl).map_err(|error| error.to_string())?;
    }
    println!(
        "recorded research loop completed state={:?} events={} manifest_hash={} artifacts={}",
        result.final_state, result.event_count, result.manifest_hash, result.artifact_count
    );
    Ok(())
}

pub(crate) fn native_response_adapter_cli(output_path: Option<PathBuf>) -> Result<(), String> {
    let result =
        run_recorded_non_stream_response_fixture_runtime(&NoModelCodingFixtureConfig::default())?;
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(output_path, &result.event_jsonl).map_err(|error| error.to_string())?;
    }
    println!(
        "native response adapter providers={},{} events={} artifacts={},{}",
        "deepseek",
        "qwen",
        result.event_count,
        result.deepseek_transcript_hash,
        result.qwen_transcript_hash
    );
    Ok(())
}

pub(crate) fn native_response_normalizer_smoke() -> Result<(), String> {
    let deepseek = normalize_deepseek_anthropic_response(
        r#"{"model":"deepseek-v4-flash","content":[{"type":"text","text":"Visible OK"}],"reasoning_content":"Need sk-testsecret from .env","usage":{"input_tokens":100,"output_tokens":20,"reasoning_tokens":15,"cache_read_input_tokens":80,"cache_creation_input_tokens":20}}"#,
    )?;
    let qwen = normalize_qwen_openai_response(
        r#"{"model":"Qwen/Qwen3.6-27B","choices":[{"message":{"reasoning_content":"Need sk-testsecret from .env","content":"Patch ready"}}],"usage":{"prompt_tokens":90,"completion_tokens":18,"reasoning_tokens":7}}"#,
    )?;
    let wrong_qwen = normalize_qwen_openai_response(r#"{"model":"Qwen/Qwen2-7B"}"#);
    if deepseek
        .hidden_reasoning_sanitized
        .as_deref()
        .unwrap_or_default()
        .contains("sk-")
        || qwen
            .hidden_reasoning_sanitized
            .as_deref()
            .unwrap_or_default()
            .contains(".env")
        || wrong_qwen.is_ok()
    {
        return Err("native response normalizer invariant failed".to_string());
    }
    println!(
        "native response normalizer deepseek_tokens={}/{} qwen_tokens={}/{}",
        deepseek.prompt_tokens,
        deepseek.completion_tokens,
        qwen.prompt_tokens,
        qwen.completion_tokens
    );
    Ok(())
}
