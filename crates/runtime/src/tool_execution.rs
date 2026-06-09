//! Runtime ToolExecutionService v0.
//!
//! This is the shared execution boundary for tool previews and permission-aware
//! AgentSession tool execution. The service does not emit session events; callers
//! remain responsible for recording plan, permission, approval, and result
//! lifecycle events around this boundary.

use crate::agent_kernel::permission_gate::{
    DefaultTool, FileEditTool, FileWriteTool, PatchApplyTool, ShellCommandTool,
};
use crate::agent_kernel::PermissionGate;
use crate::command::{prepare_command, run_prepared_command, CommandRequest, CommandRunError};
use crate::file_tool::{is_sensitive_path, read_file, FileReadRequest};
use crate::git_tool::{git_status, GitStatusRequest};
use crate::patch::{
    apply_replace_patch_allowing_protected, stable_text_hash, PatchCheck, PatchValidation,
    ReplacePatch,
};
use crate::permission_policy::PermissionResolution;
use crate::repo_map::{build_repo_map, RepoMapRequest};
use crate::research_worker::{
    run_csv_profile_sidecar, ResearchCsvProfileRequest, ResearchWorkerLimits,
};
use crate::search_tool::{search_text_with_outcome, SearchError, SearchMatch, SearchRequest};
use crate::secret_scan::redact_text_for_secrets;
use crate::tool_result_format::{
    format_file_edit_preview, format_file_multi_edit_preview, format_file_read_preview,
    format_file_write_preview, format_list_directory_preview, format_list_tree_preview,
    format_shell_command_preview,
};
use researchcode_kernel::tool::{find_tool_spec, ToolRisk};
use researchcode_kernel::{PermissionDecisionKind, PermissionRequestType};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolExecutionMode {
    ReadOnlyPreview,
    ApplyWithPermission {
        permission_decision: Option<PermissionDecisionKind>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolExecutionArgs {
    pub path: Option<String>,
    pub root: Option<String>,
    pub include_hidden: Option<bool>,
    pub pattern: Option<String>,
    pub query: Option<String>,
    pub content: Option<String>,
    pub max_bytes: Option<usize>,
    pub max_results: Option<usize>,
    pub max_files: Option<usize>,
    pub max_depth: Option<usize>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    pub command: Option<String>,
    pub old_string: Option<String>,
    pub new_string: Option<String>,
    pub base_hash: Option<String>,
    pub replace_all: Option<bool>,
    pub edits_json: Option<String>,
    pub input_csv: Option<String>,
    pub job_id: Option<String>,
    pub output_dir: Option<String>,
    pub answer: Option<String>,
    pub model_role: Option<String>,
    pub write_scope_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecutionRequest {
    pub workspace_root: PathBuf,
    pub tool_call_id: String,
    pub tool_id: String,
    pub mode: ToolExecutionMode,
    pub args: ToolExecutionArgs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecutionResult {
    pub tool_call_id: String,
    pub tool_id: String,
    pub ok: bool,
    pub preview: String,
    pub detail_json: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub tool_call_id: String,
    pub tool_id: String,
    pub args: ToolExecutionArgs,
}

/// A batch of tools that can run concurrently.
#[derive(Debug, Clone)]
pub struct ToolBatch {
    pub batch_id: String,
    pub tools: Vec<ToolCall>,
    /// If true, sibling errors abort the rest of the batch.
    pub abort_on_sibling_error: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SiblingAbortController {
    aborted: Arc<Mutex<bool>>,
}

impl SiblingAbortController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn abort(&self) {
        *self.aborted.lock().unwrap() = true;
    }

    pub fn is_aborted(&self) -> bool {
        *self.aborted.lock().unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolExecutionError {
    UnknownTool(String),
    PermissionRequired(String),
    NonReadOnlyTool(String),
    SensitivePath(String),
    PathEscapesWorkspace(String),
    MissingArgument(String),
    ValidationFailed(String),
    ToolFailed(String),
}

pub fn execute_tool(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    execute_tool_inner(request, None)
}

pub fn execute_tool_with_permission_gate(
    request: &ToolExecutionRequest,
    permission_gate: &mut PermissionGate,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    execute_tool_inner(request, Some(permission_gate))
}

fn execute_tool_inner(
    request: &ToolExecutionRequest,
    permission_gate: Option<&mut PermissionGate>,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let spec = find_tool_spec(&request.tool_id)
        .ok_or_else(|| ToolExecutionError::UnknownTool(request.tool_id.clone()))?;
    match &request.mode {
        ToolExecutionMode::ReadOnlyPreview => execute_tool_preview(request),
        ToolExecutionMode::ApplyWithPermission {
            permission_decision,
        } => {
            if spec.risk == ToolRisk::ReadOnly && !spec.permission_required {
                let mut preview_request = request.clone();
                preview_request.mode = ToolExecutionMode::ReadOnlyPreview;
                return execute_tool_preview(&preview_request);
            }
            let permission_decision = match (permission_decision.clone(), permission_gate) {
                (Some(decision), _) => Some(decision),
                (None, Some(gate)) => Some(resolve_permission_with_gate(request, gate)?),
                (None, None) => None,
            };
            match request.tool_id.as_str() {
                "shell.command" => execute_shell_command(request, permission_decision),
                "patch.apply" => execute_patch_apply(request, permission_decision),
                "file.edit" => execute_file_edit(request, permission_decision),
                "file.write" => execute_file_write(request, permission_decision),
                "file.multi_edit" => execute_file_multi_edit(request, permission_decision),
                "todo.write" => execute_todo_write_preview(request),
                "ask_user" => execute_ask_user_preview(request),
                other if spec.permission_required => {
                    Err(ToolExecutionError::PermissionRequired(other.to_string()))
                }
                other => Err(ToolExecutionError::NonReadOnlyTool(other.to_string())),
            }
        }
    }
}

fn resolve_permission_with_gate(
    request: &ToolExecutionRequest,
    permission_gate: &mut PermissionGate,
) -> Result<PermissionDecisionKind, ToolExecutionError> {
    let request_type = permission_request_type_for_execution_tool(&request.tool_id)
        .ok_or_else(|| ToolExecutionError::PermissionRequired(request.tool_id.clone()))?;
    let args_json = permission_args_json(&request.args);
    let command_summary = permission_summary(&request.tool_id, &request.args);
    let tool = permission_tool_for_execution(&request.tool_id);
    match permission_gate.evaluate_current(
        &request.tool_id,
        &args_json,
        request_type,
        command_summary.as_deref(),
        tool.as_ref(),
    ) {
        PermissionResolution::Allow => Ok(PermissionDecisionKind::AllowOnce),
        PermissionResolution::Ask { .. } => Err(ToolExecutionError::PermissionRequired(
            request.tool_id.clone(),
        )),
        PermissionResolution::Deny { .. } => Err(ToolExecutionError::PermissionRequired(
            request.tool_id.clone(),
        )),
    }
}

fn permission_request_type_for_execution_tool(tool_id: &str) -> Option<PermissionRequestType> {
    match tool_id {
        "shell.command" => Some(PermissionRequestType::Command),
        "patch.apply" | "file.edit" | "file.write" | "file.multi_edit" => {
            Some(PermissionRequestType::FileWrite)
        }
        "artifact.export" => Some(PermissionRequestType::ArtifactExport),
        _ => None,
    }
}

fn permission_tool_for_execution(
    tool_id: &str,
) -> Box<dyn crate::permission_policy::PermissionCheck> {
    match tool_id {
        "shell.command" => Box::new(ShellCommandTool),
        "patch.apply" => Box::new(PatchApplyTool),
        "file.write" => Box::new(FileWriteTool),
        "file.edit" | "file.multi_edit" => Box::new(FileEditTool),
        _ => Box::new(DefaultTool::new(tool_id)),
    }
}

fn permission_summary(tool_id: &str, args: &ToolExecutionArgs) -> Option<String> {
    match tool_id {
        "shell.command" => args.command.clone(),
        "patch.apply" | "file.edit" | "file.write" | "file.multi_edit" => args.path.clone(),
        _ => None,
    }
}

fn permission_args_json(args: &ToolExecutionArgs) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    if let Some(value) = &args.path {
        object.insert("path".to_string(), serde_json::Value::String(value.clone()));
        object.insert(
            "file_path".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.command {
        object.insert(
            "command".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.content {
        object.insert(
            "content".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.old_string {
        object.insert(
            "old_string".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.new_string {
        object.insert(
            "new_string".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    serde_json::Value::Object(object)
}

/// Maximum tool concurrency.
pub const MAX_TOOL_CONCURRENCY: usize = 10;

/// Execute a single batch concurrently, with sibling abort propagation.
/// Results preserve the input order: the returned Vec is aligned with `batch.tools`.
pub fn execute_tool_batch_concurrent(
    batch: &ToolBatch,
    workspace_root: PathBuf,
    abort: &SiblingAbortController,
) -> Vec<ToolExecutionResult> {
    let num_tools = batch.tools.len();
    let results = Arc::new(Mutex::new(vec![
        ToolExecutionResult {
            tool_call_id: String::new(),
            tool_id: String::new(),
            ok: false,
            preview: String::new(),
            detail_json: "{}".to_string(),
            exit_code: Some(1),
        };
        num_tools
    ]));
    let mut handles = Vec::new();
    let abort_on_sibling_error = batch.abort_on_sibling_error;

    for (idx, tool) in batch.tools.iter().enumerate() {
        if abort.is_aborted() {
            break;
        }

        let tool = tool.clone();
        let ws = workspace_root.clone();
        let abort = abort.clone();
        let results = results.clone();

        let handle = thread::spawn(move || {
            if abort.is_aborted() {
                return;
            }

            let request = ToolExecutionRequest {
                workspace_root: ws,
                tool_call_id: tool.tool_call_id.clone(),
                tool_id: tool.tool_id.clone(),
                mode: ToolExecutionMode::ApplyWithPermission {
                    permission_decision: None,
                },
                args: tool.args,
            };

            let result = match execute_tool(&request) {
                Ok(result) => {
                    if tool.tool_id == "shell.command" && !result.ok && abort_on_sibling_error {
                        abort.abort();
                    }
                    result
                }
                Err(error) => ToolExecutionResult {
                    tool_call_id: tool.tool_call_id.clone(),
                    tool_id: tool.tool_id.clone(),
                    ok: false,
                    preview: format!("execution error: {error:?}"),
                    detail_json: "{}".to_string(),
                    exit_code: Some(1),
                },
            };

            if let Ok(mut guard) = results.lock() {
                if idx < guard.len() {
                    guard[idx] = result;
                }
            }
        });

        handles.push(handle);

        if handles.len() >= MAX_TOOL_CONCURRENCY {
            for handle in handles.drain(..) {
                if let Err(panic) = handle.join() {
                    let panic_msg = panic
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| panic.downcast_ref::<String>().map(|value| value.as_str()))
                        .unwrap_or("unknown panic");
                    eprintln!("tool thread panicked: {panic_msg}");
                }
            }
        }
    }

    for handle in handles {
        if let Err(panic) = handle.join() {
            let panic_msg = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(|value| value.as_str()))
                .unwrap_or("unknown panic");
            eprintln!("tool thread panicked: {panic_msg}");
        }
    }

    match Arc::try_unwrap(results) {
        Ok(mutex) => mutex.into_inner().unwrap(),
        Err(arc) => arc.lock().unwrap().clone(),
    }
}

pub fn execute_tool_preview(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    if !matches!(request.mode, ToolExecutionMode::ReadOnlyPreview) {
        return Err(ToolExecutionError::NonReadOnlyTool(request.tool_id.clone()));
    }
    let spec = find_tool_spec(&request.tool_id)
        .ok_or_else(|| ToolExecutionError::UnknownTool(request.tool_id.clone()))?;
    if spec.permission_required {
        return Err(ToolExecutionError::PermissionRequired(
            request.tool_id.clone(),
        ));
    }
    if spec.risk != ToolRisk::ReadOnly
        && !matches!(
            request.tool_id.as_str(),
            "todo.write" | "ask_user" | "plan.write" | "task.dispatch"
        )
    {
        return Err(ToolExecutionError::NonReadOnlyTool(request.tool_id.clone()));
    }
    match request.tool_id.as_str() {
        "file.read" => execute_file_read_preview(request),
        "file.list_directory" => execute_list_directory_preview(request),
        "file.list_tree" => execute_list_tree_preview(request),
        "search.ripgrep" => execute_search_preview(request),
        "repo.map" => execute_repo_map_preview(request),
        "git.status" => execute_git_status_preview(request),
        "research.csv_profile" => execute_research_csv_profile_preview(request),
        "lsp.diagnostics" => execute_lsp_diagnostics_preview(request),
        "todo.write" => execute_todo_write_preview(request),
        "plan.write" => execute_plan_write_preview(request),
        "ask_user" => execute_ask_user_preview(request),
        "task.dispatch" => execute_task_dispatch_preview(request),
        other => Err(ToolExecutionError::UnknownTool(other.to_string())),
    }
}

fn execute_research_csv_profile_preview(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let input_csv = resolve_within_workspace(
        &request.workspace_root,
        request
            .args
            .input_csv
            .as_deref()
            .or(request.args.path.as_deref())
            .ok_or_else(|| ToolExecutionError::MissingArgument("input_csv".to_string()))?,
    )?;
    let job_id = request
        .args
        .job_id
        .clone()
        .unwrap_or_else(|| request.tool_call_id.clone());
    let output_dir = match request.args.output_dir.as_deref() {
        Some(value) => resolve_output_dir(&request.workspace_root, value)?,
        None => std::env::temp_dir().join(format!("researchcode-tool-exec-{job_id}")),
    };
    let worker_cwd = resolve_within_workspace(&request.workspace_root, "workers/research_worker")?;
    let result = run_csv_profile_sidecar(&ResearchCsvProfileRequest {
        job_id,
        input_csv: input_csv.clone(),
        output_dir,
        worker_cwd,
        limits: ResearchWorkerLimits::default(),
    })
    .map_err(|error| ToolExecutionError::ToolFailed(format!("{error:?}")))?;
    let manifest = result
        .manifest_path
        .as_ref()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default();
    let manifest_hash = result.manifest_content_hash.clone().unwrap_or_default();
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: result.exit_code == 0,
        preview: format!(
            "research csv profile exit={} artifacts={} manifest_hash={}",
            result.exit_code, result.artifact_count, manifest_hash
        ),
        detail_json: format!(
            "{{\"input_csv\":{},\"manifest_path\":{},\"manifest_hash\":{},\"artifact_count\":{},\"stdout\":{},\"stderr\":{}}}",
            json_string(&relative_display(&request.workspace_root, &input_csv)),
            json_string(&manifest),
            json_string(&manifest_hash),
            result.artifact_count,
            json_string(&result.stdout),
            json_string(&result.stderr)
        ),
        exit_code: Some(result.exit_code),
    })
}

fn execute_shell_command(
    request: &ToolExecutionRequest,
    permission_decision: Option<PermissionDecisionKind>,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let cwd_arg = request.args.root.as_deref().unwrap_or(".");
    let cwd = resolve_within_workspace(&request.workspace_root, cwd_arg)?;
    let command = request
        .args
        .command
        .clone()
        .ok_or_else(|| ToolExecutionError::MissingArgument("command".to_string()))?;
    let plan = prepare_command(CommandRequest {
        command: command.clone(),
        cwd: cwd.to_string_lossy().to_string(),
    });
    let started = Instant::now();
    let output = run_prepared_command(&plan, permission_decision).map_err(|error| match error {
        CommandRunError::NotAuthorized(_) => {
            ToolExecutionError::PermissionRequired(request.tool_id.clone())
        }
        other => ToolExecutionError::ToolFailed(format!("{other:?}")),
    })?;
    let duration_ms = started.elapsed().as_millis();
    let stdout = redact_text_for_secrets(&output.stdout);
    let stderr = redact_text_for_secrets(&output.stderr);
    let stdout_tail = text_tail(&stdout, 2000);
    let stderr_tail = text_tail(&stderr, 2000);
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: output.exit_code == 0,
        preview: format_shell_command_preview(
            &command,
            output.exit_code,
            duration_ms,
            &stdout,
            &stderr,
        ),
        detail_json: format!(
            "{{\"command\":{},\"cwd\":{},\"exit_code\":{},\"classifier_decision\":{},\"classifier_reasons\":{},\"stdout\":{},\"stderr\":{},\"stdout_tail\":{},\"stderr_tail\":{}}}",
            json_string(&command),
            json_string(&relative_display(&request.workspace_root, &cwd)),
            output.exit_code,
            json_string(&format!("{:?}", plan.classifier_decision)),
            json_string(&plan.classifier_reasons.join("; ")),
            json_string(&stdout),
            json_string(&stderr),
            json_string(&stdout_tail),
            json_string(&stderr_tail)
        ),
        exit_code: Some(output.exit_code),
    })
}

fn execute_patch_apply(
    request: &ToolExecutionRequest,
    permission_decision: Option<PermissionDecisionKind>,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    match permission_decision {
        Some(PermissionDecisionKind::AllowOnce)
        | Some(PermissionDecisionKind::AllowSession)
        | Some(PermissionDecisionKind::AllowProjectRule) => {}
        Some(PermissionDecisionKind::Deny) | Some(PermissionDecisionKind::Modify) => {
            return Err(ToolExecutionError::PermissionRequired(
                request.tool_id.clone(),
            ))
        }
        None => {
            return Err(ToolExecutionError::PermissionRequired(
                request.tool_id.clone(),
            ))
        }
    }
    let path = resolve_write_path_within_workspace(
        &request.workspace_root,
        request
            .args
            .path
            .as_deref()
            .ok_or_else(|| ToolExecutionError::MissingArgument("path".to_string()))?,
    )?;
    let old_string = request
        .args
        .old_string
        .clone()
        .ok_or_else(|| ToolExecutionError::MissingArgument("old_string".to_string()))?;
    let new_string = request
        .args
        .new_string
        .clone()
        .ok_or_else(|| ToolExecutionError::MissingArgument("new_string".to_string()))?;
    let current_text = std::fs::read_to_string(&path).ok();
    let current_hash = current_text.as_deref().map(stable_text_hash);
    let base_hash = request.args.base_hash.clone().unwrap_or_default();
    if current_text.is_some() && !old_string.is_empty() && base_hash.is_empty() {
        return Err(ToolExecutionError::MissingArgument("base_hash".to_string()));
    }
    let validation = crate::patch::validate_patch_allowing_protected(PatchCheck {
        path: &path.to_string_lossy(),
        current_text: current_text.as_deref(),
        current_hash: current_hash.as_deref(),
        old_string: &old_string,
        base_hash: &base_hash,
    });
    if !matches!(
        validation,
        PatchValidation::Pass | PatchValidation::PassCreate
    ) {
        return Err(ToolExecutionError::ValidationFailed(format!(
            "{validation:?}"
        )));
    }
    apply_replace_patch_allowing_protected(&ReplacePatch {
        path: path.clone(),
        old_string,
        new_string,
        base_hash,
    })
    .map_err(|error| ToolExecutionError::ToolFailed(format!("{error:?}")))?;
    let relative = relative_display(&request.workspace_root, &path);
    let rollback = write_rollback_artifact(&path, current_text.as_deref())?;
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: format!(
            "patch applied to {relative} validation={validation:?} rollback={rollback}"
        ),
        detail_json: format!(
            "{{\"path\":{},\"validation\":{},\"rollback_artifact\":{}}}",
            json_string(&relative),
            json_string(&format!("{validation:?}")),
            json_string(&rollback)
        ),
        exit_code: None,
    })
}

fn execute_file_edit(
    request: &ToolExecutionRequest,
    permission_decision: Option<PermissionDecisionKind>,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    ensure_write_decision(&request.tool_id, permission_decision)?;
    let path = resolve_write_path_within_workspace(
        &request.workspace_root,
        request
            .args
            .path
            .as_deref()
            .ok_or_else(|| ToolExecutionError::MissingArgument("path".to_string()))?,
    )?;
    let old_string = request
        .args
        .old_string
        .as_deref()
        .ok_or_else(|| ToolExecutionError::MissingArgument("old_string".to_string()))?;
    if old_string.is_empty() {
        return Err(ToolExecutionError::ValidationFailed(
            "file.edit requires a non-empty old_string; use file.write for create/replace"
                .to_string(),
        ));
    }
    let new_string = request
        .args
        .new_string
        .as_deref()
        .ok_or_else(|| ToolExecutionError::MissingArgument("new_string".to_string()))?;
    let base_hash = request
        .args
        .base_hash
        .as_deref()
        .ok_or_else(|| ToolExecutionError::MissingArgument("base_hash".to_string()))?;
    let replace_all = request.args.replace_all.unwrap_or(false);
    let source = read_text_preserving_style(&path)?;
    let current_hash = stable_text_hash(&source.text);
    if base_hash != current_hash {
        return Err(ToolExecutionError::ValidationFailed(
            "FailStale".to_string(),
        ));
    }
    let old = convert_to_line_ending(&normalize_line_endings(old_string), source.line_ending);
    let new = convert_to_line_ending(&normalize_line_endings(new_string), source.line_ending);
    if old == new {
        return Err(ToolExecutionError::ValidationFailed(
            "old_string and new_string are identical".to_string(),
        ));
    }
    let count = source.text.matches(&old).count();
    if count == 0 {
        return Err(ToolExecutionError::ValidationFailed(
            "FailMissingOldString".to_string(),
        ));
    }
    if count > 1 && !replace_all {
        return Err(ToolExecutionError::ValidationFailed(
            "FailAmbiguous".to_string(),
        ));
    }
    let next = if replace_all {
        source.text.replace(&old, &new)
    } else {
        source.text.replacen(&old, &new, 1)
    };
    let rollback = write_rollback_artifact(&path, Some(&source.text))?;
    write_text_preserving_bom(&path, &next, source.has_bom)?;
    let relative = relative_display(&request.workspace_root, &path);
    let new_hash = stable_text_hash(&next);
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: format_file_edit_preview(
            &relative,
            if replace_all { count } else { 1 },
            base_hash,
            &new_hash,
            &source.text,
            &next,
        ),
        detail_json: format!(
            "{{\"path\":{},\"base_hash\":{},\"new_hash\":{},\"replacement_count\":{},\"replace_all\":{},\"rollback_artifact\":{}}}",
            json_string(&relative),
            json_string(base_hash),
            json_string(&new_hash),
            if replace_all { count } else { 1 },
            replace_all,
            json_string(&rollback)
        ),
        exit_code: None,
    })
}

fn execute_file_write(
    request: &ToolExecutionRequest,
    permission_decision: Option<PermissionDecisionKind>,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    ensure_write_decision(&request.tool_id, permission_decision)?;
    let path = resolve_write_path_within_workspace(
        &request.workspace_root,
        request
            .args
            .path
            .as_deref()
            .ok_or_else(|| ToolExecutionError::MissingArgument("path".to_string()))?,
    )?;
    let content = request
        .args
        .content
        .as_deref()
        .or(request.args.new_string.as_deref())
        .ok_or_else(|| ToolExecutionError::MissingArgument("content".to_string()))?;
    let current_source = read_text_preserving_style(&path).ok();
    if let Some(existing) = current_source.as_ref() {
        let expected = request
            .args
            .base_hash
            .as_deref()
            .ok_or_else(|| ToolExecutionError::MissingArgument("base_hash".to_string()))?;
        let actual = stable_text_hash(&existing.text);
        if expected != actual {
            return Err(ToolExecutionError::ValidationFailed(
                "FailStale".to_string(),
            ));
        }
    }
    let rollback = write_rollback_artifact(
        &path,
        current_source.as_ref().map(|source| source.text.as_str()),
    )?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?;
    }
    // Secondary TOCTOU check: re-read and verify hash right before write,
    // consistent with execute_patch_apply to prevent stale writes.
    if current_source.is_some() {
        let expected = request
            .args
            .base_hash
            .as_deref()
            .ok_or_else(|| ToolExecutionError::MissingArgument("base_hash".to_string()))?;
        let recheck = read_text_preserving_style(&path)
            .map_err(|_| ToolExecutionError::ValidationFailed("FailStale".to_string()))?;
        let recheck_hash = stable_text_hash(&recheck.text);
        if expected != recheck_hash {
            return Err(ToolExecutionError::ValidationFailed(
                "FailStale".to_string(),
            ));
        }
    }
    let final_content = if let Some(source) = current_source.as_ref() {
        convert_to_line_ending(&normalize_line_endings(content), source.line_ending)
    } else {
        content.to_string()
    };
    if let Some(source) = current_source.as_ref() {
        write_text_preserving_bom(&path, &final_content, source.has_bom)?;
    } else {
        fs::write(&path, &final_content)
            .map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?;
    }
    let relative = relative_display(&request.workspace_root, &path);
    let content_hash = stable_text_hash(&final_content);
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: format_file_write_preview(
            &relative,
            final_content.len(),
            &content_hash,
            &rollback,
        ),
        detail_json: format!(
            "{{\"path\":{},\"content_hash\":{},\"rollback_artifact\":{}}}",
            json_string(&relative),
            json_string(&content_hash),
            json_string(&rollback)
        ),
        exit_code: None,
    })
}

fn execute_file_multi_edit(
    request: &ToolExecutionRequest,
    permission_decision: Option<PermissionDecisionKind>,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    ensure_write_decision(&request.tool_id, permission_decision)?;
    let path = resolve_write_path_within_workspace(
        &request.workspace_root,
        request
            .args
            .path
            .as_deref()
            .ok_or_else(|| ToolExecutionError::MissingArgument("path".to_string()))?,
    )?;
    let base_hash = request
        .args
        .base_hash
        .as_deref()
        .ok_or_else(|| ToolExecutionError::MissingArgument("base_hash".to_string()))?;
    let edits_json = request
        .args
        .edits_json
        .as_deref()
        .ok_or_else(|| ToolExecutionError::MissingArgument("edits".to_string()))?;
    let source = read_text_preserving_style(&path)?;
    let current_hash = stable_text_hash(&source.text);
    if base_hash != current_hash {
        return Err(ToolExecutionError::ValidationFailed(
            "FailStale".to_string(),
        ));
    }
    let edits = parse_simple_edits(edits_json)?;
    if edits.is_empty() {
        return Err(ToolExecutionError::MissingArgument("edits".to_string()));
    }
    let mut next = source.text.clone();
    let mut applied = 0usize;
    for edit in edits {
        let old = convert_to_line_ending(
            &normalize_line_endings(&edit.old_string),
            source.line_ending,
        );
        let new = convert_to_line_ending(
            &normalize_line_endings(&edit.new_string),
            source.line_ending,
        );
        let count = next.matches(&old).count();
        if count == 0 {
            return Err(ToolExecutionError::ValidationFailed(
                "FailMissingOldString".to_string(),
            ));
        }
        if count > 1 && !edit.replace_all {
            return Err(ToolExecutionError::ValidationFailed(
                "FailAmbiguous".to_string(),
            ));
        }
        next = if edit.replace_all {
            applied += count;
            next.replace(&old, &new)
        } else {
            applied += 1;
            next.replacen(&old, &new, 1)
        };
    }
    let rollback = write_rollback_artifact(&path, Some(&source.text))?;
    write_text_preserving_bom(&path, &next, source.has_bom)?;
    let relative = relative_display(&request.workspace_root, &path);
    let new_hash = stable_text_hash(&next);
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: format_file_multi_edit_preview(&relative, applied, base_hash, &new_hash, &rollback),
        detail_json: format!(
            "{{\"path\":{},\"base_hash\":{},\"new_hash\":{},\"replacement_count\":{},\"rollback_artifact\":{}}}",
            json_string(&relative),
            json_string(base_hash),
            json_string(&new_hash),
            applied,
            json_string(&rollback)
        ),
        exit_code: None,
    })
}

