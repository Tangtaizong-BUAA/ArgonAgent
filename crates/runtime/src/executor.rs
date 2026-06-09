//! Agent executor skeletons.
//!
//! This module intentionally starts with a deterministic no-model fixture. It
//! fixes the orchestration boundary before live DeepSeek/Qwen calls are wired:
//! context retrieval, patch validation, permission, command execution,
//! artifacts, review, and event export all flow through AgentSession.

use crate::artifact::ArtifactStore;
use crate::command::{
    capture_command_output_artifact, prepare_command, run_prepared_command, CommandRequest,
};
use crate::file_tool::{read_file, FileReadRequest};
use crate::model_adapter::{
    DeepSeekNativeAdapter, ModelAdapter, ModelAdapterRequest, ModelRole, QwenNativeAdapter,
};
use crate::native_response_normalizer::{
    normalize_deepseek_anthropic_response, normalize_qwen_openai_response,
};
use crate::parser::{classify_deepseek_output, classify_qwen_output, ParserAction};
use crate::patch::{
    apply_replace_patch, stable_text_hash, validate_patch, PatchCheck, PatchValidation,
    ReplacePatch,
};
use crate::provider_response_adapter::{
    record_native_provider_response, record_native_provider_stream, NativeProviderResponseInput,
    NativeProviderStreamInput, NativeProviderStreamKind,
};
use crate::search_tool::{search_text, SearchRequest};
use crate::session::AgentSession;
use crate::state::AgentState;
use crate::tool_result::{json_string, write_tool_result_artifact, ToolResultRecord};
use researchcode_kernel::model::{NativeModelFamily, NativeModelProfile, OptimizationLevel};
use researchcode_kernel::{PermissionDecisionKind, PermissionRequestType};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoModelCodingFixtureConfig {
    pub project_id: String,
    pub session_id: String,
    pub task_id: String,
    pub cleanup: bool,
}

