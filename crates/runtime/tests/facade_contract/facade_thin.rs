use super::support::*;
use researchcode_kernel::{PermissionDecisionKind, PlanApprovalDecisionKind};
use researchcode_runtime::native_provider::NativeProviderEndpoint;
use researchcode_runtime::runtime_facade::{AutonomyMode, FacadeToolOutcome, RuntimeModelMode};
use researchcode_runtime::tool_execution::ToolExecutionArgs;

#[test]
fn autonomy_mode_contract_values_are_stable() {
    assert_eq!(AutonomyMode::Conservative.as_str(), "conservative");
    assert_eq!(AutonomyMode::FastAuto.as_str(), "fast_auto");
    assert_eq!(AutonomyMode::ManualReview.as_str(), "manual_review");
}

#[test]
fn preview_tool_rejects_unknown_tool() {
    let fx = FacadeFixture::new();
    assert!(fx
        .facade
        .preview_tool(
            &fx.workspace,
            "unknown",
            "unknown.tool",
            ToolExecutionArgs::default()
        )
        .is_err());
}

#[test]
fn preview_tool_reads_workspace_file() {
    let fx = FacadeFixture::new();
    fx.write_workspace_file("README.md", "preview contract");
    let result = fx
        .facade
        .preview_tool(
            &fx.workspace,
            "read_preview",
            "file.read",
            ToolExecutionArgs {
                path: Some("README.md".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    assert!(result.ok);
    assert!(result.preview.contains("preview contract"));
}

#[test]
fn preview_tool_lists_workspace_directory() {
    let fx = FacadeFixture::new();
    fx.write_workspace_file("src/lib.rs", "pub fn x() {}\n");
    let result = fx
        .facade
        .preview_tool(
            &fx.workspace,
            "list_preview",
            "file.list_directory",
            ToolExecutionArgs {
                root: Some("src".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    assert!(result.ok);
    assert!(result.detail_json.contains("lib.rs"));
}

#[test]
fn execute_session_tool_rejects_unknown_session() {
    let fx = FacadeFixture::new();
    assert!(fx
        .facade
        .execute_session_tool("missing", "tc", "file.read", ToolExecutionArgs::default())
        .is_err());
}

#[test]
fn execute_session_tool_rejects_unknown_tool_as_error_boundary() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let error = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "unknown",
            "unknown.tool",
            ToolExecutionArgs::default(),
        )
        .unwrap_err();
    assert!(error.contains("UnknownTool"));
}

#[test]
fn ask_user_tool_sets_waiting_state() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let outcome = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "ask1",
            "ask_user",
            ToolExecutionArgs {
                query: Some("clarify?".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    assert!(matches!(outcome, FacadeToolOutcome::Executed(_)));
    assert_eq!(
        format!("{:?}", fx.snapshot(&handle.session_id).state),
        "WaitingForUser"
    );
}

#[test]
fn plan_enter_returns_plan_approval_id() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let outcome = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "plan1",
            "plan.enter",
            ToolExecutionArgs {
                content: Some("{\"goal\":\"ship\"}".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    assert_eq!(
        outcome,
        FacadeToolOutcome::RequiresPlanApproval {
            plan_approval_id: "plan1_plan_approval".to_string()
        }
    );
}

#[test]
fn plan_mode_blocks_shell_until_exit() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let _ = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "plan1",
            "plan.enter",
            ToolExecutionArgs::default(),
        )
        .unwrap();
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
    assert!(matches!(outcome, FacadeToolOutcome::BlockedByPolicy(_)));
}

#[test]
fn plan_exit_clears_plan_mode() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let _ = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "plan1",
            "plan.enter",
            ToolExecutionArgs::default(),
        )
        .unwrap();
    let outcome = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "plan_exit",
            "plan.exit",
            ToolExecutionArgs::default(),
        )
        .unwrap();
    assert!(matches!(outcome, FacadeToolOutcome::Executed(_)));
    assert!(!fx.snapshot(&handle.session_id).plan_mode_active);
}

#[test]
fn plan_write_writes_session_plan_file() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let outcome = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "plan_write",
            "plan.write",
            ToolExecutionArgs {
                content: Some("# Plan\n".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    assert!(matches!(outcome, FacadeToolOutcome::Executed(_)));
    assert!(fx
        .workspace
        .join(".researchcode/plans")
        .join(format!("{}.md", handle.session_id))
        .exists());
}

#[test]
fn submit_plan_decision_rejects_wrong_id() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let _ = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "plan1",
            "plan.enter",
            ToolExecutionArgs::default(),
        )
        .unwrap();
    assert!(fx
        .facade
        .submit_plan_decision(
            &handle.session_id,
            "wrong",
            PlanApprovalDecisionKind::Approve
        )
        .is_err());
}