fn execute_file_read_preview(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let raw_path = request
        .args
        .path
        .as_deref()
        .ok_or_else(|| ToolExecutionError::MissingArgument("path".to_string()))?;
    let path = match resolve_within_workspace(&request.workspace_root, raw_path) {
        Ok(path) => path,
        Err(ToolExecutionError::ToolFailed(error)) if error.contains("No such file") => {
            return Ok(structured_tool_error_result(
                request,
                "path_not_found",
                raw_path,
                true,
                "Use file.list_directory on the nearest existing parent, then read a concrete file path from that listing.",
            ));
        }
        Err(error) => return Err(error),
    };
    if path.is_dir() {
        return Ok(structured_tool_error_result(
            request,
            "path_is_directory",
            &relative_display(&request.workspace_root, &path),
            true,
            "Use file.list_directory or file.list_tree on this directory; file.read only accepts concrete files.",
        ));
    }
    let read = read_file(
        &FileReadRequest {
            path: path.clone(),
            max_bytes: request.args.max_bytes.unwrap_or(8_000).clamp(1, 80_000),
        },
        &request.workspace_root,
    )
    .map_err(|error| ToolExecutionError::ToolFailed(format!("{error:?}")))?;
    let slice = match slice_lines(
        &read.content,
        request.args.offset.unwrap_or(0),
        request.args.limit,
    ) {
        Ok(slice) => slice,
        Err(error) => {
            let relative = relative_display(&request.workspace_root, &path);
            return Ok(structured_file_read_range_error_result(
                request,
                &relative,
                read.size_bytes,
                read.truncated,
                error,
            ));
        }
    };
    let content_hash = stable_text_hash(&read.content);
    let relative = relative_display(&request.workspace_root, &path);
    let returned_line_count = if slice.line_end >= slice.line_start && slice.line_start > 0 {
        slice.line_end - slice.line_start + 1
    } else {
        0
    };
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: format_file_read_preview(
            &relative,
            &slice.content,
            slice.line_start,
            returned_line_count,
            slice.total_lines,
        ),
        detail_json: format!(
            "{{\"path\":{},\"size_bytes\":{},\"truncated\":{},\"line_start\":{},\"line_end\":{},\"line_count\":{},\"content_hash\":{},\"content\":{}}}",
            json_string(&relative),
            read.size_bytes,
            read.truncated,
            slice.line_start,
            slice.line_end,
            slice.total_lines,
            json_string(&content_hash),
            json_string(&slice.content)
        ),
        exit_code: None,
    })
}