impl Default for NoModelCodingFixtureConfig {
    fn default() -> Self {
        Self {
            project_id: "proj".to_string(),
            session_id: "sess".to_string(),
            task_id: "task".to_string(),
            cleanup: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoModelCodingFixtureResult {
    pub final_state: AgentState,
    pub event_count: usize,
    pub matches_count: usize,
    pub command_artifact_hash: String,
    pub event_jsonl: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureRepairFixtureResult {
    pub final_state: AgentState,
    pub event_count: usize,
    pub first_exit_code: i32,
    pub repaired_exit_code: i32,
    pub event_jsonl: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedModelPlannedFixtureResult {
    pub final_state: AgentState,
    pub event_count: usize,
    pub deepseek_tool_id: String,
    pub qwen_tool_id: String,
    pub qwen_mismatch_action: ParserAction,
    pub event_jsonl: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedPatchFixtureResult {
    pub final_state: AgentState,
    pub event_count: usize,
    pub qwen_stale_validation: PatchValidation,
    pub qwen_ambiguous_validation: PatchValidation,
    pub deepseek_patch_validation: PatchValidation,
    pub event_jsonl: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedLiveResponseFixtureResult {
    pub final_state: AgentState,
    pub event_count: usize,
    pub deepseek_transcript_hash: String,
    pub qwen_transcript_hash: String,
    pub event_jsonl: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedNonStreamResponseFixtureResult {
    pub final_state: AgentState,
    pub event_count: usize,
    pub deepseek_transcript_hash: String,
    pub qwen_transcript_hash: String,
    pub event_jsonl: String,
}

pub fn run_no_model_coding_fixture(
    config: &NoModelCodingFixtureConfig,
) -> Result<NoModelCodingFixtureResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-coding-fixture-{nonce}"));
    let src = root.join("src");
    fs::create_dir_all(&src).map_err(|error| error.to_string())?;
    let target = src.join("parser.rs");
    fs::write(&target, "retry_count = 3\n").map_err(|error| error.to_string())?;
    let artifact_store = ArtifactStore::new(root.join("artifacts"));

    let mut session = AgentSession::new(&config.project_id, &config.session_id, &config.task_id)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::RetrievingContext)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Executing)
        .map_err(|error| format!("{error:?}"))?;

    session
        .record_tool_call_requested("tool_read_1", "file.read")
        .map_err(|error| format!("{error:?}"))?;
    let read = read_file(
        &FileReadRequest {
            path: target.clone(),
            max_bytes: 4096,
        },
        &root,
    )
    .map_err(|error| format!("{error:?}"))?;
    session
        .record_tool_call_completed("tool_read_1", "file.read", true)
        .map_err(|error| format!("{error:?}"))?;
    let read_artifact = write_tool_result_artifact(
        &artifact_store,
        "tool_result_read_1",
        &ToolResultRecord::new(
            "tool_read_1",
            "file.read",
            true,
            format!(
                "read {} bytes from {}",
                read.size_bytes,
                read.path.display()
            ),
            format!(
                "{{\"path\":{},\"size_bytes\":{},\"truncated\":{}}}",
                json_string(&read.path.to_string_lossy()),
                read.size_bytes,
                read.truncated
            ),
        ),
    )
    .map_err(|error| error.to_string())?;
    session
        .record_tool_result_artifact(
            "tool_read_1",
            "file.read",
            read_artifact.artifact_id,
            read_artifact.content_hash,
            "read file result",
        )
        .map_err(|error| format!("{error:?}"))?;

    session
        .record_tool_call_requested("tool_search_1", "search.ripgrep")
        .map_err(|error| format!("{error:?}"))?;
    let matches = search_text(
        &SearchRequest {
            root: root.clone(),
            pattern: "retry_count".to_string(),
            max_results: 20,
        },
        &root,
    )
    .map_err(|error| format!("{error:?}"))?;
    session
        .record_tool_call_completed("tool_search_1", "search.ripgrep", true)
        .map_err(|error| format!("{error:?}"))?;
    let search_artifact = write_tool_result_artifact(
        &artifact_store,
        "tool_result_search_1",
        &ToolResultRecord::new(
            "tool_search_1",
            "search.ripgrep",
            true,
            format!("{} matches for retry_count", matches.len()),
            format!("{{\"match_count\":{}}}", matches.len()),
        ),
    )
    .map_err(|error| error.to_string())?;
    session
        .record_tool_result_artifact(
            "tool_search_1",
            "search.ripgrep",
            search_artifact.artifact_id,
            search_artifact.content_hash,
            "search result",
        )
        .map_err(|error| format!("{error:?}"))?;

    session
        .record_tool_call_requested("tool_patch_1", "patch.apply")
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_patch_proposal_created("patch_1", target.to_string_lossy())
        .map_err(|error| format!("{error:?}"))?;
    let base_hash = stable_text_hash(&read.content);
    let validation = validate_patch(PatchCheck {
        path: &target.to_string_lossy(),
        current_text: Some(&read.content),
        current_hash: Some(&base_hash),
        old_string: "retry_count = 3",
        base_hash: &base_hash,
    });
    session
        .record_patch_proposal_validated("patch_1", validation.clone())
        .map_err(|error| format!("{error:?}"))?;
    if validation != PatchValidation::Pass {
        return Err(format!("patch validation did not pass: {validation:?}"));
    }
    session
        .request_permission("perm_patch_1", PermissionRequestType::FileWrite, None)
        .map_err(|error| format!("{error:?}"))?;
    session
        .decide_permission(PermissionDecisionKind::AllowOnce)
        .map_err(|error| format!("{error:?}"))?;
    let applied_validation = apply_replace_patch(&ReplacePatch {
        path: target.clone(),
        old_string: "retry_count = 3".to_string(),
        new_string: "retry_count = 5".to_string(),
        base_hash,
    })
    .map_err(|error| format!("{error:?}"))?;
    if applied_validation != PatchValidation::Pass {
        return Err(format!(
            "patch apply validation did not pass: {applied_validation:?}"
        ));
    }
    session
        .record_patch_applied("patch_1", target.to_string_lossy())
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_tool_call_completed("tool_patch_1", "patch.apply", true)
        .map_err(|error| format!("{error:?}"))?;
    let patch_artifact = write_tool_result_artifact(
        &artifact_store,
        "tool_result_patch_1",
        &ToolResultRecord::new(
            "tool_patch_1",
            "patch.apply",
            true,
            format!("applied patch to {}", target.display()),
            format!(
                "{{\"path\":{},\"validation\":\"pass\"}}",
                json_string(&target.to_string_lossy())
            ),
        ),
    )
    .map_err(|error| error.to_string())?;
    session
        .record_tool_result_artifact(
            "tool_patch_1",
            "patch.apply",
            patch_artifact.artifact_id,
            patch_artifact.content_hash,
            "patch result",
        )
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Executing)
        .map_err(|error| format!("{error:?}"))?;

    session
        .record_tool_call_requested("tool_command_1", "shell.command")
        .map_err(|error| format!("{error:?}"))?;
    session
        .request_permission("perm_command_1", PermissionRequestType::Command, None)
        .map_err(|error| format!("{error:?}"))?;
    session
        .decide_permission(PermissionDecisionKind::AllowOnce)
        .map_err(|error| format!("{error:?}"))?;
    let command_plan = prepare_command(CommandRequest {
        command: "find . -maxdepth 0".to_string(),
        cwd: root.to_string_lossy().to_string(),
    });
    let command_output =
        run_prepared_command(&command_plan, Some(PermissionDecisionKind::AllowOnce))
            .map_err(|error| format!("{error:?}"))?;
    let command_artifact =
        capture_command_output_artifact(&artifact_store, "cmd_fixture_1", &command_output)
            .map_err(|error| error.to_string())?;
    session
        .record_tool_call_completed(
            "tool_command_1",
            "shell.command",
            command_output.exit_code == 0,
        )
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_tool_result_artifact(
            "tool_command_1",
            "shell.command",
            command_artifact.artifact_id,
            command_artifact.content_hash.clone(),
            "command output",
        )
        .map_err(|error| format!("{error:?}"))?;

    session
        .start_review()
        .map_err(|error| format!("{error:?}"))?;
    session
        .complete_after_review()
        .map_err(|error| format!("{error:?}"))?;

    let final_text = fs::read_to_string(&target).map_err(|error| error.to_string())?;
    if final_text != "retry_count = 5\n" {
        return Err("fixture patch did not produce expected file".to_string());
    }
    let result = NoModelCodingFixtureResult {
        final_state: session.state(),
        event_count: session.event_count(),
        matches_count: matches.len(),
        command_artifact_hash: command_artifact.content_hash,
        event_jsonl: session.export_events_jsonl(),
    };
    if config.cleanup {
        let _ = fs::remove_dir_all(&root);
    }
    Ok(result)
}

pub fn run_failure_repair_fixture(
    config: &NoModelCodingFixtureConfig,
) -> Result<FailureRepairFixtureResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-failure-repair-{nonce}"));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let target = root.join("expected.txt");

    let mut session = AgentSession::new(&config.project_id, &config.session_id, &config.task_id)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::RetrievingContext)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Executing)
        .map_err(|error| format!("{error:?}"))?;

    session
        .record_tool_call_requested("tool_command_fail", "shell.command")
        .map_err(|error| format!("{error:?}"))?;
    session
        .request_permission("perm_command_fail", PermissionRequestType::Command, None)
        .map_err(|error| format!("{error:?}"))?;
    session
        .decide_permission(PermissionDecisionKind::AllowOnce)
        .map_err(|error| format!("{error:?}"))?;
    let failing_plan = prepare_command(CommandRequest {
        command: "find expected.txt -maxdepth 0".to_string(),
        cwd: root.to_string_lossy().to_string(),
    });
    let failing_output =
        run_prepared_command(&failing_plan, Some(PermissionDecisionKind::AllowOnce))
            .map_err(|error| format!("{error:?}"))?;
    session
        .record_tool_call_completed("tool_command_fail", "shell.command", false)
        .map_err(|error| format!("{error:?}"))?;
    session
        .diagnose_failure()
        .map_err(|error| format!("{error:?}"))?;
    session
        .resume_after_diagnosis()
        .map_err(|error| format!("{error:?}"))?;

    session
        .record_patch_proposal_created("patch_create_expected", target.to_string_lossy())
        .map_err(|error| format!("{error:?}"))?;
    let validation = validate_patch(PatchCheck {
        path: &target.to_string_lossy(),
        current_text: None,
        current_hash: None,
        old_string: "",
        base_hash: "",
    });
    session
        .record_patch_proposal_validated("patch_create_expected", validation.clone())
        .map_err(|error| format!("{error:?}"))?;
    if validation != PatchValidation::PassCreate {
        return Err(format!(
            "create patch validation did not pass: {validation:?}"
        ));
    }
    session
        .request_permission("perm_patch_create", PermissionRequestType::FileWrite, None)
        .map_err(|error| format!("{error:?}"))?;
    session
        .decide_permission(PermissionDecisionKind::AllowOnce)
        .map_err(|error| format!("{error:?}"))?;
    let applied_validation = apply_replace_patch(&ReplacePatch {
        path: target.clone(),
        old_string: "".to_string(),
        new_string: "ok\n".to_string(),
        base_hash: "".to_string(),
    })
    .map_err(|error| format!("{error:?}"))?;
    if applied_validation != PatchValidation::PassCreate {
        return Err(format!(
            "create patch apply validation did not pass: {applied_validation:?}"
        ));
    }
    session
        .record_patch_applied("patch_create_expected", target.to_string_lossy())
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Executing)
        .map_err(|error| format!("{error:?}"))?;

