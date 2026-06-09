use researchcode_kernel::{PermissionDecisionKind, PlanApprovalDecisionKind};
use researchcode_runtime::live_model_request::{
    build_deepseek_anthropic_request, build_deepseek_openai_request, build_qwen_openai_request,
    ModelRequestMessage,
};
use researchcode_runtime::native_provider::NativeProviderEndpoint;
use researchcode_runtime::runtime_facade::{
    AutonomyMode, RuntimeFacade, RuntimeModelMode, RuntimeSessionSnapshot,
};
use researchcode_runtime::sidecar_http_transport::{
    ProviderSidecarHealthStatus, PythonSidecarLiveHttpTransport,
};
use researchcode_runtime::state::AgentState;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};

const EVENT_CHANNEL: &str = "runtime://event";
// Keep desktop agent turns uncapped by default. Convergence must come from
// model output, user cancellation, permissions, and context handling rather
// than an arbitrary GUI loop cap.
const DEFAULT_MAX_ITERATIONS: usize = 0;
const DEFAULT_MAX_TOOL_CALLS: usize = 0;
const DEFAULT_CONTINUE_PROMPT: &str = "Continue the current session using prior context.";
const APPROVED_PLAN_CONTINUE_PROMPT: &str =
    "The plan was approved. Continue implementing the approved plan using existing evidence. Do not call plan.enter again unless the user asks for a new plan.";
const SMALL_TALK_NO_TOOL_HINT: &str = "The user sent a greeting or simple social opener. Respond naturally and briefly. Do not call any tool for this turn.";
const DEFAULT_RUNTIME_COMMANDS: &[&str] = &[
    "/repo [path]",
    "/read <path> [offset] [limit]",
    "/search <pattern> [path]",
    "/git status|diff|log [args]",
    "/run <command>",
    "/plan [goal]",
    "/plan approve",
    "/plan reject <feedback>",
    "/permissions",
    "/snapshot",
    "/export [path]",
];

struct DesktopRuntimeState {
    facade: RuntimeFacade,
    workspace_root: PathBuf,
    artifact_root: PathBuf,
    argon_workspace_root: PathBuf,
    project_workspace_root: PathBuf,
    last_prompts: Mutex<HashMap<String, String>>,
    active_turns: Mutex<HashMap<String, u64>>,
    next_turn_generation: AtomicU64,
}