fn execute_list_directory_preview(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let raw_path = request
        .args
        .path
        .as_deref()
        .or(request.args.root.as_deref())
        .unwrap_or(".");
    let path = match resolve_within_workspace(&request.workspace_root, raw_path) {
        Ok(path) => path,
        Err(ToolExecutionError::ToolFailed(error)) if error.contains("No such file") => {
            return Ok(structured_tool_error_result(
                request,
                "path_not_found",
                raw_path,
                true,
                "Use repo.map on the nearest existing parent, then retry file.list_directory with a concrete path.",
            ));
        }
        Err(error) => return Err(error),
    };
    if !path.is_dir() {
        return Ok(structured_tool_error_result(
            request,
            "path_not_directory",
            &relative_display(&request.workspace_root, &path),
            true,
            "Use file.read for files, and file.list_directory for directories.",
        ));
    }
    let include_hidden = request.args.include_hidden.unwrap_or(false);
    let max_entries = request.args.max_results.unwrap_or(200).clamp(1, 2000);
    let mut entries = fs::read_dir(&path)
        .map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    let mut serialized_entries = Vec::new();
    let mut omitted_count = 0usize;
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if !include_hidden && name.starts_with('.') {
            omitted_count += 1;
            continue;
        }
        let entry_path = entry.path();
        if is_sensitive_path(&entry_path.to_string_lossy()) {
            omitted_count += 1;
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?;
        let kind = if file_type.is_dir() {
            "directory"
        } else if file_type.is_file() {
            "file"
        } else if file_type.is_symlink() {
            "symlink"
        } else {
            "other"
        };
        let size_bytes = if file_type.is_file() {
            entry.metadata().ok().map(|meta| meta.len())
        } else {
            None
        };
        let extension = entry_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        serialized_entries.push(format!(
            "{{\"name\":{},\"kind\":\"{}\",\"size_bytes\":{},\"extension\":{}}}",
            json_string(&name),
            kind,
            size_bytes
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
            if extension.is_empty() {
                "null".to_string()
            } else {
                json_string(&extension)
            }
        ));
        if serialized_entries.len() >= max_entries {
            break;
        }
    }
    let rel = relative_display(&request.workspace_root, &path);
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: format_list_directory_preview(&rel, serialized_entries.len(), omitted_count),
        detail_json: format!(
            "{{\"path\":{},\"entry_count\":{},\"omitted_count\":{},\"entries\":[{}]}}",
            json_string(&rel),
            serialized_entries.len(),
            omitted_count,
            serialized_entries.join(",")
        ),
        exit_code: None,
    })
}

