use super::support::*;
use researchcode_kernel::model::NativeModelFamily;
use researchcode_runtime::event_log::EventLog;
use researchcode_runtime::runtime_facade::{AutonomyMode, RuntimeModelMode};
use std::fs;

#[test]
fn facade_new_exposes_configured_workspace_root() {
    let fx = FacadeFixture::new();
    assert_eq!(fx.facade.workspace_root(), fx.workspace.as_path());
}

#[test]
fn start_session_uses_default_workspace_when_none() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    assert_eq!(handle.workspace_root, fx.workspace);
}

#[test]
fn start_session_accepts_explicit_workspace() {
    let fx = FacadeFixture::new();
    let explicit = fx.workspace.join("explicit");
    fs::create_dir_all(&explicit).unwrap();
    let handle = fx
        .facade
        .start_session(
            Some(explicit.clone()),
            RuntimeModelMode::DeepSeek,
            AutonomyMode::Conservative,
        )
        .unwrap();
    assert_eq!(handle.workspace_root, explicit);
}

#[test]
fn start_session_places_artifacts_under_session_directory() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    assert_eq!(handle.artifact_root, fx.artifacts.join(&handle.session_id));
}

#[test]
fn start_session_preserves_model_and_autonomy_modes() {
    let fx = FacadeFixture::new();
    let handle = fx.start_with(RuntimeModelMode::Qwen, AutonomyMode::FastAuto);
    assert_eq!(handle.model_mode, RuntimeModelMode::Qwen);
    assert_eq!(handle.autonomy_mode, AutonomyMode::FastAuto);
}

#[test]
fn start_session_generates_distinct_session_ids() {
    let fx = FacadeFixture::new();
    let first = fx.start();
    let second = fx.start();
    assert_ne!(first.session_id, second.session_id);
}

#[test]
fn start_session_generates_distinct_task_ids() {
    let fx = FacadeFixture::new();
    let first = fx.start();
    let second = fx.start();
    assert_ne!(first.task_id, second.task_id);
}

#[test]
fn get_session_snapshot_rejects_unknown_session() {
    let fx = FacadeFixture::new();
    assert!(fx.facade.get_session_snapshot("missing").is_err());
}

#[test]
fn get_session_snapshot_reports_started_session_state() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let snapshot = fx.snapshot(&handle.session_id);
    assert_eq!(snapshot.session_id, handle.session_id);
    assert_eq!(snapshot.model_mode, RuntimeModelMode::DeepSeek);
    assert!(snapshot.event_count >= 4);
}

#[test]
fn get_session_snapshot_starts_with_empty_approval_queue() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let snapshot = fx.snapshot(&handle.session_id);
    assert_eq!(snapshot.pending_permission_count, 0);
    assert_eq!(snapshot.pending_plan_approval_count, 0);
}

#[test]
fn stream_agent_events_rejects_unknown_session() {
    let fx = FacadeFixture::new();
    assert!(fx.facade.stream_agent_events("missing").is_err());
}

#[test]
fn stream_agent_events_returns_jsonl_for_started_session() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let stream = fx.facade.stream_agent_events(&handle.session_id).unwrap();
    assert_eq!(stream.session_id, handle.session_id);
    assert!(stream.jsonl.contains("session.created"));
}

#[test]
fn stream_agent_events_since_zero_returns_initial_events() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let delta = fx
        .facade
        .stream_agent_events_since(&handle.session_id, 0, None)
        .unwrap();
    assert_eq!(delta.from_cursor, 0);
    assert_eq!(
        delta.events.len(),
        fx.snapshot(&handle.session_id).event_count
    );
}

#[test]
fn stream_agent_events_since_limit_advances_cursor() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let delta = fx
        .facade
        .stream_agent_events_since(&handle.session_id, 0, Some(1))
        .unwrap();
    assert_eq!(delta.next_cursor, 1);
    assert_eq!(delta.events.len(), 1);
    assert!(delta.has_more);
}

#[test]
fn stream_agent_events_since_at_end_returns_empty_delta() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let count = fx.snapshot(&handle.session_id).event_count;
    let delta = fx
        .facade
        .stream_agent_events_since(&handle.session_id, count, Some(10))
        .unwrap();
    assert!(delta.events.is_empty());
    assert_eq!(delta.jsonl, "");
    assert!(!delta.has_more);
}

#[test]
fn stream_agent_events_since_past_end_resyncs_to_tail() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let count = fx.snapshot(&handle.session_id).event_count;
    let delta = fx
        .facade
        .stream_agent_events_since(&handle.session_id, count + 1, None)
        .unwrap();
    assert_eq!(delta.from_cursor, count);
    assert_eq!(delta.next_cursor, count);
    assert!(delta.events.is_empty());
    assert_eq!(delta.jsonl, "");
    assert!(!delta.has_more);
}

#[test]
fn submit_user_message_rejects_empty_text() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    assert!(fx
        .facade
        .submit_user_message(&handle.session_id, "  ")
        .is_err());
}

#[test]
fn submit_user_message_rejects_unknown_session() {
    let fx = FacadeFixture::new();
    assert!(fx.facade.submit_user_message("missing", "hello").is_err());
}

#[test]
fn submit_user_message_increments_event_count() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let before = fx.snapshot(&handle.session_id).event_count;
    fx.facade
        .submit_user_message(&handle.session_id, "hello from contract")
        .unwrap();
    assert!(fx.snapshot(&handle.session_id).event_count > before);
}

#[test]
fn submit_user_message_is_visible_in_event_stream() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    fx.facade
        .submit_user_message(&handle.session_id, "visible contract message")
        .unwrap();
    let stream = fx.facade.stream_agent_events(&handle.session_id).unwrap();
    assert!(stream.jsonl.contains("visible contract message"));
}

#[test]
fn export_events_writes_importable_jsonl() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let path = fx.artifacts.join("session.jsonl");
    fx.facade.export_events(&handle.session_id, &path).unwrap();
    let log = EventLog::read_jsonl(&path).unwrap();
    assert_eq!(log.len(), fx.snapshot(&handle.session_id).event_count);
}

#[test]
fn close_session_removes_session_record() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    fx.facade.close_session(&handle.session_id).unwrap();
    assert!(fx.facade.get_session_snapshot(&handle.session_id).is_err());
}

#[test]
fn close_session_is_idempotent_for_unknown_session() {
    let fx = FacadeFixture::new();
    fx.facade.close_session("missing").unwrap();
}

#[test]
fn resume_session_from_eventlog_restores_session_id() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let path = fx.artifacts.join("resume.jsonl");
    fx.facade.export_events(&handle.session_id, &path).unwrap();
    fx.facade.close_session(&handle.session_id).unwrap();
    let resumed = fx.facade.resume_session_from_eventlog(&path).unwrap();
    assert_eq!(resumed.session_id, handle.session_id);
}

#[test]
fn conversation_history_unknown_session_errors() {
    let fx = FacadeFixture::new();
    assert!(fx
        .facade
        .conversation_history_openai_json("missing")
        .is_err());
}

#[test]
fn runtime_model_mode_contract_values_are_stable() {
    assert_eq!(
        RuntimeModelMode::DeepSeek.family(),
        NativeModelFamily::DeepSeek
    );
    assert_eq!(RuntimeModelMode::Qwen.family(), NativeModelFamily::Qwen);
    assert_eq!(RuntimeModelMode::DeepSeek.as_str(), "deepseek");
    assert_eq!(RuntimeModelMode::Qwen.as_str(), "qwen");
}