impl DesktopRuntimeState {
    fn new(
        workspace_root: PathBuf,
        artifact_root: PathBuf,
        argon_workspace_root: PathBuf,
        project_workspace_root: PathBuf,
    ) -> Self {
        Self {
            facade: RuntimeFacade::new(workspace_root.clone(), artifact_root.clone()),
            workspace_root,
            artifact_root,
            argon_workspace_root,
            project_workspace_root,
            last_prompts: Mutex::new(HashMap::new()),
            active_turns: Mutex::new(HashMap::new()),
            next_turn_generation: AtomicU64::new(1),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeBootstrapDto {
    transport: String,
    workspace_root: String,
    artifact_root: String,
    argon_workspace_root: String,
    project_workspace_root: String,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeStartSessionDto {
    session_id: String,
    task_id: String,
    workspace_root: String,
    model_mode: String,
    autonomy_mode: String,
    state: String,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeSessionSnapshotDto {
    session_id: String,
    state: String,
    event_count: usize,
    model_mode: String,
    autonomy_mode: String,
    workspace_root: String,
    pending_permission_count: usize,
    pending_plan_approval_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeEventDto {
    event_id: Option<String>,
    sequence: Option<usize>,
    event_type: String,
    payload: Option<Value>,
    created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeEventEnvelope {
    session_id: String,
    event: RuntimeEventDto,
    #[serde(skip_serializing_if = "String::is_empty")]
    raw_jsonl: String,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeStreamResultDto {
    session_id: String,
    from_cursor: usize,
    next_cursor: usize,
    has_more: bool,
    events: Vec<RuntimeEventDto>,
    #[serde(skip_serializing_if = "String::is_empty")]
    jsonl: String,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeSubmitResultDto {
    ok: bool,
    session_id: String,
    error_code: Option<String>,
    permission_id: Option<String>,
    tool_call_id: Option<String>,
    provider_tool_call_id: Option<String>,
    tool_id: Option<String>,
    resume_strategy: Option<String>,
    tool_executed: bool,
    model_continuation_required: bool,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeExportResultDto {
    ok: bool,
    session_id: String,
    path: String,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeConfigureProviderResultDto {
    ok: bool,
    env_path: String,
    updated_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeProviderHealthResultDto {
    ok: bool,
    provider: String,
    status: String,
    reason: Option<String>,
    http_status_code: Option<u16>,
    target_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeProjectFolderPickDto {
    path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeSessionRecordWriteResultDto {
    ok: bool,
    path: String,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeActionResultDto {
    ok: bool,
    session_id: String,
    state: Option<String>,
    autonomy_mode: Option<String>,
}

#[tauri::command]
fn runtime_bootstrap(state: State<'_, DesktopRuntimeState>) -> RuntimeBootstrapDto {
    RuntimeBootstrapDto {
        transport: "tauri".to_string(),
        workspace_root: state.workspace_root.to_string_lossy().to_string(),
        artifact_root: state.artifact_root.to_string_lossy().to_string(),
        argon_workspace_root: state.argon_workspace_root.to_string_lossy().to_string(),
        project_workspace_root: state.project_workspace_root.to_string_lossy().to_string(),
    }
}

#[tauri::command]
fn runtime_start_session(
    state: State<'_, DesktopRuntimeState>,
    workspace: Option<String>,
    model_mode: String,
    autonomy_mode: Option<String>,
) -> Result<RuntimeStartSessionDto, String> {
    let source_workspace_path = resolve_workspace_path(&state.workspace_root, workspace)?;
    let workspace_path = ensure_isolated_project_workspace(state.inner(), &source_workspace_path)?;
    let model_mode = parse_model_mode(&model_mode)?;
    let autonomy_mode = parse_autonomy_mode(autonomy_mode.as_deref())?;
    let handle = state
        .facade
        .start_session(Some(workspace_path), model_mode, autonomy_mode)?;
    let snapshot = state.facade.get_session_snapshot(&handle.session_id)?;
    Ok(RuntimeStartSessionDto {
        session_id: handle.session_id,
        task_id: handle.task_id,
        workspace_root: handle.workspace_root.to_string_lossy().to_string(),
        model_mode: handle.model_mode.as_str().to_string(),
        autonomy_mode: handle.autonomy_mode.as_str().to_string(),
        state: agent_state_to_wire(snapshot.state).to_string(),
    })
}

#[tauri::command]
fn runtime_stream_events(
    state: State<'_, DesktopRuntimeState>,
    session_id: String,
    cursor: usize,
) -> Result<RuntimeStreamResultDto, String> {
    let delta = state
        .facade
        .stream_agent_events_since(&session_id, cursor, None)?;
    Ok(RuntimeStreamResultDto {
        session_id: delta.session_id,
        from_cursor: delta.from_cursor,
        next_cursor: delta.next_cursor,
        has_more: delta.has_more,
        events: parse_jsonl_events(&delta.jsonl),
        jsonl: String::new(),
    })
}

#[tauri::command]
fn runtime_get_snapshot(
    state: State<'_, DesktopRuntimeState>,
    session_id: String,
) -> Result<RuntimeSessionSnapshotDto, String> {
    Ok(runtime_snapshot_dto(
        state.facade.get_session_snapshot(&session_id)?,
    ))
}

#[tauri::command]
fn runtime_submit_user_message(
    app: AppHandle,
    state: State<'_, DesktopRuntimeState>,
    session_id: String,
    text: String,
) -> Result<RuntimeSubmitResultDto, String> {
    let turn_generation = {
        let mut turns = state
            .active_turns
            .lock()
            .map_err(|_| "runtime active-turn lock poisoned".to_string())?;
        if turns.contains_key(&session_id) {
            if state
                .facade
                .get_session_snapshot(&session_id)
                .map(|snapshot| is_terminal_agent_state(snapshot.state))
                .unwrap_or(false)
            {
                turns.remove(&session_id);
            } else {
                return Ok(RuntimeSubmitResultDto {
                    ok: false,
                    session_id,
                    error_code: Some("runtime_turn_in_progress".to_string()),
                    permission_id: None,
                    tool_call_id: None,
                    provider_tool_call_id: None,
                    tool_id: None,
                    resume_strategy: None,
                    tool_executed: false,
                    model_continuation_required: false,
                });
            }
        }
        let generation = state.next_turn_generation.fetch_add(1, Ordering::Relaxed);
        turns.insert(session_id.clone(), generation);
        generation
    };
    if let Err(error) = state.facade.submit_user_message(&session_id, &text) {
        release_active_turn(state.inner(), &session_id, turn_generation);
        return Err(error);
    }
    state
        .last_prompts
        .lock()
        .map_err(|_| "runtime prompt lock poisoned".to_string())?
        .insert(session_id.clone(), text.clone());
    let app_handle = app.clone();
    let session_id_for_loop = session_id.clone();
    let text_for_loop = text.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let managed_state = app_handle.state::<DesktopRuntimeState>();
        let loop_result = catch_unwind(AssertUnwindSafe(|| {
            run_native_loop(
                &app_handle,
                managed_state.inner(),
                &session_id_for_loop,
                &text_for_loop,
            )
        }));
        match loop_result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                if !is_runtime_interrupted_error(&error) {
                    record_and_emit_runtime_error_event(
                        &app_handle,
                        managed_state.inner(),
                        &session_id_for_loop,
                        "runtime_turn_failed",
                        &error,
                    );
                }
            }
            Err(payload) => {
                record_and_emit_runtime_error_event(
                    &app_handle,
                    managed_state.inner(),
                    &session_id_for_loop,
                    "runtime_turn_panicked",
                    &panic_payload_to_string(payload),
                );
            }
        }
        release_active_turn(
            managed_state.inner(),
            &session_id_for_loop,
            turn_generation,
        );
    });
    Ok(RuntimeSubmitResultDto {
        ok: true,
        session_id,
        error_code: None,
        permission_id: None,
        tool_call_id: None,
        provider_tool_call_id: None,
        tool_id: None,
        resume_strategy: None,
        tool_executed: false,
        model_continuation_required: false,
    })
}

#[tauri::command]
fn runtime_interrupt_session(
    app: AppHandle,
    state: State<'_, DesktopRuntimeState>,
    session_id: String,
) -> Result<RuntimeActionResultDto, String> {
    state.facade.cancel_session(&session_id)?;
    clear_active_turn(state.inner(), &session_id);
    let _ = emit_runtime_events_since(&app, state.inner(), &session_id, 0);
    let snapshot = state.facade.get_session_snapshot(&session_id).ok();
    Ok(RuntimeActionResultDto {
        ok: true,
        session_id,
        state: snapshot
            .as_ref()
            .map(|value| agent_state_to_wire(value.state).to_string()),
        autonomy_mode: snapshot.map(|value| value.autonomy_mode.as_str().to_string()),
    })
}

#[tauri::command]
fn runtime_set_autonomy_mode(
    state: State<'_, DesktopRuntimeState>,
    session_id: String,
    autonomy_mode: String,
) -> Result<RuntimeActionResultDto, String> {
    let mode = parse_autonomy_mode(Some(&autonomy_mode))?;
    let handle = state.facade.set_autonomy_mode(&session_id, mode)?;
    let snapshot = state.facade.get_session_snapshot(&session_id).ok();
    Ok(RuntimeActionResultDto {
        ok: true,
        session_id,
        state: snapshot
            .as_ref()
            .map(|value| agent_state_to_wire(value.state).to_string()),
        autonomy_mode: Some(handle.autonomy_mode.as_str().to_string()),
    })
}

#[tauri::command]
fn runtime_submit_permission_decision(
    app: AppHandle,
    state: State<'_, DesktopRuntimeState>,
    session_id: String,
    permission_id: String,
    decision: String,
    feedback: Option<String>,
) -> Result<RuntimeSubmitResultDto, String> {
    let decision_kind = parse_permission_decision(&decision)?;
    if let Some(feedback) = feedback
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        emit_runtime_permission_feedback_event(&app, &session_id, &permission_id, &feedback);
    }
    let should_continue = should_continue_after_permission_decision(&decision_kind);
    let event_cursor_before_submission = state
        .facade
        .get_session_snapshot(&session_id)
        .map(|snapshot| snapshot.event_count)
        .unwrap_or(0);
    emit_runtime_permission_submission_event(
        &app,
        &session_id,
        &permission_id,
        "runtime.permission_submission.queued",
        None,
    );
    let app_handle = app.clone();
    let session_id_for_task = session_id.clone();
    let permission_id_for_task = permission_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let managed_state = app_handle.state::<DesktopRuntimeState>();
        emit_runtime_permission_submission_event(
            &app_handle,
            &session_id_for_task,
            &permission_id_for_task,
            "runtime.permission_submission.waiting_for_runtime",
            None,
        );
        let outcome = submit_permission_decision_when_ready(
            managed_state.inner(),
            &session_id_for_task,
            &permission_id_for_task,
            decision_kind,
        );
        match outcome {
            Ok(outcome) => {
                if let Err(error) = emit_runtime_events_since(
                    &app_handle,
                    managed_state.inner(),
                    &session_id_for_task,
                    event_cursor_before_submission,
                ) {
                    emit_runtime_permission_error_event(
                        &app_handle,
                        &session_id_for_task,
                        &permission_id_for_task,
                        "runtime_permission_event_flush_failed",
                        &error,
                    );
                }
                emit_runtime_permission_submission_event(
                    &app_handle,
                    &session_id_for_task,
                    &permission_id_for_task,
                    "runtime.permission_submission.accepted",
                    outcome.tool_id.as_deref(),
                );
                if outcome.model_continuation_required && should_continue {
                    if let Err(error) = spawn_continue_from_last_prompt(
                        &app_handle,
                        &managed_state,
                        &session_id_for_task,
                    ) {
                        emit_runtime_permission_error_event(
                            &app_handle,
                            &session_id_for_task,
                            &permission_id_for_task,
                            "runtime_continue_failed",
                            &error,
                        );
                    }
                }
            }
            Err(error) => {
                emit_runtime_permission_error_event(
                    &app_handle,
                    &session_id_for_task,
                    &permission_id_for_task,
                    "runtime_permission_decision_failed",
                    &error,
                );
            }
        }
    });
    Ok(RuntimeSubmitResultDto {
        ok: true,
        session_id,
        error_code: None,
        permission_id: Some(permission_id),
        tool_call_id: None,
        provider_tool_call_id: None,
        tool_id: None,
        resume_strategy: Some("async_permission_resume".to_string()),
        tool_executed: false,
        model_continuation_required: false,
    })
}

#[tauri::command]
fn runtime_submit_plan_decision(
    app: AppHandle,
    state: State<'_, DesktopRuntimeState>,
    session_id: String,
    plan_approval_id: String,
    decision: String,
    _feedback: Option<String>,
) -> Result<RuntimeSubmitResultDto, String> {
    let decision_kind = parse_plan_decision(&decision)?;
    let continue_after = matches!(decision_kind, PlanApprovalDecisionKind::Approve);
    state
        .facade
        .submit_plan_decision(&session_id, &plan_approval_id, decision_kind)?;
    if continue_after {
        if let Err(error) = spawn_continue_with_prompt(
            &app,
            &state,
            &session_id,
            Some(APPROVED_PLAN_CONTINUE_PROMPT.to_string()),
        ) {
            emit_runtime_error_event(&app, &session_id, "runtime_continue_failed", &error);
            return Ok(RuntimeSubmitResultDto {
                ok: false,
                session_id,
                error_code: Some("runtime_continue_failed".to_string()),
                permission_id: None,
                tool_call_id: None,
                provider_tool_call_id: None,
                tool_id: None,
                resume_strategy: None,
                tool_executed: false,
                model_continuation_required: false,
            });
        }
    }
    Ok(RuntimeSubmitResultDto {
        ok: true,
        session_id,
        error_code: None,
        permission_id: None,
        tool_call_id: None,
        provider_tool_call_id: None,
        tool_id: None,
        resume_strategy: None,
        tool_executed: false,
        model_continuation_required: false,
    })
}

#[tauri::command]
fn runtime_export_events(
    state: State<'_, DesktopRuntimeState>,
    session_id: String,
    path: Option<String>,
) -> Result<RuntimeExportResultDto, String> {
    let output_path = match path {
        Some(value) if !value.trim().is_empty() => {
            let candidate = PathBuf::from(value.trim());
            if candidate.is_absolute() {
                candidate
            } else {
                state.workspace_root.join(candidate)
            }
        }
        _ => state
            .artifact_root
            .join(&session_id)
            .join("events")
            .join("runtime_events.jsonl"),
    };
    state.facade.export_events(&session_id, &output_path)?;
    Ok(RuntimeExportResultDto {
        ok: true,
        session_id,
        path: output_path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
fn runtime_list_commands() -> Vec<String> {
    DEFAULT_RUNTIME_COMMANDS
        .iter()
        .map(|command| (*command).to_string())
        .collect()
}

#[tauri::command]
fn runtime_configure_provider(
    state: State<'_, DesktopRuntimeState>,
    provider: String,
    api_key: Option<String>,
    base_url: Option<String>,
    model_id: Option<String>,
) -> Result<RuntimeConfigureProviderResultDto, String> {
    let provider = provider.trim().to_ascii_lowercase();
    let env_path = state.workspace_root.join(".env");
    let mut updates: Vec<(String, String)> = Vec::new();
    updates.push(("RESEARCHCODE_ALLOW_NETWORK".to_string(), "1".to_string()));
    match provider.as_str() {
        "deepseek" => {
            if let Some(key) = api_key.map(|value| value.trim().to_string()) {
                if !key.is_empty() {
                    updates.push(("DEEPSEEK_API_KEY".to_string(), key));
                }
            }
            if let Some(url) = base_url.map(|value| value.trim().to_string()) {
                if !url.is_empty() {
                    updates.push(("DEEPSEEK_BASE_URL".to_string(), url.clone()));
                    let protocol = if url.contains("/anthropic") {
                        "anthropic_compatible"
                    } else {
                        "openai_compatible"
                    };
                    updates.push((
                        "RESEARCHCODE_DEEPSEEK_PROTOCOL".to_string(),
                        protocol.to_string(),
                    ));
                }
            }
            if let Some(model) = model_id.map(|value| value.trim().to_string()) {
                if !model.is_empty() {
                    updates.push(("DEEPSEEK_MODEL".to_string(), model));
                }
            }
        }
        "qwen" => {
            let configured_key = api_key
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            if let Some(key) = configured_key {
                updates.push(("QWEN_API_KEY".to_string(), key));
            } else if env::var("QWEN_API_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .is_none()
            {
                updates.push(("QWEN_API_KEY".to_string(), "local-qwen-ollama".to_string()));
            }
            if let Some(url) = base_url.map(|value| value.trim().to_string()) {
                if !url.is_empty() {
                    updates.push(("QWEN_BASE_URL".to_string(), url));
                }
            }
        }
        other => return Err(format!("unsupported provider {other}")),
    }
    upsert_env_pairs(&env_path, &updates)?;
    for (key, value) in &updates {
        env::set_var(key, value);
    }
    Ok(RuntimeConfigureProviderResultDto {
        ok: true,
        env_path: env_path.to_string_lossy().to_string(),
        updated_keys: updates.into_iter().map(|(key, _)| key).collect(),
    })
}

#[tauri::command]
fn runtime_health_check_provider(
    provider: String,
) -> Result<RuntimeProviderHealthResultDto, String> {
    let provider = provider.trim().to_ascii_lowercase();
    let endpoint = match provider.as_str() {
        "deepseek" => deepseek_live_endpoint_from_env(),
        "qwen" => qwen_live_endpoint_from_env(),
        other => return Err(format!("unsupported provider {other}")),
    };
    let messages = [
        ModelRequestMessage::new("system", "Health check. Reply with ok."),
        ModelRequestMessage::new("user", "ping"),
    ];
    let request = match endpoint.family {
        researchcode_kernel::model::NativeModelFamily::DeepSeek => {
            if endpoint.protocol == "anthropic_compatible" {
                build_deepseek_anthropic_request(&endpoint, &messages, 16, false)
            } else {
                build_deepseek_openai_request(&endpoint, &messages, 16, false)
            }
        }
        researchcode_kernel::model::NativeModelFamily::Qwen => {
            build_qwen_openai_request(&endpoint, &messages, 16, false)
        }
    }?;
    let transport = PythonSidecarLiveHttpTransport::default_workspace_sidecar();
    let report = transport.health_check(&request)?;
    let status = match report.status {
        ProviderSidecarHealthStatus::Skipped => "skipped",
        ProviderSidecarHealthStatus::Healthy => "healthy",
        ProviderSidecarHealthStatus::Unhealthy => "unhealthy",
    }
    .to_string();
    let ok = matches!(status.as_str(), "healthy" | "skipped");
    Ok(RuntimeProviderHealthResultDto {
        ok,
        provider,
        status,
        reason: report.reason,
        http_status_code: report.http_status_code,
        target_kind: report.target_kind,
    })
}

#[tauri::command]
fn runtime_pick_project_folder(default_path: Option<String>) -> RuntimeProjectFolderPickDto {
    let mut dialog = rfd::FileDialog::new();
    if let Some(path) = default_path
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        dialog = dialog.set_directory(path);
    }
    let selected = dialog.pick_folder();
    RuntimeProjectFolderPickDto {
        path: selected.map(|path| path.to_string_lossy().to_string()),
    }
}

#[tauri::command]
fn runtime_write_session_record(
    state: State<'_, DesktopRuntimeState>,
    workspace_root: String,
    run_id: String,
    session_id: Option<String>,
    content_json: String,
) -> Result<RuntimeSessionRecordWriteResultDto, String> {
    let workspace = resolve_workspace_path(&state.workspace_root, Some(workspace_root))?;
    let safe_run_id = sanitize_file_stem(&run_id);
    let sessions_dir = workspace.join(".argon_agent").join("sessions");
    fs::create_dir_all(&sessions_dir).map_err(|error| error.to_string())?;
    let mut payload = serde_json::from_str::<Value>(&content_json)
        .unwrap_or_else(|_| json!({ "raw": content_json }));
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "session_id".to_string(),
            Value::String(session_id.unwrap_or_default()),
        );
        object.insert(
            "workspace_root".to_string(),
            Value::String(workspace.to_string_lossy().to_string()),
        );
    }
    let output_path = sessions_dir.join(format!("{safe_run_id}.json"));
    let output = serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?;
    fs::write(&output_path, format!("{output}\n")).map_err(|error| error.to_string())?;
    Ok(RuntimeSessionRecordWriteResultDto {
        ok: true,
        path: output_path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
fn runtime_reveal_path(path: String) -> Result<RuntimeActionResultDto, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path is required".to_string());
    }
    let target = PathBuf::from(trimmed);
    let status = if cfg!(target_os = "macos") {
        Command::new("open")
            .arg("-R")
            .arg(&target)
            .status()
            .map_err(|error| format!("open -R failed: {error}"))?
    } else {
        Command::new("xdg-open")
            .arg(target.parent().unwrap_or_else(|| Path::new(".")))
            .status()
            .map_err(|error| format!("xdg-open failed: {error}"))?
    };
    if !status.success() {
        return Err(format!(
            "reveal path failed for {}",
            target.to_string_lossy()
        ));
    }
    Ok(RuntimeActionResultDto {
        ok: true,
        session_id: String::new(),
        state: None,
        autonomy_mode: None,
    })
}

#[tauri::command]
fn desktop_mark_ready(app: AppHandle) -> Result<(), String> {
    if let Some(main_window) = app.get_webview_window("main") {
        let _ = main_window.show();
        let _ = main_window.set_focus();
    }
    if let Some(splash_window) = app.get_webview_window("splash") {
        let _ = splash_window.close();
    }
    Ok(())
}

fn runtime_snapshot_dto(snapshot: RuntimeSessionSnapshot) -> RuntimeSessionSnapshotDto {
    RuntimeSessionSnapshotDto {
        session_id: snapshot.session_id,
        state: agent_state_to_wire(snapshot.state).to_string(),
        event_count: snapshot.event_count,
        model_mode: snapshot.model_mode.as_str().to_string(),
        autonomy_mode: snapshot.autonomy_mode.as_str().to_string(),
        workspace_root: snapshot.workspace_root.to_string_lossy().to_string(),
        pending_permission_count: snapshot.pending_permission_count,
        pending_plan_approval_count: snapshot.pending_plan_approval_count,
    }
}

fn resolve_workspace_path(
    default_workspace: &PathBuf,
    workspace: Option<String>,
) -> Result<PathBuf, String> {
    let raw = workspace.unwrap_or_else(|| ".".to_string());
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "." {
        return Ok(default_workspace.clone());
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(default_workspace.join(path))
    }
}

fn parse_model_mode(value: &str) -> Result<RuntimeModelMode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "deepseek" => Ok(RuntimeModelMode::DeepSeek),
        "qwen" => Ok(RuntimeModelMode::Qwen),
        other => Err(format!("unsupported model_mode {other}")),
    }
}

fn parse_autonomy_mode(value: Option<&str>) -> Result<AutonomyMode, String> {
    let mode = value.unwrap_or("fast_auto").trim().to_ascii_lowercase();
    match mode.as_str() {
        "conservative" => Ok(AutonomyMode::Conservative),
        "fast_auto" => Ok(AutonomyMode::FastAuto),
        "manual_review" => Ok(AutonomyMode::ManualReview),
        other => Err(format!("unsupported autonomy_mode {other}")),
    }
}

fn parse_permission_decision(value: &str) -> Result<PermissionDecisionKind, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "allow_once" => Ok(PermissionDecisionKind::AllowOnce),
        "allow_session" => Ok(PermissionDecisionKind::AllowSession),
        "allow_project" | "allow_project_rule" => Ok(PermissionDecisionKind::AllowProjectRule),
        "deny" => Ok(PermissionDecisionKind::Deny),
        "deny_with_suggestion" => Ok(PermissionDecisionKind::Modify),
        "modify" => Ok(PermissionDecisionKind::Modify),
        other => Err(format!("unsupported permission decision {other}")),
    }
}

fn should_continue_after_permission_decision(decision: &PermissionDecisionKind) -> bool {
    matches!(
        decision,
        PermissionDecisionKind::AllowOnce
            | PermissionDecisionKind::AllowSession
            | PermissionDecisionKind::AllowProjectRule
    )
}

fn submit_permission_decision_when_ready(
    state: &DesktopRuntimeState,
    session_id: &str,
    permission_id: &str,
    decision: PermissionDecisionKind,
) -> Result<researchcode_runtime::runtime_facade::RuntimePermissionDecisionOutcome, String> {
    let mut last_error = None;
    let mut inactive_settle_attempts = 0usize;
    for attempt in 0..=60 {
        match state.facade.submit_permission_decision_with_outcome(
            session_id,
            permission_id,
            decision.clone(),
        ) {
            Ok(outcome) => return Ok(outcome),
            Err(error) => {
                last_error = Some(error);
                let active = clear_active_turn_if_resumable_boundary(state, session_id)?
                    || state
                        .active_turns
                        .lock()
                        .map_err(|_| "runtime active-turn lock poisoned".to_string())?
                        .contains_key(session_id);
                if !active {
                    inactive_settle_attempts += 1;
                }
                if attempt == 60 || (!active && inactive_settle_attempts >= 6) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "permission decision was not submitted".to_string()))
}

fn clear_active_turn_if_resumable_boundary(
    state: &DesktopRuntimeState,
    session_id: &str,
) -> Result<bool, String> {
    let observed_generation = state
        .active_turns
        .lock()
        .map_err(|_| "runtime active-turn lock poisoned".to_string())?
        .get(session_id)
        .copied();
    let Some(observed_generation) = observed_generation else {
        return Ok(false);
    };
    let resumable = state
        .facade
        .get_session_snapshot(session_id)
        .map(|snapshot| is_resumable_approval_state(snapshot.state))
        .unwrap_or(false);
    if !resumable {
        return Ok(true);
    }
    let mut turns = state
        .active_turns
        .lock()
        .map_err(|_| "runtime active-turn lock poisoned".to_string())?;
    Ok(!clear_matching_active_turn_generation(
        &mut turns,
        session_id,
        observed_generation,
    ))
}

fn is_resumable_approval_state(state: AgentState) -> bool {
    matches!(
        state,
        AgentState::WaitingForToolApproval | AgentState::WaitingForPlanApproval
    )
}

fn clear_matching_active_turn_generation(
    active_turns: &mut HashMap<String, u64>,
    session_id: &str,
    observed_generation: u64,
) -> bool {
    if active_turns.get(session_id).copied() == Some(observed_generation) {
        active_turns.remove(session_id);
        true
    } else {
        false
    }
}

fn parse_plan_decision(value: &str) -> Result<PlanApprovalDecisionKind, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "approve" => Ok(PlanApprovalDecisionKind::Approve),
        "reject" => Ok(PlanApprovalDecisionKind::Reject),
        "request_revision" => Ok(PlanApprovalDecisionKind::RequestRevision),
        other => Err(format!("unsupported plan decision {other}")),
    }
}

fn continue_from_last_prompt(
    app: &AppHandle,
    state: &DesktopRuntimeState,
    session_id: &str,
) -> Result<(), String> {
    let prompt = state
        .last_prompts
        .lock()
        .map_err(|_| "runtime prompt lock poisoned".to_string())?
        .get(session_id)
        .cloned()
        .unwrap_or_else(|| DEFAULT_CONTINUE_PROMPT.to_string());
    run_native_loop(app, state, session_id, &prompt)
}

fn spawn_continue_from_last_prompt(
    app: &AppHandle,
    state: &DesktopRuntimeState,
    session_id: &str,
) -> Result<(), String> {
    spawn_continue_with_prompt(app, state, session_id, None)
}

fn spawn_continue_with_prompt(
    app: &AppHandle,
    state: &DesktopRuntimeState,
    session_id: &str,
    prompt_override: Option<String>,
) -> Result<(), String> {
    let _ = state;
    let app_handle = app.clone();
    let session_id_for_loop = session_id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let managed_state = app_handle.state::<DesktopRuntimeState>();
        let mut acquired_turn_generation = None;
        for attempt in 0..=100 {
            match managed_state.active_turns.lock() {
                Ok(mut turns) => {
                    if !turns.contains_key(&session_id_for_loop) {
                        let generation = managed_state
                            .next_turn_generation
                            .fetch_add(1, Ordering::Relaxed);
                        turns.insert(session_id_for_loop.clone(), generation);
                        acquired_turn_generation = Some(generation);
                        break;
                    }
                }
                Err(_) => {
                    emit_runtime_error_event(
                        &app_handle,
                        &session_id_for_loop,
                        "runtime_continue_failed",
                        "runtime active-turn lock poisoned",
                    );
                    return;
                }
            }
            match clear_active_turn_if_resumable_boundary(
                managed_state.inner(),
                &session_id_for_loop,
            ) {
                Ok(false) => continue,
                Ok(true) => {}
                Err(error) => {
                    emit_runtime_error_event(
                        &app_handle,
                        &session_id_for_loop,
                        "runtime_continue_failed",
                        &error,
                    );
                    return;
                }
            }
            if attempt == 100 {
                emit_runtime_error_event(
                    &app_handle,
                    &session_id_for_loop,
                    "runtime_continue_failed",
                    "runtime_turn_still_active_after_permission_resume",
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let loop_result = catch_unwind(AssertUnwindSafe(|| {
            if let Some(prompt) = prompt_override {
                run_native_loop(
                    &app_handle,
                    managed_state.inner(),
                    &session_id_for_loop,
                    &prompt,
                )
            } else {
                continue_from_last_prompt(&app_handle, managed_state.inner(), &session_id_for_loop)
            }
        }));
        match loop_result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                record_and_emit_runtime_error_event(
                    &app_handle,
                    managed_state.inner(),
                    &session_id_for_loop,
                    "runtime_turn_failed",
                    &error,
                );
            }
            Err(payload) => {
                record_and_emit_runtime_error_event(
                    &app_handle,
                    managed_state.inner(),
                    &session_id_for_loop,
                    "runtime_turn_panicked",
                    &panic_payload_to_string(payload),
                );
            }
        }
        if let Some(generation) = acquired_turn_generation {
            release_active_turn(managed_state.inner(), &session_id_for_loop, generation);
        }
    });
    Ok(())
}

fn run_native_loop(
    app: &AppHandle,
    state: &DesktopRuntimeState,
    session_id: &str,
    prompt: &str,
) -> Result<(), String> {
    let snapshot = state.facade.get_session_snapshot(session_id)?;
    let mut sink = |event_line: &str| emit_runtime_event_line(app, session_id, event_line);
    let transport = PythonSidecarLiveHttpTransport::default_workspace_sidecar();
    let small_talk_turn = is_small_talk_prompt(prompt);
    let effective_prompt = if small_talk_turn {
        format!("{SMALL_TALK_NO_TOOL_HINT}\n\nUser: {prompt}")
    } else {
        prompt.to_string()
    };
    let max_tool_calls = if small_talk_turn {
        1
    } else {
        DEFAULT_MAX_TOOL_CALLS
    };
    match snapshot.model_mode {
        RuntimeModelMode::DeepSeek => {
            let primary = deepseek_live_endpoint_from_env();
            match state
                .facade
                .run_deepseek_agent_loop_with_transport_and_event_sink(
                    &transport,
                    session_id,
                    &effective_prompt,
                    primary.clone(),
                    DEFAULT_MAX_ITERATIONS,
                    max_tool_calls,
                    &mut sink,
                ) {
                Ok(_) => Ok(()),
                Err(error)
                    if primary.protocol == "anthropic_compatible"
                        && error.contains("http failure")
                        && error.contains("400") =>
                {
                    let fallback = deepseek_openai_fallback_endpoint_from(&primary);
                    match state
                        .facade
                        .run_deepseek_agent_loop_with_transport_and_event_sink(
                            &transport,
                            session_id,
                            &effective_prompt,
                            fallback,
                            DEFAULT_MAX_ITERATIONS,
                            max_tool_calls,
                            &mut sink,
                        ) {
                        Ok(_) => Ok(()),
                        Err(inner) => {
                            handle_live_loop_error(app, state, session_id, "deepseek", inner)
                        }
                    }
                }
                Err(error) => handle_live_loop_error(app, state, session_id, "deepseek", error),
            }
        }
        RuntimeModelMode::Qwen => {
            match state
                .facade
                .run_qwen_agent_loop_with_transport_and_event_sink(
                    &transport,
                    session_id,
                    &effective_prompt,
                    qwen_live_endpoint_from_env(),
                    DEFAULT_MAX_ITERATIONS,
                    max_tool_calls,
                    &mut sink,
                ) {
                Ok(_) => Ok(()),
                Err(error) => handle_live_loop_error(app, state, session_id, "qwen", error),
            }
        }
    }
}

fn handle_live_loop_error(
    app: &AppHandle,
    state: &DesktopRuntimeState,
    session_id: &str,
    provider: &str,
    error: String,
) -> Result<(), String> {
    if is_recoverable_live_block_error(&error) {
        state.facade.record_live_model_blocked(
            session_id,
            provider,
            &sanitize_gate_for_panel(&error),
        )?;
        emit_runtime_error_event(app, session_id, "live_model_blocked", &error);
        return Ok(());
    }
    Err(error)
}

fn is_recoverable_live_block_error(error: &str) -> bool {
    error.contains("sidecar_skipped")
        || error.contains("network_not_enabled")
        || error.contains("live provider")
        || error.contains("missing_api_key")
        || error.contains("MissingApiKey")
}

fn sanitize_gate_for_panel(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | ':') {
                character
            } else {
                '_'
            }
        })
        .take(96)
        .collect()
}

fn is_small_talk_prompt(prompt: &str) -> bool {
    let compact = prompt.trim().to_lowercase();
    if compact.is_empty() || compact.chars().count() > 24 {
        return false;
    }
    matches!(
        compact.as_str(),
        "hi" | "hello"
            | "hey"
            | "yo"
            | "你好"
            | "你好啊"
            | "嗨"
            | "在吗"
            | "在么"
            | "早上好"
            | "晚上好"
            | "下午好"
    ) || compact.starts_with("你好")
}

fn deepseek_openai_fallback_endpoint_from(
    primary: &NativeProviderEndpoint,
) -> NativeProviderEndpoint {
    let mut fallback = NativeProviderEndpoint::deepseek_v4_flash_openai();
    fallback.live_calls_enabled_by_default = true;
    fallback.actual_model_name = primary.actual_model_name.clone();
    fallback.display_model_name = primary.display_model_name.clone();
    if let Ok(base_url) = env::var("DEEPSEEK_OPENAI_BASE_URL") {
        let base_url = base_url.trim();
        if !base_url.is_empty() {
            fallback.base_url = base_url.to_string();
            return fallback;
        }
    }
    if primary.base_url.contains("/anthropic") {
        fallback.base_url = primary.base_url.replace("/anthropic", "");
    }
    fallback
}

fn deepseek_live_endpoint_from_env() -> NativeProviderEndpoint {
    let protocol = env::var("RESEARCHCODE_DEEPSEEK_PROTOCOL")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let env_base_url = env::var("DEEPSEEK_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let mut endpoint = if matches!(protocol.as_str(), "openai" | "openai_compatible") {
        NativeProviderEndpoint::deepseek_v4_flash_openai()
    } else if matches!(protocol.as_str(), "anthropic" | "anthropic_compatible") {
        NativeProviderEndpoint::deepseek_v4_flash_anthropic()
    } else if env_base_url
        .as_deref()
        .is_some_and(|value| value.contains("/anthropic"))
    {
        NativeProviderEndpoint::deepseek_v4_flash_anthropic()
    } else {
        NativeProviderEndpoint::deepseek_v4_flash_anthropic()
    };
    endpoint.live_calls_enabled_by_default = true;
    if let Some(base_url) = env_base_url {
        endpoint.base_url = normalize_deepseek_base_url_for_protocol(&endpoint.protocol, &base_url);
    }
    if let Ok(model_name) = env::var("DEEPSEEK_MODEL") {
        let model_name = model_name.trim();
        if !model_name.is_empty() {
            endpoint.actual_model_name = model_name.to_string();
        }
    }
    endpoint
}

fn qwen_live_endpoint_from_env() -> NativeProviderEndpoint {
    let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
    endpoint.live_calls_enabled_by_default = true;
    if let Ok(base_url) = env::var("QWEN_BASE_URL") {
        let base_url = base_url.trim();
        if !base_url.is_empty() {
            endpoint.base_url = base_url.to_string();
        }
    }
    endpoint
}

fn normalize_deepseek_base_url_for_protocol(protocol: &str, base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if protocol == "anthropic_compatible" {
        if trimmed.ends_with("/anthropic") {
            return trimmed.to_string();
        }
        if let Some(root) = trimmed.strip_suffix("/v1") {
            return format!("{root}/anthropic");
        }
        return format!("{trimmed}/anthropic");
    }
    trimmed.to_string()
}

fn agent_state_to_wire(state: AgentState) -> &'static str {
    match state {
        AgentState::Created => "Created",
        AgentState::Planning => "Planning",
        AgentState::WaitingForPlanApproval => "WaitingForPlanApproval",
        AgentState::RetrievingContext => "RetrievingContext",
        AgentState::Executing => "Executing",
        AgentState::WaitingForToolApproval => "WaitingForToolApproval",
        AgentState::ApplyingPatch => "ApplyingPatch",
        AgentState::RunningCommand => "RunningCommand",
        AgentState::DiagnosingFailure => "DiagnosingFailure",
        AgentState::Reviewing => "Reviewing",
        AgentState::WaitingForUser => "WaitingForUser",
        AgentState::Completed => "Completed",
        AgentState::Failed => "Failed",
        AgentState::Cancelled => "Cancelled",
    }
}