    session
        .record_tool_call_requested("tool_command_repaired", "shell.command")
        .map_err(|error| format!("{error:?}"))?;
    session
        .request_permission(
            "perm_command_repaired",
            PermissionRequestType::Command,
            None,
        )
        .map_err(|error| format!("{error:?}"))?;
    session
        .decide_permission(PermissionDecisionKind::AllowOnce)
        .map_err(|error| format!("{error:?}"))?;
    let repaired_output =
        run_prepared_command(&failing_plan, Some(PermissionDecisionKind::AllowOnce))
            .map_err(|error| format!("{error:?}"))?;
    session
        .record_tool_call_completed(
            "tool_command_repaired",
            "shell.command",
            repaired_output.exit_code == 0,
        )
        .map_err(|error| format!("{error:?}"))?;
    session
        .start_review()
        .map_err(|error| format!("{error:?}"))?;
    session
        .complete_after_review()
        .map_err(|error| format!("{error:?}"))?;

    let result = FailureRepairFixtureResult {
        final_state: session.state(),
        event_count: session.event_count(),
        first_exit_code: failing_output.exit_code,
        repaired_exit_code: repaired_output.exit_code,
        event_jsonl: session.export_events_jsonl(),
    };
    if config.cleanup {
        let _ = fs::remove_dir_all(&root);
    }
    Ok(result)
}