fn execute_list_tree_preview(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let raw_path = request
        .args
        .path
        .as_deref()
        .or(request.args.root.as_deref())
        .unwrap_or(".");
    let path = match resolve_within_workspace(&request.workspace_root, raw_path) {
        Ok(path) => path,
        Err(ToolExecutionError::ToolFailed(error)) if error.contains("No such file") => {
            return Ok(structured_tool_error_result(
                request,
                "path_not_found",
                raw_path,
                true,
                "Use repo.map on the nearest existing parent, then retry file.list_tree with a concrete path.",
            ));
        }
        Err(error) => return Err(error),
    };
    if !path.is_dir() {
        return Ok(structured_tool_error_result(
            request,
            "path_not_directory",
            &relative_display(&request.workspace_root, &path),
            true,
            "Use file.read for files, and file.list_tree for directories.",
        ));
    }
    let depth = request.args.max_depth.unwrap_or(2).clamp(1, 6);
    let max_entries = request.args.max_results.unwrap_or(240).clamp(1, 2000);
    let map = build_repo_map(&RepoMapRequest {
        root: path.clone(),
        max_files: max_entries,
        max_depth: depth,
    })
    .map_err(ToolExecutionError::ToolFailed)?;
    let rel = relative_display(&request.workspace_root, &path);
    let tree_head = map
        .tree_lines
        .iter()
        .take(max_entries)
        .cloned()
        .collect::<Vec<_>>();
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: format_list_tree_preview(&rel, tree_head.len(), map.file_count, map.omitted_count),
        detail_json: format!(
            "{{\"path\":{},\"line_count\":{},\"file_count\":{},\"omitted_count\":{},\"tree_lines\":{}}}",
            json_string(&rel),
            tree_head.len(),
            map.file_count,
            map.omitted_count,
            json_string_array(&tree_head)
        ),
        exit_code: None,
    })
}

fn structured_tool_error_result(
    request: &ToolExecutionRequest,
    error_code: &str,
    path: &str,
    recoverable: bool,
    next_action_hint: &str,
) -> ToolExecutionResult {
    let suggested_tool = match error_code {
        "path_is_directory" => "file.list_directory",
        "path_not_found" => "repo.map",
        _ => "none",
    };
    ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: false,
        preview: format!(
            "tool error {error_code} path={path}; next_action={next_action_hint}"
        ),
        detail_json: format!(
            "{{\"ok\":false,\"error_code\":{},\"path\":{},\"recoverable\":{},\"suggested_tool\":{},\"next_action_hint\":{},\"artifact_ref\":null}}",
            json_string(error_code),
            json_string(path),
            recoverable,
            json_string(suggested_tool),
            json_string(next_action_hint)
        ),
        exit_code: None,
    }
}

fn structured_search_error_result(
    request: &ToolExecutionRequest,
    error_code: &str,
    path: &str,
    pattern: &str,
    recoverable: bool,
    next_action_hint: &str,
) -> ToolExecutionResult {
    let suggested_tool = match error_code {
        "path_not_found" => "repo.map",
        _ => "none",
    };
    ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: false,
        preview: format!(
            "tool error {error_code} path={path}; pattern={pattern}; next_action={next_action_hint}"
        ),
        detail_json: format!(
            "{{\"ok\":false,\"error_code\":{},\"path\":{},\"pattern\":{},\"matches\":[],\"match_count\":0,\"truncated\":false,\"searched_files\":0,\"recoverable\":{},\"suggested_tool\":{},\"next_action_hint\":{},\"artifact_ref\":null}}",
            json_string(error_code),
            json_string(path),
            json_string(pattern),
            recoverable,
            json_string(suggested_tool),
            json_string(next_action_hint)
        ),
        exit_code: None,
    }
}

fn structured_file_read_range_error_result(
    request: &ToolExecutionRequest,
    path: &str,
    size_bytes: usize,
    source_truncated: bool,
    error: LineSliceError,
) -> ToolExecutionResult {
    let valid_range = if error.total_lines == 0 {
        "empty_file".to_string()
    } else {
        format!("offset 0..{}", error.total_lines.saturating_sub(1))
    };
    let next_action_hint = if source_truncated {
        "The requested line range is outside the bytes already loaded. Retry once with a larger max_bytes only if the missing suffix is essential; otherwise use the prior evidence and continue."
    } else if error.total_lines == 0 {
        "The file is empty. Do not retry this file range; use other evidence or continue the task."
    } else {
        "The requested range is past EOF or empty. Do not keep paging this file; use the prior evidence, choose a valid offset, or continue implementation."
    };
    ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: false,
        preview: format!(
            "tool error READ_RANGE_EMPTY_OR_EOF path={path}; requested offset={} limit={}; line_count={}; valid_range={valid_range}",
            error.requested_offset,
            error
                .requested_limit
                .map(|value| value.to_string())
                .unwrap_or_else(|| "all".to_string()),
            error.total_lines
        ),
        detail_json: format!(
            "{{\"ok\":false,\"error_code\":\"READ_RANGE_EMPTY_OR_EOF\",\"path\":{},\"size_bytes\":{},\"source_truncated\":{},\"requested_offset\":{},\"requested_limit\":{},\"line_count\":{},\"valid_range\":{},\"recoverable\":true,\"suggested_tool\":\"none\",\"next_action_hint\":{},\"artifact_ref\":null}}",
            json_string(path),
            size_bytes,
            source_truncated,
            error.requested_offset,
            json_optional_usize(error.requested_limit),
            error.total_lines,
            json_string(&valid_range),
            json_string(next_action_hint)
        ),
        exit_code: None,
    }
}

fn execute_lsp_diagnostics_preview(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: "lsp diagnostics unavailable in current runtime; gated adapter required"
            .to_string(),
        detail_json: "{\"available\":false,\"diagnostics\":[]}".to_string(),
        exit_code: None,
    })
}

fn execute_todo_write_preview(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let todo_text = request
        .args
        .content
        .as_deref()
        .or(request.args.edits_json.as_deref())
        .ok_or_else(|| ToolExecutionError::MissingArgument("content".to_string()))?;
    let item_count = todo_text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
        .max(1);
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: format!("todo.write updated todo view items={item_count}"),
        detail_json: format!(
            "{{\"item_count\":{},\"todo_text\":{}}}",
            item_count,
            json_string(todo_text)
        ),
        exit_code: None,
    })
}

fn execute_plan_write_preview(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let content = request
        .args
        .content
        .as_deref()
        .or(request.args.edits_json.as_deref())
        .ok_or_else(|| ToolExecutionError::MissingArgument("content".to_string()))?;
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview:
            "plan.write preview accepted; RuntimeFacade writes .researchcode/plans/<session_id>.md"
                .to_string(),
        detail_json: format!(
            "{{\"preview_only\":true,\"content_hash\":{}}}",
            json_string(&stable_text_hash(content))
        ),
        exit_code: None,
    })
}

fn execute_ask_user_preview(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let question = request
        .args
        .query
        .as_deref()
        .or(request.args.content.as_deref())
        .ok_or_else(|| ToolExecutionError::MissingArgument("query".to_string()))?;
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: "ask_user queued a user decision request".to_string(),
        detail_json: format!("{{\"question\":{}}}", json_string(question)),
        exit_code: None,
    })
}

fn execute_task_dispatch_preview(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let prompt = request
        .args
        .query
        .as_deref()
        .or(request.args.content.as_deref())
        .ok_or_else(|| ToolExecutionError::MissingArgument("prompt".to_string()))?;
    let model_role = request.args.model_role.as_deref().unwrap_or("compactor");
    let write_scope_json = request
        .args
        .write_scope_json
        .as_deref()
        .unwrap_or("{\"paths\":[]}");
    let write_scope_payload = if write_scope_json.trim_start().starts_with('{') {
        write_scope_json
    } else {
        "{\"paths\":[]}"
    };
    let task_id = format!("subagent_{}", stable_preview_id(prompt));
    let evidence = collect_task_dispatch_evidence(request, prompt);
    let evidence_preview = evidence
        .iter()
        .map(|item| item.preview.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    let evidence_refs = evidence
        .iter()
        .map(|item| json_string(&item.evidence_ref))
        .collect::<Vec<_>>()
        .join(",");
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: format!(
            "task.dispatch completed isolated subagent {task_id}: {}",
            if evidence_preview.is_empty() {
                prompt.chars().take(120).collect::<String>()
            } else {
                evidence_preview.chars().take(180).collect::<String>()
            }
        ),
        detail_json: format!(
            "{{\"task_id\":{},\"status\":\"completed\",\"prompt_preview\":{},\"llm_turn\":\"deterministic_local_child\",\"model_role\":{},\"write_scope\":{},\"write_scope_applied\":false,\"isolation\":\"deterministic_child_event_summary\",\"evidence_refs\":[{}],\"evidence_preview\":{}}}",
            json_string(&task_id),
            json_string(&prompt.chars().take(240).collect::<String>()),
            json_string(model_role),
            write_scope_payload,
            evidence_refs,
            json_string(&evidence_preview.chars().take(800).collect::<String>())
        ),
        exit_code: None,
    })
}

#[derive(Debug, Clone)]
struct TaskDispatchEvidence {
    evidence_ref: String,
    preview: String,
}