fn is_terminal_agent_state(state: AgentState) -> bool {
    matches!(
        state,
        AgentState::Completed | AgentState::Failed | AgentState::Cancelled
    )
}

fn is_runtime_interrupted_error(error: &str) -> bool {
    error.contains("sidecar_interrupted") || error.contains("interrupted")
}

fn parse_jsonl_events(jsonl: &str) -> Vec<RuntimeEventDto> {
    jsonl.lines().map(runtime_event_from_line).collect()
}

fn runtime_event_from_line(line: &str) -> RuntimeEventDto {
    let parsed: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => {
            return RuntimeEventDto {
                event_id: None,
                sequence: None,
                event_type: "runtime.error".to_string(),
                payload: Some(json!({
                    "error_code": "event_parse_failed",
                    "message": trim_for_event(&error.to_string(), 220),
                })),
                created_at: None,
            }
        }
    };
    let payload = parsed.get("payload").and_then(|value| {
        value
            .as_object()
            .map(|object| Value::Object(object.clone()))
    });
    RuntimeEventDto {
        event_id: parsed
            .get("event_id")
            .and_then(|value| value.as_str().map(|s| s.to_string())),
        sequence: parsed
            .get("sequence")
            .and_then(|value| value.as_u64())
            .map(|value| value as usize),
        event_type: parsed
            .get("event_type")
            .and_then(|value| value.as_str())
            .unwrap_or("runtime.unknown")
            .to_string(),
        payload,
        created_at: parsed
            .get("created_at")
            .and_then(|value| value.as_str().map(|s| s.to_string())),
    }
}