pub fn run_recorded_model_planned_fixture(
    config: &NoModelCodingFixtureConfig,
) -> Result<RecordedModelPlannedFixtureResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-recorded-model-fixture-{nonce}"));
    let src = root.join("src");
    fs::create_dir_all(&src).map_err(|error| error.to_string())?;
    let target = src.join("parser.ts");
    fs::write(&target, "export const retry_count = 3;\n").map_err(|error| error.to_string())?;
    let artifact_store = ArtifactStore::new(root.join("artifacts"));

    let mut session = AgentSession::new(&config.project_id, &config.session_id, &config.task_id)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::RetrievingContext)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Executing)
        .map_err(|error| format!("{error:?}"))?;

    let deepseek_raw =
        r#"{"tool_calls":[{"name":"file.read","arguments":{"path":"src/parser.ts"}}]}"#;
    let deepseek = classify_deepseek_output(deepseek_raw);
    if deepseek.action != ParserAction::Execute || deepseek.tool_id.as_deref() != Some("file.read")
    {
        return Err(format!("unexpected DeepSeek parser result: {deepseek:?}"));
    }
    session
        .record_tool_call_requested("ds_tool_read_1", "file.read")
        .map_err(|error| format!("{error:?}"))?;
    let ds_read = read_file(
        &FileReadRequest {
            path: target.clone(),
            max_bytes: 4096,
        },
        &root,
    )
    .map_err(|error| format!("{error:?}"))?;
    session
        .record_tool_call_completed("ds_tool_read_1", "file.read", true)
        .map_err(|error| format!("{error:?}"))?;
    let ds_artifact = write_tool_result_artifact(
        &artifact_store,
        "tool_result_ds_read_1",
        &ToolResultRecord::new(
            "ds_tool_read_1",
            "file.read",
            true,
            "DeepSeek planned file.read",
            format!(
                "{{\"source\":\"deepseek_parser_fixture\",\"path\":{},\"size_bytes\":{}}}",
                json_string(&ds_read.path.to_string_lossy()),
                ds_read.size_bytes
            ),
        ),
    )
    .map_err(|error| error.to_string())?;
    session
        .record_tool_result_artifact(
            "ds_tool_read_1",
            "file.read",
            ds_artifact.artifact_id,
            ds_artifact.content_hash,
            "DeepSeek planned file read",
        )
        .map_err(|error| format!("{error:?}"))?;

    let qwen_raw = r#"{"reasoning":"Need exact file.","tool_calls":[{"name":"file.read","arguments":{"path":"src/parser.ts"}}]}"#;
    let qwen = classify_qwen_output(qwen_raw);
    if qwen.action != ParserAction::Execute || qwen.tool_id.as_deref() != Some("file.read") {
        return Err(format!("unexpected Qwen parser result: {qwen:?}"));
    }
    session
        .record_tool_call_requested("qw_tool_read_1", "file.read")
        .map_err(|error| format!("{error:?}"))?;
    let qw_read = read_file(
        &FileReadRequest {
            path: target.clone(),
            max_bytes: 4096,
        },
        &root,
    )
    .map_err(|error| format!("{error:?}"))?;
    session
        .record_tool_call_completed("qw_tool_read_1", "file.read", true)
        .map_err(|error| format!("{error:?}"))?;
    let qw_artifact = write_tool_result_artifact(
        &artifact_store,
        "tool_result_qw_read_1",
        &ToolResultRecord::new(
            "qw_tool_read_1",
            "file.read",
            true,
            "Qwen planned file.read",
            format!(
                "{{\"source\":\"qwen_parser_fixture\",\"path\":{},\"size_bytes\":{}}}",
                json_string(&qw_read.path.to_string_lossy()),
                qw_read.size_bytes
            ),
        ),
    )
    .map_err(|error| error.to_string())?;
    session
        .record_tool_result_artifact(
            "qw_tool_read_1",
            "file.read",
            qw_artifact.artifact_id,
            qw_artifact.content_hash,
            "Qwen planned file read",
        )
        .map_err(|error| format!("{error:?}"))?;

    let qwen_mismatch_raw = r#"{"deployment":{"model":"Qwen2-7B"},"tool_calls":[{"name":"file.read","arguments":{"path":"src/parser.ts"}}]}"#;
    let qwen_mismatch = classify_qwen_output(qwen_mismatch_raw);
    if qwen_mismatch.action != ParserAction::BlockNativeSession {
        return Err(format!(
            "Qwen native mismatch did not block: {qwen_mismatch:?}"
        ));
    }

    session
        .start_review()
        .map_err(|error| format!("{error:?}"))?;
    session
        .complete_after_review()
        .map_err(|error| format!("{error:?}"))?;

    let result = RecordedModelPlannedFixtureResult {
        final_state: session.state(),
        event_count: session.event_count(),
        deepseek_tool_id: deepseek.tool_id.unwrap_or_default(),
        qwen_tool_id: qwen.tool_id.unwrap_or_default(),
        qwen_mismatch_action: qwen_mismatch.action,
        event_jsonl: session.export_events_jsonl(),
    };
    if config.cleanup {
        let _ = fs::remove_dir_all(&root);
    }
    Ok(result)
}

