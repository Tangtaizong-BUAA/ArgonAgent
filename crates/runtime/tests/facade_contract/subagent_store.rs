use super::support::*;
use researchcode_kernel::model::NativeModelFamily;
use researchcode_kernel::PermissionDecisionKind;
use researchcode_runtime::runtime_facade::AutonomyMode;
use researchcode_runtime::subagent::{SubagentRequest, SubagentStatus, SubagentType};
use researchcode_runtime::tool_execution::ToolExecutionArgs;

#[test]
fn spawn_subagent_rejects_unknown_parent() {
    let fx = FacadeFixture::new();
    let request = readonly_request("missing", "scan");
    assert!(fx.facade.spawn_subagent("missing", request).is_err());
}

#[test]
fn spawn_subagent_creates_explorer_child() {
    let fx = FacadeFixture::new();
    let parent = fx.start();
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            readonly_request(&parent.session_id, "scan docs"),
        )
        .unwrap();
    assert_eq!(child.parent_session_id, parent.session_id);
    assert_eq!(child.agent_type, SubagentType::Explorer);
    assert_eq!(child.status, SubagentStatus::Created);
}

#[test]
fn spawn_subagent_records_parent_event() {
    let fx = FacadeFixture::new();
    let parent = fx.start();
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            readonly_request(&parent.session_id, "scan docs"),
        )
        .unwrap();
    let parent_stream = fx.facade.stream_agent_events(&parent.session_id).unwrap();
    assert!(parent_stream.jsonl.contains(&child.subagent_id));
    assert!(contains_event_type(
        &parent_stream.jsonl,
        "subagent.spawned"
    ));
}

#[test]
fn spawn_subagent_records_child_event_stream() {
    let fx = FacadeFixture::new();
    let parent = fx.start();
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            readonly_request(&parent.session_id, "scan docs"),
        )
        .unwrap();
    let stream = fx
        .facade
        .stream_subagent_events(&child.subagent_id)
        .unwrap();
    assert!(contains_event_type(&stream.jsonl, "subagent.child_created"));
}

#[test]
fn send_subagent_message_moves_child_to_running() {
    let fx = FacadeFixture::new();
    let parent = fx.start();
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            readonly_request(&parent.session_id, "scan docs"),
        )
        .unwrap();
    fx.facade
        .send_subagent_message(&child.subagent_id, "please inspect")
        .unwrap();
    let resumed = fx.facade.resume_subagent(&child.subagent_id).unwrap();
    assert_eq!(resumed.status, SubagentStatus::Running);
}

#[test]
fn send_subagent_message_rejects_unknown_child() {
    let fx = FacadeFixture::new();
    assert!(fx.facade.send_subagent_message("missing", "hello").is_err());
}

#[test]
fn send_subagent_message_rejects_terminal_child() {
    let fx = FacadeFixture::new();
    let parent = fx.start();
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            readonly_request(&parent.session_id, "scan docs"),
        )
        .unwrap();
    fx.facade.summarize_subagent(&child.subagent_id).unwrap();
    assert!(fx
        .facade
        .send_subagent_message(&child.subagent_id, "too late")
        .is_err());
}

#[test]
fn resume_subagent_rejects_unknown_child() {
    let fx = FacadeFixture::new();
    assert!(fx.facade.resume_subagent("missing").is_err());
}

#[test]
fn cancel_subagent_sets_cancelled_status() {
    let fx = FacadeFixture::new();
    let parent = fx.start();
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            readonly_request(&parent.session_id, "scan docs"),
        )
        .unwrap();
    let cancelled = fx.facade.cancel_subagent(&child.subagent_id).unwrap();
    assert_eq!(cancelled.status, SubagentStatus::Cancelled);
}

#[test]
fn cancel_subagent_records_parent_and_child_events() {
    let fx = FacadeFixture::new();
    let parent = fx.start();
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            readonly_request(&parent.session_id, "scan docs"),
        )
        .unwrap();
    fx.facade.cancel_subagent(&child.subagent_id).unwrap();
    let parent_stream = fx.facade.stream_agent_events(&parent.session_id).unwrap();
    let child_stream = fx
        .facade
        .stream_subagent_events(&child.subagent_id)
        .unwrap();
    assert!(contains_event_type(
        &parent_stream.jsonl,
        "subagent.cancelled"
    ));
    assert!(contains_event_type(
        &child_stream.jsonl,
        "subagent.cancelled"
    ));
}

#[test]
fn cancel_subagent_rejects_unknown_child() {
    let fx = FacadeFixture::new();
    assert!(fx.facade.cancel_subagent("missing").is_err());
}