fn emit_runtime_event_line(app: &AppHandle, session_id: &str, line: &str) {
    let _ = app.emit(
        EVENT_CHANNEL,
        RuntimeEventEnvelope {
            session_id: session_id.to_string(),
            event: runtime_event_from_line(line),
            raw_jsonl: String::new(),
        },
    );
}

fn emit_runtime_events_since(
    app: &AppHandle,
    state: &DesktopRuntimeState,
    session_id: &str,
    cursor: usize,
) -> Result<(), String> {
    let delta = state
        .facade
        .stream_agent_events_since(session_id, cursor, None)?;
    for line in delta.jsonl.lines() {
        emit_runtime_event_line(app, session_id, line);
    }
    Ok(())
}

fn emit_runtime_error_event(app: &AppHandle, session_id: &str, error_code: &str, message: &str) {
    let event = RuntimeEventDto {
        event_id: None,
        sequence: None,
        event_type: "runtime.error".to_string(),
        payload: Some(json!({
            "error_code": error_code,
            "message": trim_for_event(message, 480),
            "recoverable": true,
        })),
        created_at: None,
    };
    let _ = app.emit(
        EVENT_CHANNEL,
        RuntimeEventEnvelope {
            session_id: session_id.to_string(),
            event,
            raw_jsonl: String::new(),
        },
    );
}