#[test]
fn submit_plan_decision_approve_clears_pending_plan() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let _ = fx
        .facade
        .execute_session_tool(
            &handle.session_id,
            "plan1",
            "plan.enter",
            ToolExecutionArgs::default(),
        )
        .unwrap();
    fx.facade
        .submit_plan_decision(
            &handle.session_id,
            "plan1_plan_approval",
            PlanApprovalDecisionKind::Approve,
        )
        .unwrap();
    assert_eq!(
        fx.snapshot(&handle.session_id).pending_plan_approval_count,
        0
    );
}

#[test]
fn record_live_model_blocked_adds_event() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    fx.facade
        .record_live_model_blocked(&handle.session_id, "deepseek", "disabled")
        .unwrap();
    let stream = fx.facade.stream_agent_events(&handle.session_id).unwrap();
    assert!(stream.jsonl.contains("model.call_blocked"));
}

#[test]
fn record_runtime_error_sets_failed_state() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    fx.facade
        .record_runtime_error(&handle.session_id, "contract_error", "boom")
        .unwrap();
    assert_eq!(
        format!("{:?}", fx.snapshot(&handle.session_id).state),
        "Failed"
    );
}

#[test]
fn run_ultraplan_fixture_requires_deepseek() {
    let fx = FacadeFixture::new();
    let handle = fx.start_with(RuntimeModelMode::Qwen, AutonomyMode::Conservative);
    assert!(fx
        .facade
        .run_ultraplan_fixture(&handle.session_id, "goal")
        .is_err());
}

#[test]
fn run_ultraplan_fixture_requests_plan_approval_for_deepseek() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let plan = fx
        .facade
        .run_ultraplan_fixture(&handle.session_id, "goal")
        .unwrap();
    assert!(!plan.plan_id.is_empty());
    assert_eq!(
        fx.snapshot(&handle.session_id).pending_plan_approval_count,
        1
    );
}

#[test]
fn run_ultrareview_fixture_records_summary_for_deepseek() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let report = fx
        .facade
        .run_ultrareview_fixture(&handle.session_id, "target")
        .unwrap();
    assert!(!report.report_id.is_empty());
    let stream = fx.facade.stream_agent_events(&handle.session_id).unwrap();
    assert!(stream.jsonl.contains("ultrareview.completed"));
}

#[test]
fn run_deepseek_agent_loop_rejects_qwen_session() {
    let fx = FacadeFixture::new();
    let handle = fx.start_with(RuntimeModelMode::Qwen, AutonomyMode::Conservative);
    let transport =
        researchcode_runtime::live_http_transport::ScriptedLiveHttpTransport::new(Vec::new());
    let error = fx
        .facade
        .run_deepseek_agent_loop_with_transport(
            &transport,
            &handle.session_id,
            "prompt",
            NativeProviderEndpoint::deepseek_v4_flash_openai(),
            1,
            0,
        )
        .unwrap_err();
    assert!(error.contains("requires deepseek mode"));
}

#[test]
fn run_qwen_agent_loop_rejects_deepseek_session() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let transport =
        researchcode_runtime::live_http_transport::ScriptedLiveHttpTransport::new(Vec::new());
    let error = fx
        .facade
        .run_qwen_agent_loop_with_transport(
            &transport,
            &handle.session_id,
            "prompt",
            NativeProviderEndpoint::qwen36_27b_custom_endpoint(),
            1,
            0,
        )
        .unwrap_err();
    assert!(error.contains("requires qwen mode"));
}

#[test]
fn export_events_rejects_unknown_session() {
    let fx = FacadeFixture::new();
    assert!(fx
        .facade
        .export_events("missing", &fx.artifacts.join("missing.jsonl"))
        .is_err());
}

#[test]
fn submit_permission_decision_unknown_session_stays_error_boundary() {
    let fx = FacadeFixture::new();
    assert!(fx
        .facade
        .submit_permission_decision("missing", "perm", PermissionDecisionKind::Deny)
        .is_err());
}