pub fn run_recorded_patch_fixture(
    config: &NoModelCodingFixtureConfig,
) -> Result<RecordedPatchFixtureResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-recorded-patch-fixture-{nonce}"));
    let src = root.join("src");
    fs::create_dir_all(&src).map_err(|error| error.to_string())?;
    let target = src.join("parser.ts");
    fs::write(&target, "old\nhelper()\nhelper()\n").map_err(|error| error.to_string())?;
    let artifact_store = ArtifactStore::new(root.join("artifacts"));

    let mut session = AgentSession::new(&config.project_id, &config.session_id, &config.task_id)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::RetrievingContext)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Executing)
        .map_err(|error| format!("{error:?}"))?;

    let current_text = fs::read_to_string(&target).map_err(|error| error.to_string())?;
    let current_hash = stable_text_hash(&current_text);

    let qwen_stale_raw = r#"{"tool_calls":[{"name":"patch.propose","arguments":{"path":"src/parser.ts","old_string":"old","new_string":"new"}}]}"#;
    let qwen_stale = classify_qwen_output(qwen_stale_raw);
    if qwen_stale.action != ParserAction::ExecuteOnlyAfterFileReadHash {
        return Err(format!(
            "unexpected Qwen stale parser action: {qwen_stale:?}"
        ));
    }
    session
        .record_tool_call_requested("qwen_patch_stale", "patch.apply")
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_patch_proposal_created("patch_qwen_stale", target.to_string_lossy())
        .map_err(|error| format!("{error:?}"))?;
    let qwen_stale_validation = validate_patch(PatchCheck {
        path: &target.to_string_lossy(),
        current_text: Some(&current_text),
        current_hash: Some(&current_hash),
        old_string: "old",
        base_hash: "fnv64_stale",
    });
    session
        .record_patch_proposal_validated("patch_qwen_stale", qwen_stale_validation.clone())
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_tool_call_completed("qwen_patch_stale", "patch.apply", false)
        .map_err(|error| format!("{error:?}"))?;

    let qwen_ambiguous_raw = r#"{"tool_calls":[{"name":"patch.propose","arguments":{"path":"src/parser.ts","old_string":"helper()","new_string":"helper2()"}}]}"#;
    let qwen_ambiguous = classify_qwen_output(qwen_ambiguous_raw);
    if qwen_ambiguous.action != ParserAction::PatchValidatorMustRejectAmbiguousMatch {
        return Err(format!(
            "unexpected Qwen ambiguous parser action: {qwen_ambiguous:?}"
        ));
    }
    session
        .record_tool_call_requested("qwen_patch_ambiguous", "patch.apply")
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_patch_proposal_created("patch_qwen_ambiguous", target.to_string_lossy())
        .map_err(|error| format!("{error:?}"))?;
    let qwen_ambiguous_validation = validate_patch(PatchCheck {
        path: &target.to_string_lossy(),
        current_text: Some(&current_text),
        current_hash: Some(&current_hash),
        old_string: "helper()",
        base_hash: &current_hash,
    });
    session
        .record_patch_proposal_validated("patch_qwen_ambiguous", qwen_ambiguous_validation.clone())
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_tool_call_completed("qwen_patch_ambiguous", "patch.apply", false)
        .map_err(|error| format!("{error:?}"))?;

    let deepseek_raw = r#"[TOOL_CALL]{"name":"patch.propose","arguments":{"path":"src/parser.ts","old_string":"old","new_string":"new"}}[/TOOL_CALL]"#;
    let deepseek = classify_deepseek_output(deepseek_raw);
    if deepseek.action != ParserAction::Execute
        || deepseek.tool_id.as_deref() != Some("patch.propose")
    {
        return Err(format!(
            "unexpected DeepSeek patch parser result: {deepseek:?}"
        ));
    }
    session
        .record_tool_call_requested("deepseek_patch_apply", "patch.apply")
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_patch_proposal_created("patch_deepseek_valid", target.to_string_lossy())
        .map_err(|error| format!("{error:?}"))?;
    let deepseek_patch_validation = validate_patch(PatchCheck {
        path: &target.to_string_lossy(),
        current_text: Some(&current_text),
        current_hash: Some(&current_hash),
        old_string: "old",
        base_hash: &current_hash,
    });
    session
        .record_patch_proposal_validated("patch_deepseek_valid", deepseek_patch_validation.clone())
        .map_err(|error| format!("{error:?}"))?;
    if deepseek_patch_validation != PatchValidation::Pass {
        return Err(format!(
            "DeepSeek valid patch failed validation: {deepseek_patch_validation:?}"
        ));
    }
    session
        .request_permission(
            "perm_deepseek_patch",
            PermissionRequestType::FileWrite,
            None,
        )
        .map_err(|error| format!("{error:?}"))?;
    session
        .decide_permission(PermissionDecisionKind::AllowOnce)
        .map_err(|error| format!("{error:?}"))?;
    let applied = apply_replace_patch(&ReplacePatch {
        path: target.clone(),
        old_string: "old".to_string(),
        new_string: "new".to_string(),
        base_hash: current_hash,
    })
    .map_err(|error| format!("{error:?}"))?;
    if applied != PatchValidation::Pass {
        return Err(format!("DeepSeek patch apply failed: {applied:?}"));
    }
    session
        .record_patch_applied("patch_deepseek_valid", target.to_string_lossy())
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_tool_call_completed("deepseek_patch_apply", "patch.apply", true)
        .map_err(|error| format!("{error:?}"))?;
    let patch_artifact = write_tool_result_artifact(
        &artifact_store,
        "tool_result_deepseek_patch",
        &ToolResultRecord::new(
            "deepseek_patch_apply",
            "patch.apply",
            true,
            "DeepSeek recorded patch applied",
            format!(
                "{{\"path\":{},\"validation\":\"pass\"}}",
                json_string(&target.to_string_lossy())
            ),
        ),
    )
    .map_err(|error| error.to_string())?;
    session
        .record_tool_result_artifact(
            "deepseek_patch_apply",
            "patch.apply",
            patch_artifact.artifact_id,
            patch_artifact.content_hash,
            "DeepSeek patch result",
        )
        .map_err(|error| format!("{error:?}"))?;
    session
        .start_review()
        .map_err(|error| format!("{error:?}"))?;
    session
        .complete_after_review()
        .map_err(|error| format!("{error:?}"))?;

    let result = RecordedPatchFixtureResult {
        final_state: session.state(),
        event_count: session.event_count(),
        qwen_stale_validation,
        qwen_ambiguous_validation,
        deepseek_patch_validation,
        event_jsonl: session.export_events_jsonl(),
    };
    if config.cleanup {
        let _ = fs::remove_dir_all(&root);
    }
    Ok(result)
}