fn collect_task_dispatch_evidence(
    request: &ToolExecutionRequest,
    prompt: &str,
) -> Vec<TaskDispatchEvidence> {
    let mut evidence = Vec::new();
    collect_task_dispatch_tool(
        request,
        &mut evidence,
        "repo.map",
        ToolExecutionArgs {
            root: Some(".".to_string()),
            max_files: Some(80),
            max_depth: Some(3),
            ..ToolExecutionArgs::default()
        },
    );
    if let Some(path) = infer_task_dispatch_path(prompt) {
        collect_task_dispatch_tool(
            request,
            &mut evidence,
            "file.read",
            ToolExecutionArgs {
                path: Some(path),
                max_bytes: Some(12_000),
                ..ToolExecutionArgs::default()
            },
        );
    }
    if let Some(pattern) = infer_task_dispatch_pattern(prompt) {
        collect_task_dispatch_tool(
            request,
            &mut evidence,
            "search.ripgrep",
            ToolExecutionArgs {
                root: Some(".".to_string()),
                pattern: Some(pattern),
                max_results: Some(12),
                ..ToolExecutionArgs::default()
            },
        );
    }
    collect_task_dispatch_tool(
        request,
        &mut evidence,
        "git.status",
        ToolExecutionArgs {
            root: Some(".".to_string()),
            ..ToolExecutionArgs::default()
        },
    );
    evidence
}

fn collect_task_dispatch_tool(
    request: &ToolExecutionRequest,
    evidence: &mut Vec<TaskDispatchEvidence>,
    tool_id: &str,
    args: ToolExecutionArgs,
) {
    let tool_call_id = format!(
        "{}_child_{}_{}",
        request.tool_call_id,
        tool_id.replace('.', "_"),
        evidence.len() + 1
    );
    let result = execute_tool(&ToolExecutionRequest {
        workspace_root: request.workspace_root.clone(),
        tool_call_id,
        tool_id: tool_id.to_string(),
        mode: ToolExecutionMode::ReadOnlyPreview,
        args,
    })
    .unwrap_or_else(|error| ToolExecutionResult {
        tool_call_id: format!(
            "{}_child_error_{}",
            request.tool_call_id,
            evidence.len() + 1
        ),
        tool_id: tool_id.to_string(),
        ok: false,
        preview: format!("{tool_id} error: {error:?}"),
        detail_json: format!("{{\"error\":{}}}", json_string(&format!("{error:?}"))),
        exit_code: None,
    });
    evidence.push(TaskDispatchEvidence {
        evidence_ref: format!("{tool_id}:{}", stable_preview_id(&result.detail_json)),
        preview: format!(
            "{} ok={} {}",
            tool_id,
            result.ok,
            result.preview.chars().take(180).collect::<String>()
        ),
    });
}

fn infer_task_dispatch_path(prompt: &str) -> Option<String> {
    prompt
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| {
                matches!(ch, '"' | '\'' | '`' | ',' | ';' | ':' | ')' | '(')
            })
        })
        .find(|token| {
            token.contains('/')
                || token.ends_with(".rs")
                || token.ends_with(".md")
                || token.ends_with(".toml")
                || token.ends_with(".ts")
                || token.ends_with(".tsx")
                || token.ends_with(".json")
        })
        .map(str::to_string)
}

fn infer_task_dispatch_pattern(prompt: &str) -> Option<String> {
    for marker in ["search ", "find ", "grep ", "查找", "搜索"] {
        if let Some(index) = prompt.to_ascii_lowercase().find(marker.trim()) {
            let start = index + marker.len();
            let value = prompt[start..]
                .split(['\n', '.', ',', ';'])
                .next()
                .unwrap_or_default()
                .trim();
            if value.chars().count() >= 3 {
                return Some(value.chars().take(80).collect());
            }
        }
    }
    None
}

fn stable_preview_id(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn execute_search_preview(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let target_arg = request
        .args
        .path
        .as_deref()
        .or(request.args.root.as_deref())
        .unwrap_or(".");
    let pattern = request
        .args
        .pattern
        .as_deref()
        .or(request.args.query.as_deref())
        .unwrap_or("")
        .trim();
    if pattern.is_empty() {
        return Ok(structured_search_error_result(
            request,
            "empty_pattern",
            target_arg,
            pattern,
            true,
            "Provide a non-empty pattern before retrying search.ripgrep.",
        ));
    }
    let root = match resolve_within_workspace(&request.workspace_root, target_arg) {
        Ok(root) => root,
        Err(ToolExecutionError::ToolFailed(error)) if error.contains("No such file") => {
            return Ok(structured_search_error_result(
                request,
                "path_not_found",
                target_arg,
                pattern,
                true,
                "Use repo.map on the nearest existing parent, then search within a concrete workspace file or directory.",
            ));
        }
        Err(ToolExecutionError::PathEscapesWorkspace(path)) => {
            return Ok(structured_search_error_result(
                request,
                "path_escapes_workspace",
                &path,
                pattern,
                false,
                "Retry with a file or directory path inside the workspace.",
            ));
        }
        Err(error) => return Err(error),
    };
    let outcome = match search_text_with_outcome(
        &SearchRequest {
            root: root.clone(),
            pattern: pattern.to_string(),
            max_results: request.args.max_results.unwrap_or(20).clamp(1, 100),
        },
        &request.workspace_root,
    ) {
        Ok(outcome) => outcome,
        Err(SearchError::EmptyPattern) => {
            return Ok(structured_search_error_result(
                request,
                "empty_pattern",
                target_arg,
                pattern,
                true,
                "Provide a non-empty pattern before retrying search.ripgrep.",
            ));
        }
        Err(SearchError::PathNotFound(path)) => {
            return Ok(structured_search_error_result(
                request,
                "path_not_found",
                &path,
                pattern,
                true,
                "Use repo.map on the nearest existing parent, then search within a concrete workspace file or directory.",
            ));
        }
        Err(SearchError::PathEscapesWorkspace(path)) => {
            return Ok(structured_search_error_result(
                request,
                "path_escapes_workspace",
                &path,
                pattern,
                false,
                "Retry with a file or directory path inside the workspace.",
            ));
        }
        Err(SearchError::Io(error)) => return Err(ToolExecutionError::ToolFailed(error)),
    };
    let target_display = relative_display(&request.workspace_root, &root);
    let first = outcome
        .matches
        .first()
        .map(|item| {
            format!(
                "{}:{} {}",
                relative_display(&request.workspace_root, &item.path),
                item.line_number,
                item.line
            )
        })
        .unwrap_or_else(|| "no matches".to_string());
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: format!(
            "{} matches in {} files{}; {}",
            outcome.matches.len(),
            outcome.searched_files,
            if outcome.truncated { " (truncated)" } else { "" },
            first
        ),
        detail_json: format!(
            "{{\"path\":{},\"pattern\":{},\"match_count\":{},\"matches\":{},\"truncated\":{},\"searched_files\":{}}}",
            json_string(&target_display),
            json_string(pattern),
            outcome.matches.len(),
            json_search_matches(&request.workspace_root, &outcome.matches),
            outcome.truncated,
            outcome.searched_files
        ),
        exit_code: None,
    })
}

fn execute_repo_map_preview(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let root_arg = request
        .args
        .root
        .as_deref()
        .or(request.args.path.as_deref())
        .unwrap_or(".");
    let root = match resolve_within_workspace(&request.workspace_root, root_arg) {
        Ok(root) => root,
        Err(ToolExecutionError::ToolFailed(error)) if error.contains("No such file") => {
            return Ok(structured_tool_error_result(
                request,
                "path_not_found",
                root_arg,
                true,
                "Map an existing workspace directory such as . or a concrete nested root.",
            ));
        }
        Err(error) => return Err(error),
    };
    let repo_map = build_repo_map(&RepoMapRequest {
        root,
        max_files: request.args.max_files.unwrap_or(160).clamp(1, 400),
        max_depth: request.args.max_depth.unwrap_or(4).clamp(1, 8),
    })
    .map_err(ToolExecutionError::ToolFailed)?;
    let important_files = repo_map
        .important_files
        .iter()
        .take(24)
        .map(|path| relative_display(&request.workspace_root, path))
        .collect::<Vec<_>>();
    let tree_head = repo_map
        .tree_lines
        .iter()
        .take(80)
        .cloned()
        .collect::<Vec<_>>();
    let tree_preview = tree_head
        .iter()
        .take(12)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: format!(
            "repo map files={} omitted={} stack={} sample={}",
            repo_map.file_count,
            repo_map.omitted_count,
            repo_map.tech_stack.join(","),
            tree_preview
        ),
        detail_json: format!(
            "{{\"file_count\":{},\"omitted_count\":{},\"tech_stack\":{},\"important_files\":{},\"tree_head\":{},\"context_text\":{}}}",
            repo_map.file_count,
            repo_map.omitted_count,
            json_string(&repo_map.tech_stack.join(",")),
            json_string_array(&important_files),
            json_string_array(&tree_head),
            json_string(&repo_map.to_context_text())
        ),
        exit_code: None,
    })
}