fn record_and_emit_runtime_error_event(
    app: &AppHandle,
    state: &DesktopRuntimeState,
    session_id: &str,
    error_code: &str,
    message: &str,
) {
    let _ = state
        .facade
        .record_runtime_error(session_id, error_code, message);
    emit_runtime_error_event(app, session_id, error_code, message);
}

fn release_active_turn(state: &DesktopRuntimeState, session_id: &str, generation: u64) {
    if let Ok(mut turns) = state.active_turns.lock() {
        if turns.get(session_id).copied() == Some(generation) {
            turns.remove(session_id);
        }
    }
}

fn clear_active_turn(state: &DesktopRuntimeState, session_id: &str) {
    if let Ok(mut turns) = state.active_turns.lock() {
        turns.remove(session_id);
    }
}

fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "unknown panic payload".to_string()
}

fn emit_runtime_permission_submission_event(
    app: &AppHandle,
    session_id: &str,
    permission_id: &str,
    event_type: &str,
    tool_id: Option<&str>,
) {
    let event = RuntimeEventDto {
        event_id: None,
        sequence: None,
        event_type: event_type.to_string(),
        payload: Some(json!({
            "permission_id": permission_id,
            "tool_id": tool_id,
        })),
        created_at: None,
    };
    let _ = app.emit(
        EVENT_CHANNEL,
        RuntimeEventEnvelope {
            session_id: session_id.to_string(),
            event,
            raw_jsonl: String::new(),
        },
    );
}