pub fn run_recorded_live_response_fixture(
    config: &NoModelCodingFixtureConfig,
) -> Result<RecordedLiveResponseFixtureResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-recorded-live-response-{nonce}"));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let artifact_store = ArtifactStore::new(root.join("artifacts"));

    let mut session = AgentSession::new(&config.project_id, &config.session_id, &config.task_id)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::RetrievingContext)
        .map_err(|error| format!("{error:?}"))?;

    let deepseek_adapter = DeepSeekNativeAdapter::new(
        NativeModelProfile {
            profile_id: "deepseek-v4-native".to_string(),
            family: NativeModelFamily::DeepSeek,
            optimization_level: OptimizationLevel::Native,
        },
        "deepseek-v4-flash",
    )
    .map_err(|error| error.to_string())?;
    let deepseek_plan = deepseek_adapter
        .plan_call(&ModelAdapterRequest {
            role: ModelRole::Planner,
            task_summary: "recorded DeepSeek stream".to_string(),
            requires_tools: true,
            context_tokens_estimate: 2_000,
        })
        .map_err(|error| error.to_string())?;
    let deepseek_result = record_native_provider_stream(
        &mut session,
        &artifact_store,
        NativeProviderStreamInput {
            provider: NativeProviderStreamKind::DeepSeek,
            call_id: "ds_call_1",
            stream_id: "ds_stream_1",
            role: ModelRole::Planner,
            plan: &deepseek_plan,
            request_preview: "recorded DeepSeek request",
            transcript_id: "deepseek_recorded_live_transcript",
            live: false,
            record_content_deltas: true,
            lines: &[
                r#"data: {"choices":[{"delta":{"reasoning_content":"Need sk-testsecret from .env"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"Visible DeepSeek answer"}}]}"#,
                r#"data: {"usage":{"prompt_tokens":100,"completion_tokens":20,"reasoning_tokens":15,"prompt_cache_hit_tokens":80,"prompt_cache_miss_tokens":20}}"#,
                "data: [DONE]",
            ],
        },
    )?;

    let qwen_adapter = QwenNativeAdapter::new(
        NativeModelProfile {
            profile_id: "qwen3-6-27b-native".to_string(),
            family: NativeModelFamily::Qwen,
            optimization_level: OptimizationLevel::Native,
        },
        "Qwen/Qwen3.6-27B",
    )
    .map_err(|error| error.to_string())?;
    let qwen_plan = qwen_adapter
        .plan_call(&ModelAdapterRequest {
            role: ModelRole::Executor,
            task_summary: "recorded Qwen stream".to_string(),
            requires_tools: true,
            context_tokens_estimate: 2_000,
        })
        .map_err(|error| error.to_string())?;
    let qwen_result = record_native_provider_stream(
        &mut session,
        &artifact_store,
        NativeProviderStreamInput {
            provider: NativeProviderStreamKind::Qwen,
            call_id: "qw_call_1",
            stream_id: "qw_stream_1",
            role: ModelRole::Executor,
            plan: &qwen_plan,
            request_preview: "recorded Qwen request",
            transcript_id: "qwen_recorded_live_transcript",
            live: false,
            record_content_deltas: true,
            lines: &[
                r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"reasoning_content":"Need sk-testsecret from .env"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"Visible Qwen answer"}}]}"#,
                r#"data: {"usage":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}}"#,
                "data: [DONE]",
            ],
        },
    )?;

    session
        .start_review()
        .map_err(|error| format!("{error:?}"))?;
    session
        .complete_after_review()
        .map_err(|error| format!("{error:?}"))?;

    let event_jsonl = session.export_events_jsonl();
    if event_jsonl.contains("sk-testsecret") || event_jsonl.contains(".env") {
        return Err("recorded live response leaked raw secret/path".to_string());
    }
    let result = RecordedLiveResponseFixtureResult {
        final_state: session.state(),
        event_count: session.event_count(),
        deepseek_transcript_hash: deepseek_result.content_hash,
        qwen_transcript_hash: qwen_result.content_hash,
        event_jsonl,
    };
    if config.cleanup {
        let _ = fs::remove_dir_all(&root);
    }
    Ok(result)
}

