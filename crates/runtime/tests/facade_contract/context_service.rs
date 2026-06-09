use super::support::*;
use researchcode_kernel::context::ContextItemKind;
use researchcode_runtime::runtime_facade::{AutonomyMode, RuntimeModelMode};
use researchcode_runtime::tool_execution::ToolExecutionArgs;

#[test]
fn build_context_bundle_rejects_unknown_session() {
    let fx = FacadeFixture::new();
    assert!(fx.facade.build_context_bundle("missing").is_err());
}

#[test]
fn build_context_bundle_uses_deepseek_model_family() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let bundle = fx.facade.build_context_bundle(&handle.session_id).unwrap();
    assert_eq!(bundle.model_family, "deepseek");
}

#[test]
fn build_context_bundle_uses_qwen_model_family() {
    let fx = FacadeFixture::new();
    let handle = fx.start_with(RuntimeModelMode::Qwen, AutonomyMode::Conservative);
    let bundle = fx.facade.build_context_bundle(&handle.session_id).unwrap();
    assert_eq!(bundle.model_family, "qwen");
}

#[test]
fn build_context_bundle_includes_user_task() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let bundle = fx.facade.build_context_bundle(&handle.session_id).unwrap();
    assert!(bundle
        .items
        .iter()
        .any(|item| item.kind == ContextItemKind::UserTask));
}

#[test]
fn build_context_bundle_reads_project_instructions() {
    let fx = FacadeFixture::new();
    fx.write_workspace_file("AGENTS.md", "project instructions from contract");
    let handle = fx.start();
    let bundle = fx.facade.build_context_bundle(&handle.session_id).unwrap();
    assert!(bundle.items.iter().any(|item| item.source == "AGENTS.md"
        && item.content.contains("project instructions from contract")));
}

#[test]
fn build_context_bundle_includes_git_status_item() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let bundle = fx.facade.build_context_bundle(&handle.session_id).unwrap();
    assert!(bundle
        .items
        .iter()
        .any(|item| item.source.starts_with("git.")));
}

#[test]
fn build_context_bundle_includes_session_memory_after_user_message() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    fx.facade
        .submit_user_message(&handle.session_id, "remember this contract note")
        .unwrap();
    let bundle = fx.facade.build_context_bundle(&handle.session_id).unwrap();
    assert!(bundle.items.iter().any(|item| {
        item.source == "runtime.session_memory" && item.content.contains("contract note")
    }));
}

#[test]
fn build_context_bundle_includes_file_state_after_file_read() {
    let fx = FacadeFixture::new();
    fx.write_workspace_file("src/lib.rs", "pub fn answer() {}\n");
    let handle = fx.start();
    fx.facade
        .execute_session_tool(
            &handle.session_id,
            "read1",
            "file.read",
            ToolExecutionArgs {
                path: Some("src/lib.rs".to_string()),
                ..ToolExecutionArgs::default()
            },
        )
        .unwrap();
    let bundle = fx.facade.build_context_bundle(&handle.session_id).unwrap();
    assert!(bundle
        .items
        .iter()
        .any(|item| item.source.contains("file_state:src/lib.rs")));
}

#[test]
fn build_context_bundle_includes_path_correction_after_missing_file() {
    let fx = FacadeFixture::new();
    fx.write_workspace_file("real.txt", "real");
    let handle = fx.start();
    let _ = fx.facade.execute_session_tool(
        &handle.session_id,
        "read_missing",
        "file.read",
        ToolExecutionArgs {
            path: Some("missing.txt".to_string()),
            ..ToolExecutionArgs::default()
        },
    );
    let bundle = fx.facade.build_context_bundle(&handle.session_id).unwrap();
    assert!(bundle
        .items
        .iter()
        .any(|item| item.source.contains("path_correction:")
            || item.content.contains("path correction")));
}

#[test]
fn conversation_history_starts_as_json_array() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let json = fx
        .facade
        .conversation_history_openai_json(&handle.session_id)
        .unwrap();
    assert!(json.trim_start().starts_with('['));
}

#[test]
fn conversation_history_records_user_message_projection() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    fx.facade
        .submit_user_message(&handle.session_id, "history projection contract")
        .unwrap();
    let json = fx
        .facade
        .conversation_history_openai_json(&handle.session_id)
        .unwrap();
    assert!(json.contains("history projection contract"));
}

#[test]
fn build_context_bundle_respects_context_budget_ceiling() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let bundle = fx.facade.build_context_bundle(&handle.session_id).unwrap();
    assert!(bundle.token_estimate() <= bundle.max_context_tokens);
}