fn emit_runtime_permission_error_event(
    app: &AppHandle,
    session_id: &str,
    permission_id: &str,
    error_code: &str,
    message: &str,
) {
    // Generate a unique event_id so GUI dedupe logic does not collapse
    // distinct synthetic error events into one entry.
    let synthetic_event_id = format!(
        "synthetic_error_{}_{}_{}",
        permission_id,
        error_code,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let event = RuntimeEventDto {
        event_id: Some(synthetic_event_id),
        sequence: None,
        event_type: "runtime.error".to_string(),
        payload: Some(json!({
            "error_code": error_code,
            "message": trim_for_event(message, 480),
            "permission_id": permission_id,
            "recoverable": true,
        })),
        created_at: None,
    };
    let _ = app.emit(
        EVENT_CHANNEL,
        RuntimeEventEnvelope {
            session_id: session_id.to_string(),
            event,
            raw_jsonl: String::new(),
        },
    );
}

fn emit_runtime_permission_feedback_event(
    app: &AppHandle,
    session_id: &str,
    permission_id: &str,
    feedback: &str,
) {
    let event = RuntimeEventDto {
        event_id: None,
        sequence: None,
        event_type: "permission.suggestion_submitted".to_string(),
        payload: Some(json!({
            "permission_id": permission_id,
            "feedback": trim_for_event(feedback, 280),
        })),
        created_at: None,
    };
    let _ = app.emit(
        EVENT_CHANNEL,
        RuntimeEventEnvelope {
            session_id: session_id.to_string(),
            event,
            raw_jsonl: String::new(),
        },
    );
}

fn trim_for_event(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn parse_dotenv_pairs(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            let (key, value) = trimmed.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            let value = value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            Some((key.to_string(), value))
        })
        .collect()
}