pub fn run_recorded_non_stream_response_fixture(
    config: &NoModelCodingFixtureConfig,
) -> Result<RecordedNonStreamResponseFixtureResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("researchcode-recorded-non-stream-response-{nonce}"));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let artifact_store = ArtifactStore::new(root.join("artifacts"));

    let mut session = AgentSession::new(&config.project_id, &config.session_id, &config.task_id)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::RetrievingContext)
        .map_err(|error| format!("{error:?}"))?;

    let deepseek_adapter = DeepSeekNativeAdapter::new(
        NativeModelProfile {
            profile_id: "deepseek-v4-native".to_string(),
            family: NativeModelFamily::DeepSeek,
            optimization_level: OptimizationLevel::Native,
        },
        "deepseek-v4-flash",
    )
    .map_err(|error| error.to_string())?;
    let deepseek_plan = deepseek_adapter
        .plan_call(&ModelAdapterRequest {
            role: ModelRole::Reviewer,
            task_summary: "recorded DeepSeek non-stream response".to_string(),
            requires_tools: false,
            context_tokens_estimate: 2_000,
        })
        .map_err(|error| error.to_string())?;
    let deepseek_normalized = normalize_deepseek_anthropic_response(
        r#"{"model":"deepseek-v4-flash","content":[{"type":"text","text":"Visible response includes sk-responsesecret and .env"}],"reasoning_content":"Need sk-requestsecret from .env","usage":{"input_tokens":100,"output_tokens":24,"reasoning_tokens":8,"cache_read_input_tokens":70,"cache_creation_input_tokens":30}}"#,
    )?;
    let deepseek_result = record_native_provider_response(
        &mut session,
        &artifact_store,
        NativeProviderResponseInput {
            provider: deepseek_normalized.provider,
            call_id: "ds_non_stream_call_1",
            stream_id: "ds_non_stream_response_1",
            role: ModelRole::Reviewer,
            plan: &deepseek_plan,
            request_preview: "request includes .env sk-requestsecret",
            transcript_id: "deepseek_non_stream_transcript",
            live: false,
            visible_content: &deepseek_normalized.visible_content,
            hidden_reasoning_sanitized: deepseek_normalized.hidden_reasoning_sanitized.as_deref(),
            prompt_tokens: deepseek_normalized.prompt_tokens,
            completion_tokens: deepseek_normalized.completion_tokens,
            reasoning_tokens: deepseek_normalized.reasoning_tokens,
            prompt_cache_hit_tokens: deepseek_normalized.prompt_cache_hit_tokens,
            prompt_cache_miss_tokens: deepseek_normalized.prompt_cache_miss_tokens,
        },
    )?;

    let qwen_adapter = QwenNativeAdapter::new(
        NativeModelProfile {
            profile_id: "qwen3-6-27b-native".to_string(),
            family: NativeModelFamily::Qwen,
            optimization_level: OptimizationLevel::Native,
        },
        "Qwen/Qwen3.6-27B",
    )
    .map_err(|error| error.to_string())?;
    let qwen_plan = qwen_adapter
        .plan_call(&ModelAdapterRequest {
            role: ModelRole::Executor,
            task_summary: "recorded Qwen non-stream response".to_string(),
            requires_tools: true,
            context_tokens_estimate: 2_000,
        })
        .map_err(|error| error.to_string())?;
    let qwen_normalized = normalize_qwen_openai_response(
        r#"{"model":"Qwen/Qwen3.6-27B","choices":[{"message":{"reasoning_content":"Need sk-qwenrequestsecret from .env","content":"Qwen visible response with sk-qwenresponsesecret and .env"}}],"usage":{"prompt_tokens":96,"completion_tokens":18,"reasoning_tokens":0}}"#,
    )?;
    let qwen_result = record_native_provider_response(
        &mut session,
        &artifact_store,
        NativeProviderResponseInput {
            provider: qwen_normalized.provider,
            call_id: "qw_non_stream_call_1",
            stream_id: "qw_non_stream_response_1",
            role: ModelRole::Executor,
            plan: &qwen_plan,
            request_preview: "qwen request includes .env sk-qwenrequestsecret",
            transcript_id: "qwen_non_stream_transcript",
            live: false,
            visible_content: &qwen_normalized.visible_content,
            hidden_reasoning_sanitized: qwen_normalized.hidden_reasoning_sanitized.as_deref(),
            prompt_tokens: qwen_normalized.prompt_tokens,
            completion_tokens: qwen_normalized.completion_tokens,
            reasoning_tokens: qwen_normalized.reasoning_tokens,
            prompt_cache_hit_tokens: qwen_normalized.prompt_cache_hit_tokens,
            prompt_cache_miss_tokens: qwen_normalized.prompt_cache_miss_tokens,
        },
    )?;

    let event_jsonl = session.export_events_jsonl();
    if event_jsonl.contains("sk-requestsecret")
        || event_jsonl.contains("sk-responsesecret")
        || event_jsonl.contains("sk-qwenrequestsecret")
        || event_jsonl.contains("sk-qwenresponsesecret")
        || event_jsonl.contains(".env")
    {
        return Err("recorded non-stream response leaked raw secret/path".to_string());
    }
    let result = RecordedNonStreamResponseFixtureResult {
        final_state: session.state(),
        event_count: session.event_count(),
        deepseek_transcript_hash: deepseek_result.content_hash,
        qwen_transcript_hash: qwen_result.content_hash,
        event_jsonl,
    };
    if config.cleanup {
        let _ = fs::remove_dir_all(&root);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_log::EventLog;

    #[test]
    fn no_model_fixture_runs_to_completion_and_exports_events() {
        let result = run_no_model_coding_fixture(&NoModelCodingFixtureConfig::default()).unwrap();
        assert_eq!(result.final_state, AgentState::Completed);
        assert!(result.event_count >= 28);
        assert_eq!(result.matches_count, 1);
        assert!(result.command_artifact_hash.starts_with("fnv64_"));
        assert!(result
            .event_jsonl
            .contains("\"event_type\":\"tool.result_recorded\""));
        assert_eq!(
            EventLog::import_jsonl(&result.event_jsonl).unwrap().len(),
            result.event_count
        );
    }

    #[test]
    fn failure_repair_fixture_diagnoses_and_recovers() {
        let result = run_failure_repair_fixture(&NoModelCodingFixtureConfig::default()).unwrap();
        assert_eq!(result.final_state, AgentState::Completed);
        assert_ne!(result.first_exit_code, 0);
        assert_eq!(result.repaired_exit_code, 0);
        assert!(result
            .event_jsonl
            .contains("\"to_state\":\"DiagnosingFailure\""));
        assert_eq!(
            EventLog::import_jsonl(&result.event_jsonl).unwrap().len(),
            result.event_count
        );
    }

    #[test]
    fn recorded_model_planned_fixture_executes_safe_tool_calls() {
        let result =
            run_recorded_model_planned_fixture(&NoModelCodingFixtureConfig::default()).unwrap();
        assert_eq!(result.final_state, AgentState::Completed);
        assert_eq!(result.deepseek_tool_id, "file.read");
        assert_eq!(result.qwen_tool_id, "file.read");
        assert_eq!(
            result.qwen_mismatch_action,
            ParserAction::BlockNativeSession
        );
        assert!(result
            .event_jsonl
            .contains("\"event_type\":\"tool.result_recorded\""));
        assert_eq!(
            EventLog::import_jsonl(&result.event_jsonl).unwrap().len(),
            result.event_count
        );
    }

    #[test]
    fn recorded_patch_fixture_blocks_stale_and_ambiguous_before_apply() {
        let result = run_recorded_patch_fixture(&NoModelCodingFixtureConfig::default()).unwrap();
        assert_eq!(result.final_state, AgentState::Completed);
        assert_eq!(result.qwen_stale_validation, PatchValidation::FailStale);
        assert_eq!(
            result.qwen_ambiguous_validation,
            PatchValidation::FailAmbiguous
        );
        assert_eq!(result.deepseek_patch_validation, PatchValidation::Pass);
        assert!(result.event_jsonl.contains("\"validation\":\"fail_stale\""));
        assert!(result
            .event_jsonl
            .contains("\"validation\":\"fail_ambiguous\""));
        assert_eq!(
            EventLog::import_jsonl(&result.event_jsonl).unwrap().len(),
            result.event_count
        );
    }

    #[test]
    fn recorded_live_response_fixture_records_stream_and_transcripts() {
        let result =
            run_recorded_live_response_fixture(&NoModelCodingFixtureConfig::default()).unwrap();
        assert_eq!(result.final_state, AgentState::Completed);
        assert!(result.deepseek_transcript_hash.starts_with("fnv64_"));
        assert!(result.qwen_transcript_hash.starts_with("fnv64_"));
        assert!(result
            .event_jsonl
            .contains("\"event_type\":\"model.stream_completed\""));
        assert!(result
            .event_jsonl
            .contains("\"event_type\":\"model.call_completed\""));
        assert!(!result.event_jsonl.contains("sk-testsecret"));
        assert!(!result.event_jsonl.contains(".env"));
        assert_eq!(
            EventLog::import_jsonl(&result.event_jsonl).unwrap().len(),
            result.event_count
        );
    }

    #[test]
    fn recorded_non_stream_response_fixture_uses_normalizer_and_adapter() {
        let result =
            run_recorded_non_stream_response_fixture(&NoModelCodingFixtureConfig::default())
                .unwrap();
        assert_eq!(result.final_state, AgentState::Executing);
        assert!(result.deepseek_transcript_hash.starts_with("fnv64_"));
        assert!(result.qwen_transcript_hash.starts_with("fnv64_"));
        assert!(result
            .event_jsonl
            .contains("\"event_type\":\"model.stream_completed\""));
        assert!(result
            .event_jsonl
            .contains("\"event_type\":\"model.call_completed\""));
        assert!(!result.event_jsonl.contains("sk-requestsecret"));
        assert!(!result.event_jsonl.contains("sk-qwenrequestsecret"));
        assert!(!result.event_jsonl.contains(".env"));
        assert_eq!(
            EventLog::import_jsonl(&result.event_jsonl).unwrap().len(),
            result.event_count
        );
    }
}
