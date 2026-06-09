//! Deterministic core-tool harness.
//!
//! This module treats the kernel `ToolSpec` catalog as a contract and verifies
//! each core tool has at least one concrete positive or boundary fixture.

use crate::patch::stable_text_hash;
use crate::tool_execution::{
    execute_tool, execute_tool_preview, ToolExecutionArgs, ToolExecutionError, ToolExecutionMode,
    ToolExecutionRequest,
};
use researchcode_kernel::tool::core_tool_specs;
use researchcode_kernel::PermissionDecisionKind;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolHarnessCaseResult {
    pub case_id: String,
    pub tool_id: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolHarnessSuiteResult {
    pub passed: bool,
    pub cases: Vec<ToolHarnessCaseResult>,
    pub covered_tools: Vec<String>,
    pub missing_tools: Vec<String>,
}

impl ToolHarnessSuiteResult {
    pub fn passed_count(&self) -> usize {
        self.cases.iter().filter(|case| case.passed).count()
    }

    pub fn to_summary_line(&self) -> String {
        format!(
            "tool harness passed={}/{} tools={}/{} missing={} ok={}",
            self.passed_count(),
            self.cases.len(),
            self.covered_tools.len(),
            enabled_tool_count(),
            self.missing_tools.len(),
            self.passed
        )
    }
}

pub fn run_core_tool_harness_suite() -> ToolHarnessSuiteResult {
    let root = temp_root("tool-harness");
    let mut cases = Vec::new();
    prepare_workspace(&root);

    cases.push(case_file_read_positive(&root));
    cases.push(case_list_directory_positive(&root));
    cases.push(case_list_tree_positive(&root));
    cases.push(case_file_read_sensitive_denied(&root));
    cases.push(case_file_read_escape_denied(&root));
    cases.push(case_search_positive(&root));
    cases.push(case_search_escape_denied(&root));
    cases.push(case_repo_map_positive(&root));
    cases.push(case_git_status_no_repo(&root));
    cases.push(case_shell_requires_permission(&root));
    cases.push(case_shell_allowed_non_destructive(&root));
    cases.push(case_shell_destructive_blocked(&root));
    cases.push(case_file_edit_applies_with_permission(&root));
    cases.push(case_file_edit_ambiguous_denied(&root));
    cases.push(case_file_write_creates_with_permission(&root));
    cases.push(case_file_multi_edit_applies_with_permission(&root));
    cases.push(case_patch_stale_denied(&root));
    cases.push(case_patch_applies_with_permission(&root));
    cases.push(case_lsp_diagnostics_stub(&root));
    cases.push(case_todo_write_stub(&root));
    cases.push(case_plan_enter_requires_governance(&root));
    cases.push(case_plan_exit_requires_governance(&root));
    cases.push(case_plan_write_preview(&root));
    cases.push(case_ask_user_stub(&root));
    cases.push(case_research_csv_profile_positive());
    cases.push(case_artifact_export_requires_permission(&root));
    cases.push(case_task_dispatch_preview(&root));

    let all = core_tool_specs()
        .into_iter()
        .filter(|tool| tool.enabled_by_default)
        .map(|tool| tool.tool_id.clone())
        .collect::<HashSet<_>>();
    let covered = covered_tools(&cases)
        .into_iter()
        .filter(|tool_id| all.contains(tool_id))
        .collect::<Vec<_>>();
    let missing = all
        .difference(&covered.iter().cloned().collect::<HashSet<_>>())
        .cloned()
        .collect::<Vec<_>>();
    let passed = cases.iter().all(|case| case.passed) && missing.is_empty();
    let _ = fs::remove_dir_all(root);
    ToolHarnessSuiteResult {
        passed,
        cases,
        covered_tools: {
            let mut values = covered;
            values.sort();
            values
        },
        missing_tools: {
            let mut values = missing;
            values.sort();
            values
        },
    }
}

fn enabled_tool_count() -> usize {
    core_tool_specs()
        .into_iter()
        .filter(|tool| tool.enabled_by_default)
        .count()
}

fn case_file_read_positive(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool_preview(&request(
        root,
        "file.read",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs {
            path: Some("README.md".to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "file_read_positive",
        "file.read",
        result.as_ref().is_ok_and(|value| value.ok),
        format!("{result:?}"),
    )
}

fn case_list_directory_positive(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool_preview(&request(
        root,
        "file.list_directory",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs {
            path: Some(".".to_string()),
            max_results: Some(64),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "list_directory_positive",
        "file.list_directory",
        result
            .as_ref()
            .is_ok_and(|value| value.ok && value.preview.contains("listed")),
        format!("{result:?}"),
    )
}

fn case_list_tree_positive(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool_preview(&request(
        root,
        "file.list_tree",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs {
            path: Some(".".to_string()),
            max_depth: Some(2),
            max_results: Some(80),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "list_tree_positive",
        "file.list_tree",
        result
            .as_ref()
            .is_ok_and(|value| value.ok && value.preview.contains("tree lines=")),
        format!("{result:?}"),
    )
}

fn case_file_read_sensitive_denied(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool_preview(&request(
        root,
        "file.read",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs {
            path: Some(".env".to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "file_read_sensitive_denied",
        "file.read",
        matches!(result, Err(ToolExecutionError::SensitivePath(_))),
        format!("{result:?}"),
    )
}

fn case_file_read_escape_denied(root: &PathBuf) -> ToolHarnessCaseResult {
    let outside = root
        .parent()
        .unwrap()
        .join(format!("researchcode-outside-{}", nonce()));
    fs::write(&outside, "outside\n").unwrap();
    let result = execute_tool_preview(&request(
        root,
        "file.read",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs {
            path: Some(outside.to_string_lossy().to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    let _ = fs::remove_file(outside);
    pass_case(
        "file_read_escape_denied",
        "file.read",
        matches!(result, Err(ToolExecutionError::PathEscapesWorkspace(_))),
        format!("{result:?}"),
    )
}

fn case_search_positive(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool_preview(&request(
        root,
        "search.ripgrep",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs {
            pattern: Some("needle".to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "search_positive",
        "search.ripgrep",
        result.as_ref().is_ok_and(|value| value.ok),
        format!("{result:?}"),
    )
}

fn case_search_escape_denied(root: &PathBuf) -> ToolHarnessCaseResult {
    let outside = root
        .parent()
        .unwrap()
        .join(format!("researchcode-outside-dir-{}", nonce()));
    fs::create_dir_all(&outside).unwrap();
    let result = execute_tool_preview(&request(
        root,
        "search.ripgrep",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs {
            root: Some(outside.to_string_lossy().to_string()),
            pattern: Some("needle".to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    let _ = fs::remove_dir_all(outside);
    pass_case(
        "search_escape_denied",
        "search.ripgrep",
        result.as_ref().is_ok_and(|value| {
            !value.ok
                && value
                    .detail_json
                    .contains("\"error_code\":\"path_escapes_workspace\"")
        }),
        format!("{result:?}"),
    )
}

fn case_repo_map_positive(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool_preview(&request(
        root,
        "repo.map",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs::default(),
    ));
    pass_case(
        "repo_map_positive",
        "repo.map",
        result
            .as_ref()
            .is_ok_and(|value| value.preview.contains("repo map files=")),
        format!("{result:?}"),
    )
}

fn case_git_status_no_repo(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool_preview(&request(
        root,
        "git.status",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs::default(),
    ));
    pass_case(
        "git_status_no_repo",
        "git.status",
        result.as_ref().is_ok_and(|value| value.ok),
        format!("{result:?}"),
    )
}

fn case_shell_requires_permission(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool(&request(
        root,
        "shell.command",
        ToolExecutionMode::ApplyWithPermission {
            permission_decision: None,
        },
        ToolExecutionArgs {
            command: Some("npm install lodash".to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "shell_requires_permission",
        "shell.command",
        matches!(result, Err(ToolExecutionError::PermissionRequired(_))),
        format!("{result:?}"),
    )
}

fn case_shell_allowed_non_destructive(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool(&request(
        root,
        "shell.command",
        ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        ToolExecutionArgs {
            command: Some("find . -maxdepth 0".to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "shell_allowed_non_destructive",
        "shell.command",
        result.as_ref().is_ok_and(|value| value.ok),
        format!("{result:?}"),
    )
}

fn case_shell_destructive_blocked(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool(&request(
        root,
        "shell.command",
        ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        ToolExecutionArgs {
            command: Some("rm -rf .".to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "shell_destructive_blocked",
        "shell.command",
        matches!(result, Err(ToolExecutionError::PermissionRequired(_))),
        format!("{result:?}"),
    )
}

fn case_file_edit_applies_with_permission(root: &PathBuf) -> ToolHarnessCaseResult {
    let path = root.join("src/edit.rs");
    fs::write(&path, "pub const VALUE: u8 = 1;\n").unwrap();
    let before = fs::read_to_string(&path).unwrap();
    let result = execute_tool(&request(
        root,
        "file.edit",
        ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        ToolExecutionArgs {
            path: Some("src/edit.rs".to_string()),
            old_string: Some("VALUE: u8 = 1".to_string()),
            new_string: Some("VALUE: u8 = 2".to_string()),
            base_hash: Some(stable_text_hash(&before)),
            ..ToolExecutionArgs::default()
        },
    ));
    let after = fs::read_to_string(&path).unwrap();
    pass_case(
        "file_edit_applies_with_permission",
        "file.edit",
        result.as_ref().is_ok_and(|value| value.ok) && after.contains("VALUE: u8 = 2"),
        format!("{result:?}"),
    )
}

fn case_file_edit_ambiguous_denied(root: &PathBuf) -> ToolHarnessCaseResult {
    let path = root.join("src/ambiguous.rs");
    fs::write(&path, "hit();\nhit();\n").unwrap();
    let before = fs::read_to_string(&path).unwrap();
    let result = execute_tool(&request(
        root,
        "file.edit",
        ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        ToolExecutionArgs {
            path: Some("src/ambiguous.rs".to_string()),
            old_string: Some("hit();".to_string()),
            new_string: Some("miss();".to_string()),
            base_hash: Some(stable_text_hash(&before)),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "file_edit_ambiguous_denied",
        "file.edit",
        matches!(result, Err(ToolExecutionError::ValidationFailed(_))),
        format!("{result:?}"),
    )
}

fn case_file_write_creates_with_permission(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool(&request(
        root,
        "file.write",
        ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        ToolExecutionArgs {
            path: Some("src/generated.rs".to_string()),
            content: Some("pub const GENERATED: bool = true;\n".to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    let after = fs::read_to_string(root.join("src/generated.rs")).unwrap_or_default();
    pass_case(
        "file_write_creates_with_permission",
        "file.write",
        result.as_ref().is_ok_and(|value| value.ok) && after.contains("GENERATED"),
        format!("{result:?}"),
    )
}

fn case_file_multi_edit_applies_with_permission(root: &PathBuf) -> ToolHarnessCaseResult {
    let path = root.join("src/multi.rs");
    fs::write(&path, "alpha();\nbeta();\n").unwrap();
    let before = fs::read_to_string(&path).unwrap();
    let result = execute_tool(&request(
        root,
        "file.multi_edit",
        ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        ToolExecutionArgs {
            path: Some("src/multi.rs".to_string()),
            base_hash: Some(stable_text_hash(&before)),
            edits_json: Some(
                r#"[{"old_string":"alpha();","new_string":"gamma();"},{"old_string":"beta();","new_string":"delta();"}]"#
                    .to_string(),
            ),
            ..ToolExecutionArgs::default()
        },
    ));
    let after = fs::read_to_string(&path).unwrap();
    pass_case(
        "file_multi_edit_applies_with_permission",
        "file.multi_edit",
        result.as_ref().is_ok_and(|value| value.ok)
            && after.contains("gamma();")
            && after.contains("delta();"),
        format!("{result:?}"),
    )
}

fn case_patch_stale_denied(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool(&request(
        root,
        "patch.apply",
        ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        ToolExecutionArgs {
            path: Some("src/lib.rs".to_string()),
            old_string: Some("pub const RETRY: u8 = 3".to_string()),
            new_string: Some("pub const RETRY: u8 = 5".to_string()),
            base_hash: Some("stale_hash".to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "patch_stale_denied",
        "patch.apply",
        matches!(result, Err(ToolExecutionError::ValidationFailed(_))),
        format!("{result:?}"),
    )
}

fn case_patch_applies_with_permission(root: &PathBuf) -> ToolHarnessCaseResult {
    let path = root.join("src/lib.rs");
    let before = fs::read_to_string(&path).unwrap();
    let result = execute_tool(&request(
        root,
        "patch.apply",
        ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        ToolExecutionArgs {
            path: Some("src/lib.rs".to_string()),
            old_string: Some("pub const RETRY: u8 = 3".to_string()),
            new_string: Some("pub const RETRY: u8 = 5".to_string()),
            base_hash: Some(stable_text_hash(&before)),
            ..ToolExecutionArgs::default()
        },
    ));
    let after = fs::read_to_string(&path).unwrap();
    pass_case(
        "patch_applies_with_permission",
        "patch.apply",
        result.as_ref().is_ok_and(|value| value.ok) && after.contains("RETRY: u8 = 5"),
        format!("{result:?}"),
    )
}

fn case_lsp_diagnostics_stub(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool_preview(&request(
        root,
        "lsp.diagnostics",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs {
            path: Some("src/lib.rs".to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "lsp_diagnostics_stub",
        "lsp.diagnostics",
        result
            .as_ref()
            .is_ok_and(|value| value.ok && value.detail_json.contains("\"available\":false")),
        format!("{result:?}"),
    )
}

fn case_todo_write_stub(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool(&request(
        root,
        "todo.write",
        ToolExecutionMode::ApplyWithPermission {
            permission_decision: None,
        },
        ToolExecutionArgs {
            content: Some("- [ ] inspect\n- [ ] patch\n".to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "todo_write_updates_view",
        "todo.write",
        result
            .as_ref()
            .is_ok_and(|value| value.ok && value.detail_json.contains("\"item_count\":2")),
        format!("{result:?}"),
    )
}

fn case_plan_enter_requires_governance(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool_preview(&request(
        root,
        "plan.enter",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs::default(),
    ));
    pass_case(
        "plan_enter_requires_governance",
        "plan.enter",
        matches!(result, Err(ToolExecutionError::PermissionRequired(_))),
        format!("{result:?}"),
    )
}

fn case_plan_exit_requires_governance(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool_preview(&request(
        root,
        "plan.exit",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs::default(),
    ));
    pass_case(
        "plan_exit_requires_governance",
        "plan.exit",
        matches!(result, Err(ToolExecutionError::PermissionRequired(_))),
        format!("{result:?}"),
    )
}

fn case_ask_user_stub(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool(&request(
        root,
        "ask_user",
        ToolExecutionMode::ApplyWithPermission {
            permission_decision: None,
        },
        ToolExecutionArgs {
            query: Some("Which file should I inspect?".to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "ask_user_queues_question",
        "ask_user",
        result
            .as_ref()
            .is_ok_and(|value| value.ok && value.detail_json.contains("Which file")),
        format!("{result:?}"),
    )
}

fn case_task_dispatch_preview(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool(&request(
        root,
        "task.dispatch",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs {
            content: Some("Inspect README in an isolated child task.".to_string()),
            model_role: Some("reviewer".to_string()),
            write_scope_json: Some(r#"{"paths":["src"]}"#.to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "task_dispatch_preview",
        "task.dispatch",
        result.as_ref().is_ok_and(|value| {
            value.ok
                && value.detail_json.contains("\"status\":\"completed\"")
                && value.detail_json.contains("\"evidence_refs\"")
                && value.detail_json.contains("\"model_role\":\"reviewer\"")
                && value
                    .detail_json
                    .contains("\"write_scope\":{\"paths\":[\"src\"]}")
        }),
        format!("{result:?}"),
    )
}

fn case_research_csv_profile_positive() -> ToolHarnessCaseResult {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .unwrap()
        .to_path_buf();
    let output_dir = std::env::temp_dir().join(format!("researchcode-tool-harness-rw-{}", nonce()));
    let result = execute_tool(&ToolExecutionRequest {
        workspace_root,
        tool_call_id: "tool_harness_research".to_string(),
        tool_id: "research.csv_profile".to_string(),
        mode: ToolExecutionMode::ReadOnlyPreview,
        args: ToolExecutionArgs {
            input_csv: Some("eval/fixtures/research/csv-quality-small/input.csv".to_string()),
            job_id: Some("tool_harness_research".to_string()),
            output_dir: Some(output_dir.to_string_lossy().to_string()),
            ..ToolExecutionArgs::default()
        },
    });
    let _ = fs::remove_dir_all(output_dir);
    pass_case(
        "research_csv_profile_positive",
        "research.csv_profile",
        result
            .as_ref()
            .is_ok_and(|value| value.ok && value.preview.contains("manifest_hash")),
        format!("{result:?}"),
    )
}

fn case_artifact_export_requires_permission(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool_preview(&request(
        root,
        "artifact.export",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs::default(),
    ));
    pass_case(
        "artifact_export_requires_permission",
        "artifact.export",
        matches!(result, Err(ToolExecutionError::PermissionRequired(_))),
        format!("{result:?}"),
    )
}

fn case_plan_write_preview(root: &PathBuf) -> ToolHarnessCaseResult {
    let result = execute_tool_preview(&request(
        root,
        "plan.write",
        ToolExecutionMode::ReadOnlyPreview,
        ToolExecutionArgs {
            content: Some("Plan content".to_string()),
            ..ToolExecutionArgs::default()
        },
    ));
    pass_case(
        "plan_write_preview",
        "plan.write",
        result
            .as_ref()
            .is_ok_and(|value| value.ok && value.preview.contains("RuntimeFacade")),
        format!("{result:?}"),
    )
}

fn prepare_workspace(root: &PathBuf) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("README.md"), "ResearchCode\nneedle\n").unwrap();
    fs::write(root.join(".env"), "SECRET=1\n").unwrap();
    fs::write(root.join("src/lib.rs"), "pub const RETRY: u8 = 3;\n").unwrap();
}

fn request(
    root: &PathBuf,
    tool_id: &str,
    mode: ToolExecutionMode,
    args: ToolExecutionArgs,
) -> ToolExecutionRequest {
    ToolExecutionRequest {
        workspace_root: root.clone(),
        tool_call_id: format!("{}_call", tool_id.replace('.', "_")),
        tool_id: tool_id.to_string(),
        mode,
        args,
    }
}

fn pass_case(case_id: &str, tool_id: &str, passed: bool, detail: String) -> ToolHarnessCaseResult {
    ToolHarnessCaseResult {
        case_id: case_id.to_string(),
        tool_id: tool_id.to_string(),
        passed,
        detail,
    }
}

fn covered_tools(cases: &[ToolHarnessCaseResult]) -> Vec<String> {
    cases
        .iter()
        .filter(|case| case.passed)
        .map(|case| case.tool_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("researchcode-{label}-{}", nonce()));
    fs::create_dir_all(&root).unwrap();
    root
}

fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_tool_harness_covers_every_tool() {
        let result = run_core_tool_harness_suite();
        assert!(result.passed, "{result:?}");
        assert_eq!(result.missing_tools, Vec::<String>::new());
    }
}