fn load_workspace_env(workspace_root: &Path) -> Result<(), String> {
    let env_path = workspace_root.join(".env");
    if !env_path.exists() {
        return Ok(());
    }
    let text = fs::read_to_string(&env_path).map_err(|error| error.to_string())?;
    for (key, value) in parse_dotenv_pairs(&text) {
        env::set_var(key, value);
    }
    Ok(())
}

fn upsert_env_pairs(env_path: &Path, updates: &[(String, String)]) -> Result<(), String> {
    let mut lines: Vec<String> = if env_path.exists() {
        fs::read_to_string(env_path)
            .map_err(|error| error.to_string())?
            .lines()
            .map(|line| line.to_string())
            .collect()
    } else {
        Vec::new()
    };
    for (key, value) in updates {
        let prefix = format!("{key}=");
        let replacement = format!("{key}={value}");
        if let Some(index) = lines
            .iter()
            .position(|line| line.trim_start().starts_with(&prefix))
        {
            lines[index] = replacement;
        } else {
            lines.push(replacement);
        }
    }
    if let Some(parent) = env_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let output = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    fs::write(env_path, output).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(env_path, permissions).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn resolve_workspace_root() -> PathBuf {
    let manifest_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest_root.join("../..");
    candidate.canonicalize().unwrap_or(candidate)
}

fn resolve_argon_workspace_root(default_workspace: &Path) -> PathBuf {
    if let Ok(value) = env::var("ARGON_WORKSPACE_ROOT") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Ok(home) = env::var("HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed)
                .join(".researchcode")
                .join("argon_agent")
                .join("workspaces");
        }
    }
    default_workspace
        .join(".researchcode")
        .join("argon_agent")
        .join("workspaces")
}