#[test]
fn summarize_subagent_sets_completed_summary() {
    let fx = FacadeFixture::new();
    let parent = fx.start();
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            readonly_request(&parent.session_id, "scan docs"),
        )
        .unwrap();
    let summary = fx.facade.summarize_subagent(&child.subagent_id).unwrap();
    assert_eq!(summary.status, SubagentStatus::Completed);
    assert!(summary.summary.contains("completed"));
}

#[test]
fn summarize_subagent_is_idempotent() {
    let fx = FacadeFixture::new();
    let parent = fx.start();
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            readonly_request(&parent.session_id, "scan docs"),
        )
        .unwrap();
    let first = fx.facade.summarize_subagent(&child.subagent_id).unwrap();
    let second = fx.facade.summarize_subagent(&child.subagent_id).unwrap();
    assert_eq!(first, second);
}

#[test]
fn stream_subagent_events_rejects_unknown_child() {
    let fx = FacadeFixture::new();
    assert!(fx.facade.stream_subagent_events("missing").is_err());
}

#[test]
fn spawn_worker_requires_isolated_write_scope() {
    let fx = FacadeFixture::new();
    let parent = fx.start();
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            worker_request(&parent.session_id, "edit src/lib.rs", "src/lib.rs"),
        )
        .unwrap();
    assert_eq!(child.agent_type, SubagentType::Worker);
    assert_eq!(child.write_scope, vec!["src/lib.rs".to_string()]);
}

#[test]
fn spawn_worker_rejects_readonly_request_shape() {
    let fx = FacadeFixture::new();
    let parent = fx.start();
    let request = SubagentRequest::readonly(
        &parent.session_id,
        SubagentType::Worker,
        "edit",
        NativeModelFamily::DeepSeek,
    );
    assert!(fx
        .facade
        .spawn_subagent(&parent.session_id, request)
        .is_err());
}

#[test]
fn execute_subagent_tool_rejects_unknown_child() {
    let fx = FacadeFixture::new();
    assert!(fx
        .facade
        .execute_subagent_tool(
            "missing",
            "tc",
            "file.read",
            ToolExecutionArgs::default(),
            None
        )
        .is_err());
}

#[test]
fn execute_subagent_tool_enforces_allowlist() {
    let fx = FacadeFixture::new();
    let parent = fx.start();
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            readonly_request(&parent.session_id, "scan docs"),
        )
        .unwrap();
    assert!(fx
        .facade
        .execute_subagent_tool(
            &child.subagent_id,
            "tc",
            "file.write",
            ToolExecutionArgs {
                path: Some("x.txt".to_string()),
                content: Some("x".to_string()),
                ..ToolExecutionArgs::default()
            },
            Some(PermissionDecisionKind::AllowOnce),
        )
        .is_err());
}

#[test]
fn execute_subagent_tool_rejects_cancelled_child() {
    let fx = FacadeFixture::new();
    let parent = fx.start();
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            readonly_request(&parent.session_id, "scan docs"),
        )
        .unwrap();
    fx.facade.cancel_subagent(&child.subagent_id).unwrap();
    assert!(fx
        .facade
        .execute_subagent_tool(
            &child.subagent_id,
            "tc",
            "file.read",
            ToolExecutionArgs {
                path: Some("README.md".to_string()),
                ..ToolExecutionArgs::default()
            },
            None,
        )
        .is_err());
}

#[test]
fn execute_subagent_tool_records_readonly_result() {
    let fx = FacadeFixture::new();
    fx.write_workspace_file("README.md", "hello");
    let parent = fx.start();
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            readonly_request(&parent.session_id, "scan docs"),
        )
        .unwrap();
    let result = fx
        .facade
        .execute_subagent_tool(
            &child.subagent_id,
            "tc_read",
            "file.read",
            ToolExecutionArgs {
                path: Some("README.md".to_string()),
                ..ToolExecutionArgs::default()
            },
            None,
        )
        .unwrap();
    assert!(result.ok);
    let stream = fx
        .facade
        .stream_subagent_events(&child.subagent_id)
        .unwrap();
    assert!(contains_event_type(
        &stream.jsonl,
        "subagent.tool_completed"
    ));
}

#[test]
fn run_subagent_task_completes_readonly_child() {
    let fx = FacadeFixture::new();
    fx.write_workspace_file("README.md", "needle");
    let parent = fx.start_with(
        researchcode_runtime::runtime_facade::RuntimeModelMode::DeepSeek,
        AutonomyMode::Conservative,
    );
    let child = fx
        .facade
        .spawn_subagent(
            &parent.session_id,
            readonly_request(&parent.session_id, "find needle"),
        )
        .unwrap();
    let summary = fx
        .facade
        .run_subagent_task(&child.subagent_id, "find needle in README.md")
        .unwrap();
    assert_eq!(summary.status, SubagentStatus::Completed);
    assert!(!summary.evidence_refs.is_empty());
}
