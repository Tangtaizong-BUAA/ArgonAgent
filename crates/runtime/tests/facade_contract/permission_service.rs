use super::support::*;
use researchcode_kernel::{PermissionDecisionKind, PermissionRequestType};
use researchcode_runtime::runtime_facade::{AutonomyMode, FacadeToolOutcome, RuntimeModelMode};
use researchcode_runtime::tool_execution::ToolExecutionArgs;

#[test]
fn conservative_shell_command_requires_permission() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let outcome = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "cmd1",
            "shell.command",
            ToolExecutionArgs {
                command: Some("echo hi".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    assert_eq!(
        outcome,
        FacadeToolOutcome::RequiresPermission {
            permission_id: "cmd1_permission".to_string(),
            request_type: PermissionRequestType::Command
        }
    );
}

#[test]
fn hard_denied_shell_command_is_blocked_without_permission() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    for (tool_call_id, command) in [
        ("cmd_hard_rm", "rm -rf target"),
        ("cmd_hard_bash", "bash -lc ls"),
        ("cmd_hard_zsh", "zsh -lc ls"),
        ("cmd_hard_dash", "dash -c ls"),
        ("cmd_hard_systemctl", "systemctl restart service"),
        ("cmd_hard_bin_bash", "/bin/bash -lc ls"),
        ("cmd_hard_usr_sudo", "/usr/bin/sudo ls"),
        ("cmd_hard_sbin_mkfs", "/sbin/mkfs.ext4 /dev/disk1"),
    ] {
        let outcome = fx
            .facade
            .execute_session_tool(
                &handle.session_id,
                tool_call_id,
                "shell.command",
                ToolExecutionArgs {
                    command: Some(command.to_string()),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        assert!(
            matches!(outcome, FacadeToolOutcome::BlockedByPolicy(_)),
            "{command} should be blocked before permission"
        );
    }
}

#[test]
fn fast_auto_safe_shell_executes_and_records_decision() {
    let fx = FacadeFixture::new();
    let handle = fx.start_with(RuntimeModelMode::DeepSeek, AutonomyMode::FastAuto);
    let outcome = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "cmd_safe",
            "shell.command",
            ToolExecutionArgs {
                command: Some("ls".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    assert!(matches!(outcome, FacadeToolOutcome::Executed(_)));
    let stream = fx.facade.stream_agent_events(&handle.session_id).unwrap();
    assert_eq!(
        count_event_type(&stream.jsonl, "permission.decision.recorded"),
        1
    );
}

#[test]
fn submit_permission_decision_rejects_unknown_session() {
    let fx = FacadeFixture::new();
    assert!(fx
        .facade
        .submit_permission_decision("missing", "perm", PermissionDecisionKind::Deny)
        .is_err());
}

#[test]
fn submit_permission_decision_rejects_mismatched_id() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let _ = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "cmd1",
            "shell.command",
            ToolExecutionArgs {
                command: Some("echo hi".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    assert!(fx
        .facade
        .submit_permission_decision(
            &handle.session_id,
            "wrong_permission",
            PermissionDecisionKind::Deny
        )
        .is_err());
}

#[test]
fn submit_permission_decision_with_outcome_records_denial() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let _ = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "cmd1",
            "shell.command",
            ToolExecutionArgs {
                command: Some("echo hi".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    let outcome = fx
        .facade
        .submit_permission_decision_with_outcome(
            &handle.session_id,
            "cmd1_permission",
            PermissionDecisionKind::Deny,
        )
        .unwrap();
    assert_eq!(outcome.resume_strategy, "decision_recorded");
    assert!(!outcome.tool_executed);
}

#[test]
fn continue_session_tool_after_permission_denies_without_execution() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let _ = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "cmd1",
            "shell.command",
            ToolExecutionArgs {
                command: Some("echo hi".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    let outcome = fx
        .facade
        .continue_session_tool_after_permission(
            &handle.session_id,
            "cmd1",
            "shell.command",
            ToolExecutionArgs {
                command: Some("echo hi".to_string()),
                ..ToolExecutionArgs::default()
            },
            PermissionDecisionKind::Deny,
        )
        .unwrap();
    assert!(matches!(outcome, FacadeToolOutcome::BlockedByPolicy(_)));
}

#[test]
fn continue_session_tool_after_permission_allows_command_execution() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let _ = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "cmd1",
            "shell.command",
            ToolExecutionArgs {
                command: Some("echo hi".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    let outcome = fx
        .facade
        .continue_session_tool_after_permission(
            &handle.session_id,
            "cmd1",
            "shell.command",
            ToolExecutionArgs {
                command: Some("echo hi".to_string()),
                ..ToolExecutionArgs::default()
            },
            PermissionDecisionKind::AllowOnce,
        )
        .unwrap();
    assert!(matches!(outcome, FacadeToolOutcome::Executed(_)));
}

#[test]
fn allow_project_rule_persists_project_policy_file() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let _ = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "cmd1",
            "shell.command",
            ToolExecutionArgs {
                command: Some("echo hi".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    fx.facade
        .continue_session_tool_after_permission(
            &handle.session_id,
            "cmd1",
            "shell.command",
            ToolExecutionArgs {
                command: Some("echo hi".to_string()),
                ..ToolExecutionArgs::default()
            },
            PermissionDecisionKind::AllowProjectRule,
        )
        .unwrap();
    assert!(fx.artifacts.join("permission_policy.tsv").exists());
}

#[test]
fn file_edit_without_prior_read_is_blocked_by_read_before_write() {
    let fx = FacadeFixture::new();
    fx.write_workspace_file("src/lib.rs", "old\n");
    let handle = fx.start();
    let outcome = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "edit1",
            "file.edit",
            ToolExecutionArgs {
                path: Some("src/lib.rs".to_string()),
                old_string: Some("old".to_string()),
                new_string: Some("new".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    assert!(matches!(outcome, FacadeToolOutcome::BlockedByPolicy(_)));
}

#[test]
fn fast_auto_file_write_executes_safe_create() {
    let fx = FacadeFixture::new();
    let handle = fx.start_with(RuntimeModelMode::DeepSeek, AutonomyMode::FastAuto);
    let outcome = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "write1",
            "file.write",
            ToolExecutionArgs {
                path: Some("new.txt".to_string()),
                content: Some("created".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    assert!(matches!(outcome, FacadeToolOutcome::Executed(_)));
    assert_eq!(fx.read_workspace_file("new.txt"), "created");
}

#[test]
fn artifact_export_currently_fails_at_execution_permission_boundary() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let error = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "artifact1",
            "artifact.export",
            ToolExecutionArgs::default(),
        )
        .unwrap_err();
    assert!(error.contains("PermissionRequired"));
}

#[test]
fn permission_decision_recorded_is_one_to_one_for_fast_auto_gate() {
    let fx = FacadeFixture::new();
    let handle = fx.start_with(RuntimeModelMode::DeepSeek, AutonomyMode::FastAuto);
    let _ = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "write1",
            "file.write",
            ToolExecutionArgs {
                path: Some("one.txt".to_string()),
                content: Some("one".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    let _ = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "write2",
            "file.write",
            ToolExecutionArgs {
                path: Some("two.txt".to_string()),
                content: Some("two".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    let stream = fx.facade.stream_agent_events(&handle.session_id).unwrap();
    assert_eq!(
        count_event_type(&stream.jsonl, "permission.decision.recorded"),
        2
    );
}