fn execute_git_status_preview(
    request: &ToolExecutionRequest,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let root_arg = request.args.root.as_deref().unwrap_or(".");
    let cwd = match resolve_within_workspace(&request.workspace_root, root_arg) {
        Ok(cwd) => cwd,
        Err(ToolExecutionError::ToolFailed(error)) if error.contains("No such file") => {
            return Ok(structured_tool_error_result(
                request,
                "path_not_found",
                root_arg,
                true,
                "Use git.status on . or an existing nested git root.",
            ));
        }
        Err(error) => return Err(error),
    };
    let status = git_status(&GitStatusRequest { cwd });
    Ok(ToolExecutionResult {
        tool_call_id: request.tool_call_id.clone(),
        tool_id: request.tool_id.clone(),
        ok: true,
        preview: format!("{:?}", status.kind),
        detail_json: format!(
            "{{\"kind\":{}}}",
            json_string(&format!("{:?}", status.kind))
        ),
        exit_code: None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEnding {
    Lf,
    Crlf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextWithStyle {
    text: String,
    has_bom: bool,
    line_ending: LineEnding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SimpleEdit {
    old_string: String,
    new_string: String,
    replace_all: bool,
}

fn ensure_write_decision(
    tool_id: &str,
    permission_decision: Option<PermissionDecisionKind>,
) -> Result<(), ToolExecutionError> {
    match permission_decision {
        Some(PermissionDecisionKind::AllowOnce)
        | Some(PermissionDecisionKind::AllowSession)
        | Some(PermissionDecisionKind::AllowProjectRule) => Ok(()),
        Some(PermissionDecisionKind::Deny) | Some(PermissionDecisionKind::Modify) | None => {
            Err(ToolExecutionError::PermissionRequired(tool_id.to_string()))
        }
    }
}

fn read_text_preserving_style(path: &Path) -> Result<TextWithStyle, ToolExecutionError> {
    let bytes =
        fs::read(path).map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?;
    let (has_bom, body) = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        (true, &bytes[3..])
    } else {
        (false, bytes.as_slice())
    };
    let text = std::str::from_utf8(body)
        .map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?
        .to_string();
    let line_ending = if text.contains("\r\n") {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    };
    Ok(TextWithStyle {
        text,
        has_bom,
        line_ending,
    })
}

fn write_text_preserving_bom(
    path: &Path,
    text: &str,
    has_bom: bool,
) -> Result<(), ToolExecutionError> {
    let mut bytes = Vec::new();
    if has_bom {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    bytes.extend_from_slice(text.as_bytes());
    fs::write(path, bytes).map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn convert_to_line_ending(text: &str, ending: LineEnding) -> String {
    match ending {
        LineEnding::Lf => text.to_string(),
        LineEnding::Crlf => text.replace('\n', "\r\n"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LineSlice {
    content: String,
    line_start: usize,
    line_end: usize,
    total_lines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LineSliceError {
    requested_offset: usize,
    requested_limit: Option<usize>,
    total_lines: usize,
}

fn slice_lines(
    content: &str,
    offset: usize,
    limit: Option<usize>,
) -> Result<LineSlice, LineSliceError> {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        if offset == 0 && limit.unwrap_or(1) > 0 {
            return Ok(LineSlice {
                content: String::new(),
                line_start: 0,
                line_end: 0,
                total_lines: 0,
            });
        }
        return Err(LineSliceError {
            requested_offset: offset,
            requested_limit: limit,
            total_lines: 0,
        });
    }
    if offset >= lines.len() || limit == Some(0) {
        return Err(LineSliceError {
            requested_offset: offset,
            requested_limit: limit,
            total_lines: lines.len(),
        });
    }
    let start = offset;
    let take = limit.unwrap_or(lines.len().saturating_sub(start));
    let end = (start + take).min(lines.len());
    if end <= start {
        return Err(LineSliceError {
            requested_offset: offset,
            requested_limit: limit,
            total_lines: lines.len(),
        });
    }
    let text = lines[start..end].join("\n");
    Ok(LineSlice {
        content: text,
        line_start: start + 1,
        line_end: end,
        total_lines: lines.len(),
    })
}

fn write_rollback_artifact(
    path: &Path,
    previous_text: Option<&str>,
) -> Result<String, ToolExecutionError> {
    let payload = previous_text.unwrap_or("");
    let hash = stable_text_hash(payload);
    let root = std::env::temp_dir().join("researchcode-rollbacks");
    fs::create_dir_all(&root).map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?;
    let file = root.join(format!(
        "{}-{}.rollback.txt",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file"),
        hash
    ));
    fs::write(&file, payload).map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?;
    Ok(file.to_string_lossy().to_string())
}

fn parse_simple_edits(input: &str) -> Result<Vec<SimpleEdit>, ToolExecutionError> {
    let mut edits = Vec::new();
    let mut cursor = 0usize;
    while let Some(start_relative) = input[cursor..].find('{') {
        let start = cursor + start_relative;
        let Some(end_relative) = input[start..].find('}') else {
            break;
        };
        let end = start + end_relative + 1;
        let object = &input[start..end];
        let Some(old_string) = crate::tcml::extract_json_string(object, "old_string") else {
            cursor = end;
            continue;
        };
        let Some(new_string) = crate::tcml::extract_json_string(object, "new_string") else {
            cursor = end;
            continue;
        };
        edits.push(SimpleEdit {
            old_string,
            new_string,
            replace_all: extract_json_bool(object, "replace_all").unwrap_or(false),
        });
        cursor = end;
    }
    Ok(edits)
}

fn extract_json_bool(input: &str, key: &str) -> Option<bool> {
    let marker = format!("\"{key}\"");
    let key_start = input.find(&marker)? + marker.len();
    let after_key = input[key_start..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    if after_colon.starts_with("true") {
        Some(true)
    } else if after_colon.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn resolve_within_workspace(
    workspace_root: &Path,
    value: &str,
) -> Result<PathBuf, ToolExecutionError> {
    if value.trim().is_empty() {
        return Err(ToolExecutionError::MissingArgument("path".to_string()));
    }
    let root = workspace_root
        .canonicalize()
        .map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?;
    let candidate = read_candidate_with_model_path_repair(&root, value)?;
    let resolved = candidate
        .canonicalize()
        .map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?;
    // Check sensitive path AFTER canonicalization so symlinks can't bypass
    if is_sensitive_path(&resolved.to_string_lossy()) {
        return Err(ToolExecutionError::SensitivePath(value.to_string()));
    }
    if !resolved.starts_with(&root) {
        return Err(ToolExecutionError::PathEscapesWorkspace(value.to_string()));
    }
    Ok(resolved)
}

fn resolve_write_path_within_workspace(
    workspace_root: &Path,
    value: &str,
) -> Result<PathBuf, ToolExecutionError> {
    if value.trim().is_empty() {
        return Err(ToolExecutionError::MissingArgument("path".to_string()));
    }
    let root = workspace_root
        .canonicalize()
        .map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?;
    let candidate = write_candidate_with_model_path_repair(&root, value);
    let parent = candidate
        .parent()
        .ok_or_else(|| ToolExecutionError::PathEscapesWorkspace(value.to_string()))?;
    let resolved_parent = if parent.exists() {
        parent
            .canonicalize()
            .map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?
    } else {
        // Parent doesn't exist yet (new file in new directory) — canonicalize the
        // nearest existing ancestor to verify it stays within workspace.
        let mut ancestor = parent.to_path_buf();
        while !ancestor.exists() {
            ancestor = ancestor
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root.clone());
        }
        let resolved_ancestor = ancestor
            .canonicalize()
            .map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?;
        if !resolved_ancestor.starts_with(&root) {
            return Err(ToolExecutionError::PathEscapesWorkspace(value.to_string()));
        }
        // Reconstruct path from resolved ancestor
        let remainder = parent.strip_prefix(&ancestor).unwrap_or(Path::new(""));
        resolved_ancestor.join(remainder)
    };
    if !resolved_parent.starts_with(&root) {
        return Err(ToolExecutionError::PathEscapesWorkspace(value.to_string()));
    }
    // Sensitive writes are gated by PermissionResolver before ApplyWithPermission.
    // The executor keeps the workspace/symlink invariant here so an approved
    // protected-path write can proceed without bypassing path containment.
    // Normalize candidate to resolve .. components before returning,
    // preventing path traversal when intermediate directories don't exist.
    let normalized: PathBuf = candidate
        .components()
        .fold(PathBuf::new(), |mut acc, comp| {
            match comp {
                std::path::Component::ParentDir => {
                    acc.pop();
                }
                std::path::Component::CurDir => {}
                other => {
                    acc.push(other);
                }
            }
            acc
        });
    if !normalized.starts_with(&root) {
        return Err(ToolExecutionError::PathEscapesWorkspace(value.to_string()));
    }
    // Reject symlink final path component to prevent symlink traversal.
    // Only the parent directory was canonicalized above; the final component
    // of normalized could be a symlink pointing outside the workspace root.
    if normalized.exists() {
        if let Ok(meta) = normalized.symlink_metadata() {
            if meta.file_type().is_symlink() {
                return Err(ToolExecutionError::PathEscapesWorkspace(value.to_string()));
            }
        }
    }
    Ok(normalized)
}

fn read_candidate_with_model_path_repair(
    workspace_root: &Path,
    value: &str,
) -> Result<PathBuf, ToolExecutionError> {
    let trimmed = value.trim();
    let input = PathBuf::from(trimmed);
    if input.is_absolute() {
        if input == Path::new("/") {
            return Ok(workspace_root.to_path_buf());
        }
        if input.starts_with(workspace_root) {
            return Ok(input);
        }
        if let Some(candidate) = workspace_rooted_absolute_hint(workspace_root, trimmed) {
            return Ok(candidate);
        }
        return Err(ToolExecutionError::PathEscapesWorkspace(value.to_string()));
    }
    Ok(workspace_root.join(input))
}

fn write_candidate_with_model_path_repair(workspace_root: &Path, value: &str) -> PathBuf {
    let trimmed = value.trim();
    let input = PathBuf::from(trimmed);
    if input.is_absolute() {
        if input == Path::new("/") {
            return workspace_root.to_path_buf();
        }
        if input.starts_with(workspace_root) {
            return input;
        }
        if let Some(candidate) = workspace_rooted_absolute_hint_for_write(workspace_root, trimmed) {
            return candidate;
        }
        return input;
    }
    workspace_root.join(input)
}

fn workspace_rooted_absolute_hint(workspace_root: &Path, value: &str) -> Option<PathBuf> {
    let stripped = value.trim_start_matches('/');
    if stripped.is_empty() || is_sensitive_path(stripped) {
        return None;
    }
    let candidate = workspace_root.join(stripped);
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

fn workspace_rooted_absolute_hint_for_write(workspace_root: &Path, value: &str) -> Option<PathBuf> {
    let stripped = value.trim_start_matches('/');
    if stripped.is_empty() || is_sensitive_path(stripped) {
        return None;
    }
    let candidate = workspace_root.join(stripped);
    let parent = candidate.parent()?;
    if parent.exists() {
        Some(candidate)
    } else {
        None
    }
}

fn resolve_output_dir(workspace_root: &Path, value: &str) -> Result<PathBuf, ToolExecutionError> {
    let candidate = PathBuf::from(value);
    if candidate.is_absolute() {
        // Check sensitive path before resolving
        if is_sensitive_path(&candidate.to_string_lossy()) {
            return Err(ToolExecutionError::SensitivePath(value.to_string()));
        }
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?;
        let parent = candidate
            .parent()
            .ok_or_else(|| ToolExecutionError::PathEscapesWorkspace(value.to_string()))?;
        // If parent exists, canonicalize and check; otherwise fall through to workspace resolution
        if parent.exists() {
            let resolved_parent = parent
                .canonicalize()
                .map_err(|error| ToolExecutionError::ToolFailed(error.to_string()))?;
            if resolved_parent.starts_with(&temp_root) {
                return Ok(candidate);
            }
        }
        // For absolute paths not in temp, or where parent doesn't exist yet: fall through
    }
    resolve_write_path_within_workspace(workspace_root, value)
}

fn relative_display(root: &Path, path: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    if relative.is_empty() {
        ".".to_string()
    } else {
        relative
    }
}

pub fn json_string(value: &str) -> String {
    let escaped: String = value
        .chars()
        .flat_map(|ch| match ch {
            '\\' => vec!['\\', '\\'],
            '"' => vec!['\\', '"'],
            '\n' => vec!['\\', 'n'],
            '\r' => vec!['\\', 'r'],
            '\t' => vec!['\\', 't'],
            '\u{0008}' => vec!['\\', 'b'],
            '\u{000c}' => vec!['\\', 'f'],
            c if c.is_control() => {
                // Escape as \uXXXX for control characters
                format!("\\u{:04x}", c as u32).chars().collect::<Vec<_>>()
            }
            other => vec![other],
        })
        .collect();
    format!("\"{escaped}\"")
}

fn json_string_array(values: &[String]) -> String {
    let quoted = values
        .iter()
        .map(|value| json_string(value))
        .collect::<Vec<_>>();
    format!("[{}]", quoted.join(","))
}

fn json_search_matches(workspace_root: &Path, matches: &[SearchMatch]) -> String {
    let entries = matches
        .iter()
        .map(|item| {
            format!(
                "{{\"path\":{},\"line_number\":{},\"line\":{}}}",
                json_string(&relative_display(workspace_root, &item.path)),
                item.line_number,
                json_string(&item.line)
            )
        })
        .collect::<Vec<_>>();
    format!("[{}]", entries.join(","))
}

fn json_optional_usize(value: Option<usize>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn text_tail(value: &str, max_chars: usize) -> String {
    if value.len() <= max_chars {
        return value.to_string();
    }
    let start = value
        .char_indices()
        .rev()
        .scan(0usize, |count, (index, ch)| {
            *count += ch.len_utf8();
            Some((index, *count))
        })
        .find(|(_, count)| *count >= max_chars)
        .map(|(index, _)| index)
        .unwrap_or(0);
    format!("...{}", &value[start..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn preview_executes_read_only_tools() {
        let root = temp_root("tool-exec-preview");
        fs::write(root.join("README.md"), "ResearchCode\nneedle\n").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn needle() {}\n").unwrap();

        let file = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_1".to_string(),
            tool_id: "file.read".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some("README.md".to_string()),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(file.preview.contains("README.md"));

        let search = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_2".to_string(),
            tool_id: "search.ripgrep".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                pattern: Some("needle".to_string()),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(search.preview.contains("matches"));

        let repo_map = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_3".to_string(),
            tool_id: "repo.map".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs::default(),
        })
        .unwrap();
        assert!(repo_map.preview.contains("repo map files="));

        let list_directory = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_4".to_string(),
            tool_id: "file.list_directory".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some(".".to_string()),
                max_results: Some(32),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(list_directory.preview.contains("listed"));

        let list_tree = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_5".to_string(),
            tool_id: "file.list_tree".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some(".".to_string()),
                max_depth: Some(2),
                max_results: Some(32),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(list_tree.preview.contains("tree lines="));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn search_preview_uses_path_file_target_and_reports_detail_json() {
        let root = temp_root("tool-exec-search-path");
        fs::write(root.join("README.md"), "needle in readme\n").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "needle in lib\n").unwrap();

        let result = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_search_path".to_string(),
            tool_id: "search.ripgrep".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some("README.md".to_string()),
                root: Some("src".to_string()),
                pattern: Some("needle".to_string()),
                max_results: Some(10),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();

        assert!(result.ok);
        assert!(result.preview.contains("1 matches in 1 files"));
        assert!(result.detail_json.contains("\"path\":\"README.md\""));
        assert!(result.detail_json.contains("\"pattern\":\"needle\""));
        assert!(result.detail_json.contains("\"searched_files\":1"));
        assert!(result.detail_json.contains("\"truncated\":false"));
        assert!(result.detail_json.contains("\"matches\":["));
        assert!(result.detail_json.contains("\"line_number\":1"));
        assert!(!result.detail_json.contains("src/lib.rs"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn search_preview_returns_structured_model_errors() {
        let root = temp_root("tool-exec-search-errors");

        let empty_pattern = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_search_empty".to_string(),
            tool_id: "search.ripgrep".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some(".".to_string()),
                pattern: Some("  ".to_string()),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(!empty_pattern.ok);
        assert!(empty_pattern
            .detail_json
            .contains("\"error_code\":\"empty_pattern\""));
        assert!(empty_pattern.detail_json.contains("\"matches\":[]"));
        assert!(empty_pattern.detail_json.contains("\"searched_files\":0"));

        let missing = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_search_missing".to_string(),
            tool_id: "search.ripgrep".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some("missing.txt".to_string()),
                pattern: Some("needle".to_string()),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(!missing.ok);
        assert!(missing
            .detail_json
            .contains("\"error_code\":\"path_not_found\""));
        assert!(missing.detail_json.contains("\"path\":\"missing.txt\""));
        assert!(missing.detail_json.contains("\"pattern\":\"needle\""));

        let outside = root.parent().unwrap().join(format!(
            "researchcode-search-outside-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&outside, "needle outside\n").unwrap();
        let escape = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_search_escape".to_string(),
            tool_id: "search.ripgrep".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some(outside.to_string_lossy().to_string()),
                pattern: Some("needle".to_string()),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(!escape.ok);
        assert!(escape
            .detail_json
            .contains("\"error_code\":\"path_escapes_workspace\""));
        assert!(escape.detail_json.contains("\"truncated\":false"));
        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preview_repairs_model_absolute_workspace_paths() {
        let root = temp_root("tool-exec-model-absolute");
        fs::write(root.join("README.md"), "ResearchCode\n").unwrap();
        fs::create_dir_all(root.join("crates/runtime/src")).unwrap();
        fs::write(root.join("crates/runtime/src/lib.rs"), "pub mod loop_v3;\n").unwrap();

        let file = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_abs_file".to_string(),
            tool_id: "file.read".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some("/README.md".to_string()),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(file.ok);
        assert!(file.preview.contains("README.md"));

        let nested = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_abs_nested".to_string(),
            tool_id: "file.read".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some("/crates/runtime/src/lib.rs".to_string()),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(nested.ok);
        assert!(nested.preview.contains("crates/runtime/src/lib.rs"));

        let repo_map = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_abs_root".to_string(),
            tool_id: "repo.map".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                root: Some("/".to_string()),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(repo_map.ok);
        assert!(repo_map.preview.contains("repo map files="));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preview_still_blocks_real_absolute_escape_after_repair_attempt() {
        let root = temp_root("tool-exec-model-absolute-escape");
        fs::write(root.join("README.md"), "ResearchCode\n").unwrap();
        let outside = root.parent().unwrap().join(format!(
            "researchcode-outside-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&outside, "outside\n").unwrap();
        let result = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_abs_escape".to_string(),
            tool_id: "file.read".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some(outside.to_string_lossy().to_string()),
                ..ToolExecutionArgs::default()
            },
        });
        assert!(matches!(
            result,
            Err(ToolExecutionError::PathEscapesWorkspace(_))
        ));
        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_read_directory_error_suggests_directory_tool() {
        let root = temp_root("tool-exec-directory-suggest");
        fs::create_dir_all(root.join("src")).unwrap();
        let result = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_dir_read".to_string(),
            tool_id: "file.read".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some("src".to_string()),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(!result.ok);
        assert!(result
            .detail_json
            .contains("\"error_code\":\"path_is_directory\""));
        assert!(result
            .detail_json
            .contains("\"suggested_tool\":\"file.list_directory\""));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_read_preview_uses_numbered_formatter() {
        let root = temp_root("tool-exec-read-formatted-preview");
        fs::write(root.join("plan.md"), "one\ntwo\nthree\n").unwrap();
        let result = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_read_format".to_string(),
            tool_id: "file.read".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some("plan.md".to_string()),
                offset: Some(1),
                limit: Some(1),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(result.preview.contains("file.read · plan.md · lines 2-2/3"));
        assert!(result.preview.contains("   2  two"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_edit_preview_uses_diff_formatter() {
        let root = temp_root("tool-exec-edit-formatted-preview");
        let path = root.join("src.txt");
        fs::write(&path, "alpha\nbeta\n").unwrap();
        let base_hash = stable_text_hash("alpha\nbeta\n");
        let result = execute_tool(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_edit_format".to_string(),
            tool_id: "file.edit".to_string(),
            mode: ToolExecutionMode::ApplyWithPermission {
                permission_decision: Some(PermissionDecisionKind::AllowOnce),
            },
            args: ToolExecutionArgs {
                path: Some("src.txt".to_string()),
                old_string: Some("beta".to_string()),
                new_string: Some("gamma".to_string()),
                base_hash: Some(base_hash),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(result
            .preview
            .contains("file.edit · src.txt · 1 replacement"));
        assert!(result.preview.contains("- beta"));
        assert!(result.preview.contains("+ gamma"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn shell_command_preview_uses_command_formatter() {
        let root = temp_root("tool-exec-shell-formatted-preview");
        let result = execute_tool(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_shell_format".to_string(),
            tool_id: "shell.command".to_string(),
            mode: ToolExecutionMode::ApplyWithPermission {
                permission_decision: Some(PermissionDecisionKind::AllowOnce),
            },
            args: ToolExecutionArgs {
                command: Some("printf hello".to_string()),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(result
            .preview
            .contains("shell.command · `printf hello` · exit 0"));
        assert!(result.preview.contains("stdout (last 80 lines):"));
        assert!(result.preview.contains("hello"));
        assert!(result.preview.contains("stderr: (empty)"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_read_offset_past_eof_returns_structured_range_error() {
        let root = temp_root("tool-exec-read-eof-range");
        fs::write(root.join("plan.md"), "one\ntwo\nthree\n").unwrap();
        let result = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_read_eof".to_string(),
            tool_id: "file.read".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some("plan.md".to_string()),
                offset: Some(3),
                limit: Some(10),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(!result.ok);
        assert!(result
            .detail_json
            .contains("\"error_code\":\"READ_RANGE_EMPTY_OR_EOF\""));
        assert!(result.detail_json.contains("\"line_count\":3"));
        assert!(result.preview.contains("valid_range=offset 0..2"));
        assert!(!result.preview.contains("lines=4..3"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_read_zero_limit_returns_structured_range_error() {
        let root = temp_root("tool-exec-read-zero-limit");
        fs::write(root.join("plan.md"), "one\ntwo\n").unwrap();
        let result = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_read_zero".to_string(),
            tool_id: "file.read".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some("plan.md".to_string()),
                offset: Some(0),
                limit: Some(0),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(!result.ok);
        assert!(result
            .detail_json
            .contains("\"error_code\":\"READ_RANGE_EMPTY_OR_EOF\""));
        assert!(result.detail_json.contains("\"requested_limit\":0"));
        assert!(!result.preview.contains("lines=1..0"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preview_rejects_permission_required_and_sensitive_paths() {
        let root = temp_root("tool-exec-deny");
        fs::write(root.join(".env"), "SECRET=1\n").unwrap();
        let shell = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_1".to_string(),
            tool_id: "shell.command".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs::default(),
        });
        assert_eq!(
            shell,
            Err(ToolExecutionError::PermissionRequired(
                "shell.command".to_string()
            ))
        );
        let sensitive = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_2".to_string(),
            tool_id: "file.read".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some(".env".to_string()),
                ..ToolExecutionArgs::default()
            },
        });
        assert!(matches!(
            sensitive,
            Err(ToolExecutionError::SensitivePath(_))
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preview_blocks_path_escape() {
        let root = temp_root("tool-exec-escape");
        let outside = root.parent().unwrap().join(format!(
            "researchcode-outside-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&outside, "outside\n").unwrap();
        let result = execute_tool_preview(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_1".to_string(),
            tool_id: "file.read".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                path: Some(outside.to_string_lossy().to_string()),
                ..ToolExecutionArgs::default()
            },
        });
        assert!(matches!(
            result,
            Err(ToolExecutionError::PathEscapesWorkspace(_))
        ));
        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_mode_executes_shell_only_with_permission() {
        let root = temp_root("tool-exec-shell");
        let denied = execute_tool(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_1".to_string(),
            tool_id: "shell.command".to_string(),
            mode: ToolExecutionMode::ApplyWithPermission {
                permission_decision: None,
            },
            args: ToolExecutionArgs {
                command: Some("npm install lodash".to_string()),
                ..ToolExecutionArgs::default()
            },
        });
        assert_eq!(
            denied,
            Err(ToolExecutionError::PermissionRequired(
                "shell.command".to_string()
            ))
        );
        let allowed = execute_tool(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_2".to_string(),
            tool_id: "shell.command".to_string(),
            mode: ToolExecutionMode::ApplyWithPermission {
                permission_decision: Some(PermissionDecisionKind::AllowOnce),
            },
            args: ToolExecutionArgs {
                command: Some("find . -maxdepth 0".to_string()),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(allowed.ok);
        assert!(allowed
            .preview
            .contains("shell.command · `find . -maxdepth 0` · exit 0"));
        assert!(allowed.detail_json.contains("classifier_decision"));
        assert!(allowed.detail_json.contains("stdout_tail"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_mode_allows_permissioned_sensitive_write() {
        let root = temp_root("tool-exec-sensitive-write");
        let denied = execute_tool(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_sensitive_denied".to_string(),
            tool_id: "file.write".to_string(),
            mode: ToolExecutionMode::ApplyWithPermission {
                permission_decision: None,
            },
            args: ToolExecutionArgs {
                path: Some(".env".to_string()),
                content: Some("TOKEN=old\n".to_string()),
                ..ToolExecutionArgs::default()
            },
        });
        assert_eq!(
            denied,
            Err(ToolExecutionError::PermissionRequired(
                "file.write".to_string()
            ))
        );

        let allowed = execute_tool(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_sensitive_allowed".to_string(),
            tool_id: "file.write".to_string(),
            mode: ToolExecutionMode::ApplyWithPermission {
                permission_decision: Some(PermissionDecisionKind::AllowOnce),
            },
            args: ToolExecutionArgs {
                path: Some(".env".to_string()),
                content: Some("TOKEN=redacted\n".to_string()),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(allowed.ok);
        assert_eq!(
            fs::read_to_string(root.join(".env")).unwrap(),
            "TOKEN=redacted\n"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn permission_gate_contract_allows_bypass_write_without_legacy_decision() {
        let root = temp_root("tool-exec-gate-allow-write");
        let mut gate = test_permission_gate(
            crate::agent_kernel::PermissionMode::BypassPermissions,
            &root,
        );
        let result = execute_tool_with_permission_gate(
            &ToolExecutionRequest {
                workspace_root: root.clone(),
                tool_call_id: "tool_gate_write".to_string(),
                tool_id: "file.write".to_string(),
                mode: ToolExecutionMode::ApplyWithPermission {
                    permission_decision: None,
                },
                args: ToolExecutionArgs {
                    path: Some("notes.txt".to_string()),
                    content: Some("hello\n".to_string()),
                    ..ToolExecutionArgs::default()
                },
            },
            &mut gate,
        )
        .unwrap();
        assert!(result.ok);
        assert_eq!(
            fs::read_to_string(root.join("notes.txt")).unwrap(),
            "hello\n"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn permission_gate_contract_default_mode_asks_before_write() {
        let root = temp_root("tool-exec-gate-ask-write");
        let mut gate = test_permission_gate(crate::agent_kernel::PermissionMode::Default, &root);
        let result = execute_tool_with_permission_gate(
            &ToolExecutionRequest {
                workspace_root: root.clone(),
                tool_call_id: "tool_gate_write_ask".to_string(),
                tool_id: "file.write".to_string(),
                mode: ToolExecutionMode::ApplyWithPermission {
                    permission_decision: None,
                },
                args: ToolExecutionArgs {
                    path: Some("notes.txt".to_string()),
                    content: Some("hello\n".to_string()),
                    ..ToolExecutionArgs::default()
                },
            },
            &mut gate,
        );
        assert_eq!(
            result,
            Err(ToolExecutionError::PermissionRequired(
                "file.write".to_string()
            ))
        );
        assert!(!root.join("notes.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn permission_gate_contract_blocks_sensitive_write_even_in_bypass() {
        let root = temp_root("tool-exec-gate-sensitive-write");
        let mut gate = test_permission_gate(
            crate::agent_kernel::PermissionMode::BypassPermissions,
            &root,
        );
        let result = execute_tool_with_permission_gate(
            &ToolExecutionRequest {
                workspace_root: root.clone(),
                tool_call_id: "tool_gate_sensitive".to_string(),
                tool_id: "file.write".to_string(),
                mode: ToolExecutionMode::ApplyWithPermission {
                    permission_decision: None,
                },
                args: ToolExecutionArgs {
                    path: Some(".env".to_string()),
                    content: Some("TOKEN=1\n".to_string()),
                    ..ToolExecutionArgs::default()
                },
            },
            &mut gate,
        );
        assert_eq!(
            result,
            Err(ToolExecutionError::PermissionRequired(
                "file.write".to_string()
            ))
        );
        assert!(!root.join(".env").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn permission_gate_contract_preserves_explicit_permission_decision() {
        let root = temp_root("tool-exec-gate-explicit-decision");
        let mut gate = test_permission_gate(crate::agent_kernel::PermissionMode::Default, &root);
        let result = execute_tool_with_permission_gate(
            &ToolExecutionRequest {
                workspace_root: root.clone(),
                tool_call_id: "tool_gate_shell".to_string(),
                tool_id: "shell.command".to_string(),
                mode: ToolExecutionMode::ApplyWithPermission {
                    permission_decision: Some(PermissionDecisionKind::AllowOnce),
                },
                args: ToolExecutionArgs {
                    command: Some("find . -maxdepth 0".to_string()),
                    ..ToolExecutionArgs::default()
                },
            },
            &mut gate,
        )
        .unwrap();
        assert!(result.ok);
        assert!(result.preview.contains("shell.command"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn file_write_preserves_existing_line_endings_and_bom() {
        let root = temp_root("tool-exec-file-write-style");
        let path = root.join("notes.txt");
        fs::write(&path, b"\xEF\xBB\xBFalpha\r\nbeta\r\n").unwrap();
        let base_hash = stable_text_hash("alpha\r\nbeta\r\n");
        let result = execute_tool(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_write_style".to_string(),
            tool_id: "file.write".to_string(),
            mode: ToolExecutionMode::ApplyWithPermission {
                permission_decision: Some(PermissionDecisionKind::AllowOnce),
            },
            args: ToolExecutionArgs {
                path: Some("notes.txt".to_string()),
                content: Some("gamma\ndelta\n".to_string()),
                base_hash: Some(base_hash),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(result.ok);
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"\xEF\xBB\xBF"));
        let text = String::from_utf8_lossy(&bytes[3..]).to_string();
        assert_eq!(text, "gamma\r\ndelta\r\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_mode_executes_patch_with_base_hash_and_permission() {
        let root = temp_root("tool-exec-patch");
        fs::create_dir_all(root.join("src")).unwrap();
        let path = root.join("src/lib.rs");
        fs::write(&path, "pub const RETRY: u8 = 3;\n").unwrap();
        let base_hash = stable_text_hash("pub const RETRY: u8 = 3;\n");
        let result = execute_tool(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_1".to_string(),
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
        .unwrap();
        assert!(result.preview.contains("patch applied"));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "pub const RETRY: u8 = 5;\n"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_mode_rejects_stale_patch_before_write() {
        let root = temp_root("tool-exec-stale-patch");
        fs::create_dir_all(root.join("src")).unwrap();
        let path = root.join("src/lib.rs");
        fs::write(&path, "pub const RETRY: u8 = 3;\n").unwrap();
        let result = execute_tool(&ToolExecutionRequest {
            workspace_root: root.clone(),
            tool_call_id: "tool_1".to_string(),
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
        assert_eq!(
            result,
            Err(ToolExecutionError::ValidationFailed(
                "FailStale".to_string()
            ))
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "pub const RETRY: u8 = 3;\n"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preview_executes_research_csv_profile_with_local_sidecar_policy() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .unwrap()
            .to_path_buf();
        let output_dir = std::env::temp_dir().join(format!(
            "researchcode-tool-exec-research-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let result = execute_tool(&ToolExecutionRequest {
            workspace_root: workspace_root.clone(),
            tool_call_id: "tool_research_1".to_string(),
            tool_id: "research.csv_profile".to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args: ToolExecutionArgs {
                input_csv: Some("eval/fixtures/research/csv-quality-small/input.csv".to_string()),
                job_id: Some("tool_exec_research_test".to_string()),
                output_dir: Some(output_dir.to_string_lossy().to_string()),
                ..ToolExecutionArgs::default()
            },
        })
        .unwrap();
        assert!(result.ok);
        assert_eq!(result.exit_code, Some(0));
        assert!(result.preview.contains("artifacts=5"));
        assert!(result.detail_json.contains("manifest_hash"));
        let _ = fs::remove_dir_all(output_dir);
    }

    fn test_permission_gate(
        mode: crate::agent_kernel::PermissionMode,
        root: &Path,
    ) -> PermissionGate {
        PermissionGate::new(
            Arc::new(crate::permission_policy::PermissionRuleStore::new(
                root.join("permissions.tsv"),
            )),
            crate::permission_policy::PermissionRuleSet::default(),
            mode,
            root.to_string_lossy(),
            "tool-execution-test",
        )
    }

    fn temp_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-{label}-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