fn canonical_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn stable_short_hash(input: &str) -> String {
    let mut hash: u64 = 1469598103934665603;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{:016x}", hash)[..8].to_string()
}

fn sanitize_workspace_name(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let compact = out.trim_matches('-').to_string();
    if compact.is_empty() {
        "workspace".to_string()
    } else {
        compact
    }
}

fn sanitize_file_stem(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let compact = out.trim_matches('_').to_string();
    if compact.is_empty() {
        "session".to_string()
    } else {
        compact
    }
}

fn git_repo_top_level(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .current_dir(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn ensure_git_detached_worktree(
    source_repo_root: &Path,
    target_workspace: &Path,
) -> Result<(), String> {
    if target_workspace.join(".git").exists() {
        return Ok(());
    }
    if let Some(parent) = target_workspace.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if !target_workspace.exists() {
        fs::create_dir_all(target_workspace).map_err(|error| error.to_string())?;
    }
    let status = Command::new("git")
        .arg("worktree")
        .arg("add")
        .arg("--detach")
        .arg(target_workspace)
        .arg("HEAD")
        .current_dir(source_repo_root)
        .status()
        .map_err(|error| format!("git worktree add failed to launch: {error}"))?;
    if !status.success() {
        return Err(format!(
            "git worktree add failed for {}",
            target_workspace.to_string_lossy()
        ));
    }
    Ok(())
}

fn write_project_workspace_metadata(
    metadata_path: &Path,
    source_root: &Path,
    workspace_root: &Path,
    mode: &str,
) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let payload = json!({
        "source_root": source_root.to_string_lossy(),
        "workspace_root": workspace_root.to_string_lossy(),
        "mode": mode,
        "updated_at_unix": now,
    });
    if let Some(parent) = metadata_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(metadata_path, payload.to_string()).map_err(|error| error.to_string())
}

fn ensure_isolated_project_workspace(
    state: &DesktopRuntimeState,
    source_workspace: &Path,
) -> Result<PathBuf, String> {
    let source_root = canonical_or_self(source_workspace);
    let project_workspace_root = canonical_or_self(&state.project_workspace_root);
    if source_root.starts_with(&project_workspace_root) {
        return Ok(source_root);
    }
    let basename = source_root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "workspace".to_string());
    let safe_basename = sanitize_workspace_name(&basename);
    let source_key = source_root.to_string_lossy().to_string();
    let project_key = format!("{}-{}", safe_basename, stable_short_hash(&source_key));
    let project_home = state.project_workspace_root.join(project_key);
    let isolated_workspace = project_home.join(&safe_basename);
    fs::create_dir_all(&project_home).map_err(|error| error.to_string())?;

    let (effective_workspace, mode) = if let Some(repo_root) = git_repo_top_level(&source_root) {
        ensure_git_detached_worktree(&repo_root, &isolated_workspace)?;
        (canonical_or_self(&isolated_workspace), "git_worktree")
    } else {
        fs::create_dir_all(&isolated_workspace).map_err(|error| error.to_string())?;
        (source_root.clone(), "source_root_fallback")
    };

    write_project_workspace_metadata(
        &project_home.join("workspace_meta.json"),
        &source_root,
        &effective_workspace,
        mode,
    )?;
    Ok(effective_workspace)
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let workspace_root = resolve_workspace_root();
            let artifact_root = workspace_root.join(".researchcode").join("runtime_desktop");
            let argon_workspace_root = resolve_argon_workspace_root(&workspace_root);
            let project_workspace_root = argon_workspace_root.join("projects");
            fs::create_dir_all(&artifact_root)?;
            fs::create_dir_all(&project_workspace_root)?;
            load_workspace_env(&workspace_root)?;
            app.manage(DesktopRuntimeState::new(
                workspace_root,
                artifact_root,
                argon_workspace_root,
                project_workspace_root,
            ));
            if let Some(main_window) = app.get_webview_window("main") {
                let _ = main_window.hide();
            }
            if let Some(splash_window) = app.get_webview_window("splash") {
                let _ = splash_window.show();
                let _ = splash_window.set_focus();
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                let state = window.app_handle().state::<DesktopRuntimeState>();
                state.facade.interrupt();
            }
        })
        .invoke_handler(tauri::generate_handler![
            desktop_mark_ready,
            runtime_bootstrap,
            runtime_start_session,
            runtime_stream_events,
            runtime_get_snapshot,
            runtime_submit_user_message,
            runtime_interrupt_session,
            runtime_set_autonomy_mode,
            runtime_submit_permission_decision,
            runtime_submit_plan_decision,
            runtime_export_events,
            runtime_list_commands,
            runtime_configure_provider,
            runtime_health_check_provider,
            runtime_pick_project_folder,
            runtime_write_session_record,
            runtime_reveal_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_agent_loop_keeps_tool_budget_uncapped() {
        assert_eq!(DEFAULT_MAX_TOOL_CALLS, 0);
        assert_eq!(DEFAULT_MAX_ITERATIONS, 0);
    }
}
