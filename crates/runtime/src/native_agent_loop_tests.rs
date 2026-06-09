use super::*;
use crate::agent_kernel::{
    requested_line_count_policy, ContinuationView, RequestedLineCountPolicy, TurnRoute,
};
use crate::native_profile::deepseek::adaptation::DualProtocolFallback;
use crate::native_profile::deepseek::stream_processor::StreamProcessor;
use crate::tcml::{
    build_tool_manifest_for_context, ParsedToolArguments, ToolManifestBuildContext,
    ToolManifestExposure,
};

#[test]
fn native_loop_user_prompt_event_strips_runtime_context() {
    assert_eq!(
        native_loop_user_prompt_for_event("Read README\n\n# Runtime Context\nrepo map"),
        "Read README"
    );
    assert_eq!(
            native_loop_user_prompt_for_event(
                "The user sent a greeting or simple social opener. Respond naturally.\n\nUser: hi\n\n# Runtime Context\nrepo map"
            ),
            "hi"
        );
}

#[test]
fn native_loop_intent_detection_ignores_runtime_context_memory() {
    let prompt = "请使用写入工具在文件夹内部写入一个30行左右的html小程序\n\n# Runtime Context\n上一轮：请测试一下你拥有的所有工具";
    assert!(native_prompt_wants_file_generation(prompt));
    assert!(!native_prompt_wants_tool_inventory(prompt));
}

#[test]
fn native_loop_tool_acceptance_write_request_is_not_inventory() {
    let prompt = "你现在要在当前工作区的 VoiceNote 项目里做一次工具链验收。必须严格按这个顺序调用工具：第一步调用 plan.write 写入测试规划；第二步调用 file.write 创建 VoiceNote/Tests/VoiceNoteTests/Smoke.swift。";
    assert!(native_prompt_wants_file_generation(prompt));
    assert!(!native_prompt_wants_tool_inventory(prompt));
}

#[test]
fn executable_dsml_detector_ignores_fenced_discussion() {
    let discussion = r#"Here is an example, do not execute it:
```xml
<｜｜DSML｜｜tool_calls>
<｜｜DSML｜｜invoke name="file_read">
<｜｜DSML｜｜parameter name="path" string="true">README.md</｜｜DSML｜｜parameter>
</｜｜DSML｜｜invoke>
</｜｜DSML｜｜tool_calls>
```"#;
    assert!(!contains_executable_dsml_markup(discussion));
    assert!(!contains_executable_dsml_markup(
        "The string <｜｜DSML｜｜tool_calls> is part of the protocol name."
    ));
    assert!(contains_executable_dsml_markup(
            "<｜｜DSML｜｜tool_calls>\n<｜｜DSML｜｜invoke name=\"file_read\">\n<｜｜DSML｜｜parameter name=\"path\" string=\"true\">README.md</｜｜DSML｜｜parameter>\n</｜｜DSML｜｜invoke>\n</｜｜DSML｜｜tool_calls>"
        ));
}

#[test]
fn native_loop_fastauto_write_prompt_injects_turn_directive() {
    let prompt = native_loop_prompt_with_turn_directives(
        "请使用写入工具在文件夹内部写入一个30行左右的html小程序",
        &NativeAgentToolExposure::FastAutoWrite,
    );
    assert!(prompt.contains("# Runtime Write Directive"));
    assert!(prompt.contains("file_write"));
    assert!(prompt.contains("path"));
    assert!(prompt.contains("content"));

    let read_only = native_loop_prompt_with_turn_directives(
        "请读取 README.md",
        &NativeAgentToolExposure::ReadOnly,
    );
    assert!(!read_only.contains("# Runtime Write Directive"));
}

#[test]
fn native_loop_code_edit_prompt_allows_precise_reads_then_requires_edits() {
    let prompt = native_loop_system_prompt(
        &NativeModelFamily::DeepSeek,
        "openai_compatible",
        &NativeAgentToolExposure::CodeEdit,
        None,
        None,
    );
    assert!(prompt.contains("implementation/editing turn"));
    assert!(prompt.contains("use file.read or directory tools only for precise new targets"));
    assert!(prompt.contains("then move to file.write"));
    assert!(!prompt.contains("Do not call file.read"));
}

#[test]
fn read_only_exposure_keeps_streamed_write_tools_routed_to_permission_gate() {
    assert!(is_stream_candidate_provider_tool(
        "file_write",
        &NativeAgentToolExposure::ReadOnly
    ));
    assert!(is_stream_candidate_provider_tool(
        "file.write",
        &NativeAgentToolExposure::ReadOnly
    ));
    assert!(is_stream_executable_tool(
        "file.write",
        &NativeAgentToolExposure::ReadOnly
    ));
    assert!(is_stream_executable_tool(
        "file.edit",
        &NativeAgentToolExposure::ReadOnly
    ));
    assert!(is_stream_executable_tool(
        "file.multi_edit",
        &NativeAgentToolExposure::ReadOnly
    ));
}

fn assert_route_maps_to_exposure(
    route: TurnRoute,
    native_exposure: NativeAgentToolExposure,
    manifest_exposure: ToolManifestExposure,
) {
    assert_eq!(
        native_agent_tool_exposure_for_route(&route),
        native_exposure
    );
    assert_eq!(
        native_loop_manifest_exposure(&NativeAgentToolExposure::ReadOnly, &route),
        manifest_exposure
    );
}

#[test]
fn route_project_status_maps_to_read_only_exposure() {
    assert_route_maps_to_exposure(
        TurnRoute::ProjectStatus,
        NativeAgentToolExposure::ReadOnly,
        ToolManifestExposure::ReadOnly,
    );
}

#[test]
fn route_direct_answer_maps_to_read_only_exposure() {
    assert_route_maps_to_exposure(
        TurnRoute::DirectAnswer,
        NativeAgentToolExposure::ReadOnly,
        ToolManifestExposure::ReadOnly,
    );
}

#[test]
fn route_read_only_explore_maps_to_read_only_exposure() {
    assert_route_maps_to_exposure(
        TurnRoute::ReadOnlyExplore,
        NativeAgentToolExposure::ReadOnly,
        ToolManifestExposure::ReadOnly,
    );
}

#[test]
fn route_code_edit_maps_to_code_edit_exposure() {
    assert_route_maps_to_exposure(
        TurnRoute::CodeEdit,
        NativeAgentToolExposure::CodeEdit,
        ToolManifestExposure::CodeEdit,
    );
}

#[test]
fn route_debug_failure_maps_to_fast_auto_write_exposure() {
    assert_route_maps_to_exposure(
        TurnRoute::DebugFailure,
        NativeAgentToolExposure::FastAutoWrite,
        ToolManifestExposure::FastAutoWrite,
    );
}

#[test]
fn route_run_tests_maps_to_fast_auto_write_exposure() {
    assert_route_maps_to_exposure(
        TurnRoute::RunTests,
        NativeAgentToolExposure::FastAutoWrite,
        ToolManifestExposure::FastAutoWrite,
    );
}

#[test]
fn route_long_horizon_maps_to_fast_auto_write_exposure() {
    assert_route_maps_to_exposure(
        TurnRoute::LongHorizonTask,
        NativeAgentToolExposure::FastAutoWrite,
        ToolManifestExposure::FastAutoWrite,
    );
}

#[test]
fn route_review_maps_to_read_only_exposure() {
    assert_route_maps_to_exposure(
        TurnRoute::Review,
        NativeAgentToolExposure::ReadOnly,
        ToolManifestExposure::ReadOnly,
    );
}

#[test]
fn final_answer_tool_is_not_exposed_to_native_loop() {
    assert!(researchcode_kernel::tool::find_tool_spec("agent.final_answer").is_none());

    let read_only_manifest = build_tool_manifest_for_context(&ToolManifestBuildContext {
        family: NativeModelFamily::DeepSeek,
        protocol: "openai_compatible".to_string(),
        exposure: ToolManifestExposure::ReadOnly,
        workflow_state: "executing".to_string(),
        permission_summary: "default".to_string(),
        task_contract_mode: "default".to_string(),
    });
    assert!(!read_only_manifest
        .manifest
        .canonical_tool_ids
        .contains(&"agent.final_answer".to_string()));
    assert!(!read_only_manifest
        .tool_schema_json
        .contains("agent_final_answer"));

    let code_edit_manifest = build_tool_manifest_for_context(&ToolManifestBuildContext {
        family: NativeModelFamily::DeepSeek,
        protocol: "openai_compatible".to_string(),
        exposure: ToolManifestExposure::CodeEdit,
        workflow_state: "executing".to_string(),
        permission_summary: "default".to_string(),
        task_contract_mode: "default".to_string(),
    });
    assert!(!code_edit_manifest
        .manifest
        .canonical_tool_ids
        .contains(&"agent.final_answer".to_string()));
    assert!(!code_edit_manifest
        .tool_schema_json
        .contains("agent_final_answer"));
}

#[test]
fn native_loop_manifest_tool_set_is_stable_across_routes() {
    let endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    let read_only_manifest = build_native_loop_tool_manifest(
        &NativeAgentToolExposure::ReadOnly,
        &TurnRoute::ReadOnlyExplore,
        &endpoint,
        "reading",
    );
    let code_edit_manifest = build_native_loop_tool_manifest(
        &NativeAgentToolExposure::CodeEdit,
        &TurnRoute::CodeEdit,
        &endpoint,
        "editing",
    );

    assert_eq!(
        read_only_manifest.manifest.canonical_tool_ids,
        code_edit_manifest.manifest.canonical_tool_ids
    );
    assert!(read_only_manifest
        .manifest
        .canonical_tool_ids
        .contains(&"file.write".to_string()));
    assert!(read_only_manifest
        .manifest
        .canonical_tool_ids
        .contains(&"shell.command".to_string()));
}

#[test]
fn structured_stop_does_not_synthesize_assistant_message() {
    let mut session = AgentSession::new("proj", "sess_structured_stop", "task").unwrap();
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .unwrap();
    let mut emitted = 0usize;
    let mut sink: Option<&mut dyn FnMut(&str)> = None;
    stop_native_loop_with_structured_failure(
        &mut session,
        "task",
        &Vec::new(),
        "empty_visible_response",
        "no_visible_answer",
        &mut emitted,
        &mut sink,
    )
    .unwrap();
    let jsonl = session.export_events_jsonl();
    assert!(jsonl.contains("\"event_type\":\"agent.loop_stopped\""));
    assert!(jsonl.contains("\"status\":\"failed\""));
    assert!(jsonl.contains("\"category\":\"no_visible_answer\""));
    assert!(!jsonl.contains("\"event_type\":\"assistant.message\""));
    assert!(!jsonl.contains("visible_finalizer"));
}

#[test]
fn native_loop_state_records_only_one_terminal_reason() {
    let mut session = AgentSession::new("proj", "sess_single_terminal", "task").unwrap();
    let mut loop_state = NativeLoopState::new("turn_single_terminal");
    loop_state.begin_iteration(3);
    loop_state
        .record_terminal(
            &mut session,
            NativeAgentLoopStatus::Blocked,
            "max_iterations",
            "turn_budget",
        )
        .unwrap();
    loop_state
        .record_terminal(
            &mut session,
            NativeAgentLoopStatus::Failed,
            "late_failure",
            "provider_failure",
        )
        .unwrap();

    let jsonl = session.export_events_jsonl();
    assert_eq!(event_count(&jsonl, "agent.loop_state.terminal"), 1);
    assert_eq!(
        event_count(&jsonl, "agent.loop_state.terminal_duplicate_suppressed"),
        1
    );
    assert_eq!(
        event_payload_string(&jsonl, "agent.loop_state.terminal", "reason"),
        "max_iterations"
    );
    assert_eq!(
        event_payload_string(&jsonl, "agent.loop_state.terminal", "loop_id"),
        "turn_single_terminal"
    );
    assert_eq!(
        event_payload_string(
            &jsonl,
            "agent.loop_state.terminal_duplicate_suppressed",
            "requested_reason",
        ),
        "late_failure"
    );
}

#[test]
fn native_permission_resolver_requests_sensitive_patch_even_in_autoallow() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("researchcode-native-permission-sensitive-{nonce}"));
    fs::create_dir_all(root.join("artifacts/session")).unwrap();
    let artifact_store = ArtifactStore::new(root.join("artifacts/session"));
    let mut session = AgentSession::new("proj", "sess", "task").unwrap();
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .unwrap();
    let args = ToolExecutionArgs {
        path: Some(".env".to_string()),
        old_string: Some("A=1".to_string()),
        new_string: Some("A=2".to_string()),
        ..ToolExecutionArgs::default()
    };
    let mut permission_gate = native_loop_permission_gate(
        &artifact_store,
        &root,
        PermissionMode::BypassPermissions,
        "sess",
    );

    let decision = tool_permission_decision(
        &mut session,
        &artifact_store,
        &mut permission_gate,
        &PermissionMode::BypassPermissions,
        "patch.apply",
        "perm_sensitive_patch",
        PermissionRequestType::FileWrite,
        &args,
        &[],
    )
    .unwrap();

    assert!(matches!(decision, NativePermissionDecisionOutcome::Pending));
    assert_eq!(session.state(), AgentState::WaitingForToolApproval);
    let jsonl = session.export_events_jsonl();
    assert!(jsonl.contains("\"permission_id\":\"perm_sensitive_patch\""));
    assert!(jsonl.contains("\"event_type\":\"permission.decision.recorded\""));
    assert!(jsonl.contains("\"tool_id\":\"patch.apply\""));
    assert_eq!(
        jsonl
            .matches("\"event_type\":\"permission.decision.recorded\"")
            .count(),
        1
    );
    assert!(!jsonl.contains("\"permission.decided\""));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_permission_denial_becomes_model_readable_outcome() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-permission-denied-outcome-{nonce}"
    ));
    fs::create_dir_all(root.join("artifacts/session")).unwrap();
    let artifact_store = ArtifactStore::new(root.join("artifacts/session"));
    let mut session = AgentSession::new("proj", "sess", "task").unwrap();
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .unwrap();
    let args = ToolExecutionArgs {
        path: Some("notes.txt".to_string()),
        content: Some("hello\n".to_string()),
        ..ToolExecutionArgs::default()
    };
    let mut permission_gate =
        native_loop_permission_gate(&artifact_store, &root, PermissionMode::DontAsk, "sess");

    let decision = tool_permission_decision(
        &mut session,
        &artifact_store,
        &mut permission_gate,
        &PermissionMode::DontAsk,
        "file.write",
        "perm_dont_ask_write",
        PermissionRequestType::FileWrite,
        &args,
        &[],
    )
    .unwrap();

    assert!(matches!(
        decision,
        NativePermissionDecisionOutcome::Denied(ModelReadableToolError {
            error_code,
            retryable: false,
            ..
        }) if error_code == "PERMISSION_DENIED"
    ));
    assert_eq!(session.state(), AgentState::Executing);
    let jsonl = session.export_events_jsonl();
    assert!(jsonl.contains("\"event_type\":\"permission.decision.recorded\""));
    assert!(jsonl.contains("\"tool_id\":\"file.write\""));
    assert!(jsonl.contains("\"mode\":\"DontAsk\""));
    assert_eq!(
        jsonl
            .matches("\"event_type\":\"permission.decision.recorded\"")
            .count(),
        1
    );
    assert!(!jsonl.contains("\"permission_id\":\"perm_dont_ask_write\""));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_permission_resolver_requests_sensitive_file_write_and_edit_even_in_autoallow() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-sensitive-file-write-edit-{nonce}"
    ));
    fs::create_dir_all(root.join("artifacts/session")).unwrap();
    let artifact_store = ArtifactStore::new(root.join("artifacts/session"));
    for (tool_id, args) in [
        (
            "file.write",
            ToolExecutionArgs {
                path: Some(".env".to_string()),
                content: Some("TOKEN=new\n".to_string()),
                ..ToolExecutionArgs::default()
            },
        ),
        (
            "file.edit",
            ToolExecutionArgs {
                path: Some(".ssh/id_rsa".to_string()),
                old_string: Some("old".to_string()),
                new_string: Some("new".to_string()),
                ..ToolExecutionArgs::default()
            },
        ),
    ] {
        let mut session = AgentSession::new("proj", &format!("sess_{tool_id}"), "task").unwrap();
        session
            .transition_to(AgentState::Planning)
            .and_then(|_| session.transition_to(AgentState::RetrievingContext))
            .and_then(|_| session.transition_to(AgentState::Executing))
            .unwrap();
        let mut permission_gate = native_loop_permission_gate(
            &artifact_store,
            &root,
            PermissionMode::BypassPermissions,
            &format!("sess_{tool_id}"),
        );
        let decision = tool_permission_decision(
            &mut session,
            &artifact_store,
            &mut permission_gate,
            &PermissionMode::BypassPermissions,
            tool_id,
            &format!("perm_sensitive_{}", tool_id.replace('.', "_")),
            PermissionRequestType::FileWrite,
            &args,
            &[],
        )
        .unwrap();

        assert!(
            matches!(decision, NativePermissionDecisionOutcome::Pending),
            "{tool_id} should request approval for dangerous path"
        );
        assert_eq!(session.state(), AgentState::WaitingForToolApproval);
        assert!(session
            .export_events_jsonl()
            .contains("permission.requested"));
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_execute_patch_requests_sensitive_path_before_reading_file() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-execute-sensitive-patch-{nonce}"
    ));
    fs::create_dir_all(root.join("artifacts/session")).unwrap();
    let artifact_store = ArtifactStore::new(root.join("artifacts/session"));
    let mut session = AgentSession::new("proj", "sess", "task").unwrap();
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .unwrap();
    let arguments = ParsedToolArguments {
        path: Some(".env".to_string()),
        old_string: Some("TOKEN=old".to_string()),
        new_string: Some("TOKEN=new".to_string()),
        ..ParsedToolArguments::default()
    };
    let mut permission_gate = native_loop_permission_gate(
        &artifact_store,
        &root,
        PermissionMode::BypassPermissions,
        "sess",
    );

    let pending = execute_patch(
        &mut session,
        &artifact_store,
        &mut permission_gate,
        &root,
        7,
        &arguments,
        &PermissionMode::BypassPermissions,
        &[],
        None,
    )
    .unwrap();

    assert!(matches!(
        pending,
        Some(PendingNativeToolExecution {
            tool_id,
            request_type: PermissionRequestType::FileWrite,
            ..
        }) if tool_id == "patch.apply"
    ));
    let events = session.export_events_jsonl();
    assert!(events.contains("\"permission.requested\""));
    assert!(!events.contains("\"patch.proposal_created\""));
    assert!(!events.contains("\"permission.decided\""));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_loop_tool_inventory_prompt_injects_bounded_directive() {
    let prompt = native_loop_prompt_with_turn_directives(
        "请测试一下你拥有的所有工具，不依赖原始记忆",
        &NativeAgentToolExposure::ReadOnly,
    );
    assert!(prompt.contains("# Runtime Tool Inventory Directive"));
    assert!(prompt.contains("Do not guess hard-coded paths"));
    assert!(prompt.contains("After the first useful tool observation"));
}

#[test]
fn native_agent_loop_v2_rebuilds_request_after_preflight_compaction() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-compact-rebuild-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: r#"data: {"choices":[{"delta":{"content":"Compacted context accepted."}}]}
data: [DONE]"#
            .to_string(),
    }]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_compact_rebuild".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: format!(
                "Summarize this oversized context.\n\n# Runtime Context\n{}",
                "x".repeat(780_000)
            ),
            max_tokens: 1024,
            max_iterations: 2,
            max_tool_calls: 4,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(
        result.status,
        NativeAgentLoopStatus::Completed,
        "{}",
        result.event_jsonl
    );
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 1);
    assert!(result.event_jsonl.contains("context.compaction.started"));
    assert!(result.event_jsonl.contains("context.compaction.completed"));
    assert!(result
        .event_jsonl
        .contains("\"prompt_tokens_after_injection\":"));
    assert!(result
        .event_jsonl
        .contains("rebuild_initial_from_compacted_context"));
    assert!(result.event_jsonl.contains("compacted_initial"));
    assert!(result.event_jsonl.contains("Compacted context accepted"));
    let sent_requests = transport.sent_requests();
    assert_eq!(sent_requests.len(), 1);
    assert!(sent_requests[0]
        .body_json
        .contains("# Recent Turns (preserved verbatim)"));
    assert!(
        sent_requests[0].body_json.contains("file_read")
            || sent_requests[0].body_json.contains("file.read")
    );
}

#[test]
fn native_agent_loop_v2_long_context_compaction_drops_next_prompt_tokens() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-compact-token-drop-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: r#"data: {"choices":[{"delta":{"content":"Compacted context accepted."}}]}
data: [DONE]"#
            .to_string(),
    }]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_compact_token_drop".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: format!(
                "Continue this long session.\n\n# Runtime Context\n{}",
                "x".repeat(800_000)
            ),
            max_tokens: 1024,
            max_iterations: 2,
            max_tool_calls: 4,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);

    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(
        event_count(&result.event_jsonl, "context.compaction.started"),
        1
    );
    assert_eq!(
        event_count(&result.event_jsonl, "context.compaction.projected"),
        1
    );
    assert_eq!(
        event_count(&result.event_jsonl, "context.compaction.completed"),
        1
    );
    assert_eq!(
        event_count(&result.event_jsonl, "context.compaction.skipped"),
        0
    );
    assert_eq!(event_count(&result.event_jsonl, "model.call_started"), 2);
    assert!(result.event_jsonl.contains("\"spine\""));
    assert!(result.event_jsonl.contains("\"spine_json\""));
    assert!(result.event_jsonl.contains("[pinned-context-spine]"));
    let spine_json = event_payload_value(
        &result.event_jsonl,
        "context.compaction.completed",
        "spine_json",
    );
    assert!(spine_json.is_object());
    assert!(spine_json
        .get("goal")
        .and_then(|value| value.as_str())
        .is_some());
    assert!(spine_json
        .get("confirmed_facts")
        .and_then(|value| value.as_array())
        .is_some());
    assert!(spine_json
        .get("observations")
        .and_then(|value| value.as_array())
        .is_some());
    let sent_requests = transport.sent_requests();
    assert_eq!(sent_requests.len(), 1);
    assert!(sent_requests[0]
        .body_json
        .contains("[pinned-context-spine]"));
    assert!(sent_requests[0]
        .body_json
        .contains("# Pinned Context Spine"));

    let completed_before = event_payload_u64(
        &result.event_jsonl,
        "context.compaction.completed",
        "token_estimate_before",
    );
    let completed_after = event_payload_u64(
        &result.event_jsonl,
        "context.compaction.completed",
        "prompt_tokens_after_injection",
    );
    let model_prompt_tokens = event_payload_u64_values(
        &result.event_jsonl,
        "model.call_started",
        "prompt_tokens_estimate",
    );
    let before = model_prompt_tokens[0];
    let after = model_prompt_tokens[1];

    assert!(
        before >= 200_000,
        "expected 200K-class fixture, got {before}"
    );
    assert!(completed_before >= before);
    assert_eq!(completed_after, after);
    assert!(
        after * 100 <= before * 70,
        "expected >=30% token drop, before={before}, after={after}"
    );
}

#[test]
fn native_agent_loop_v2_context_compaction_folds_reasoning_replay() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-reasoning-compact-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    let artifact_root = root.join("artifacts");
    let mut adaptation = DeepSeekAdaptationManager::new(NativeModelFamily::DeepSeek);
    adaptation.reasoning.inject(
        "sess_native_loop_v2_reasoning_compact",
        0,
        "assistant_reasoning_0",
        "raw hidden reasoning that should be folded during context compaction",
    );
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: r#"data: {"choices":[{"delta":{"content":"Reasoning replay compacted."}}]}
data: [DONE]"#
            .to_string(),
    }]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_reasoning_compact".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: format!(
                "Continue with folded reasoning.\n\n# Runtime Context\n{}",
                "x".repeat(780_000)
            ),
            max_tokens: 1024,
            max_iterations: 2,
            max_tool_calls: 4,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: Some(adaptation),
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);

    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(
        event_count(&result.event_jsonl, "deepseek.reasoning.compacted"),
        1
    );
    assert_eq!(
        event_payload_u64(
            &result.event_jsonl,
            "deepseek.reasoning.compacted",
            "preserved_reasoning_count"
        ),
        1
    );
    assert!(result
        .event_jsonl
        .contains("\"reason\":\"context_compaction\""));
}

fn event_count(jsonl: &str, event_type: &str) -> usize {
    jsonl
        .lines()
        .filter(|line| line.contains(&format!("\"event_type\":\"{event_type}\"")))
        .count()
}

fn event_payload_u64(jsonl: &str, event_type: &str, key: &str) -> u64 {
    let line = jsonl
        .lines()
        .find(|line| line.contains(&format!("\"event_type\":\"{event_type}\"")))
        .unwrap_or_else(|| panic!("missing event type {event_type}"));
    let value: serde_json::Value = serde_json::from_str(line).unwrap();
    value
        .get("payload")
        .and_then(|payload| payload.get(key))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_else(|| panic!("missing u64 payload field {key} for {event_type}"))
}

fn event_payload_string(jsonl: &str, event_type: &str, key: &str) -> String {
    let line = jsonl
        .lines()
        .find(|line| line.contains(&format!("\"event_type\":\"{event_type}\"")))
        .unwrap_or_else(|| panic!("missing event type {event_type}"));
    let value: serde_json::Value = serde_json::from_str(line).unwrap();
    value
        .get("payload")
        .and_then(|payload| payload.get(key))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("missing string payload field {key} for {event_type}"))
        .to_string()
}

fn event_payload_value(jsonl: &str, event_type: &str, key: &str) -> serde_json::Value {
    let line = jsonl
        .lines()
        .find(|line| line.contains(&format!("\"event_type\":\"{event_type}\"")))
        .unwrap_or_else(|| panic!("missing event type {event_type}"));
    let value: serde_json::Value = serde_json::from_str(line).unwrap();
    value
        .get("payload")
        .and_then(|payload| payload.get(key))
        .cloned()
        .unwrap_or_else(|| panic!("missing payload field {key} for {event_type}"))
}

fn event_payload_u64_values(jsonl: &str, event_type: &str, key: &str) -> Vec<u64> {
    jsonl
        .lines()
        .filter(|line| line.contains(&format!("\"event_type\":\"{event_type}\"")))
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            value
                .get("payload")
                .and_then(|payload| payload.get(key))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_else(|| panic!("missing u64 payload field {key} for {event_type}"))
        })
        .collect()
}

#[test]
fn native_agent_loop_v2_records_turn_route_classified_event() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-turn-route-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: r#"data: {"choices":[{"delta":{"content":"Route classified."}}]}
data: [DONE]"#
            .to_string(),
    }]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_turn_route".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "fix the parser bug".to_string(),
            max_tokens: 512,
            max_iterations: 1,
            max_tool_calls: 0,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"turn.route.classified\""));
    assert!(result
        .event_jsonl
        .contains("\"turn_id\":\"sess_native_loop_v2_turn_route:native_turn_"));
    assert!(result.event_jsonl.contains("\"route\":\"CodeEdit\""));
    assert!(!result.event_jsonl.contains("agent.turn_route.classified"));
}

#[test]
fn native_loop_fastauto_write_continuation_repairs_empty_write_args() {
    let batch = vec![(
        "toolu_write".to_string(),
        "file.write".to_string(),
        "{}".to_string(),
        crate::tool_execution::ToolExecutionResult {
            tool_call_id: "tool_write".to_string(),
            tool_id: "file.write".to_string(),
            ok: false,
            preview: "schema_validation_failed retryable=true".to_string(),
            detail_json: "{\"error_code\":\"SCHEMA_VALIDATION_FAILED\"}".to_string(),
            exit_code: None,
        },
    )];
    let hint = native_loop_continuation_hint(
        "请写入一个 html 小程序",
        &batch,
        &NativeAgentToolExposure::FastAutoWrite,
    );
    assert!(hint.contains("corrected write tool call"));
    assert!(hint.contains("path"));
    assert!(hint.contains("content"));
    assert!(hint
        .to_ascii_lowercase()
        .contains("do not list or read again"));
    assert!(hint.contains("multi-line"));
}

#[test]
fn native_loop_write_intent_continuation_moves_from_exploration_to_editing() {
    let batch = vec![(
        "toolu_list".to_string(),
        "file.list_directory".to_string(),
        "{\"path\":\"VoiceNote/Sources/Features\"}".to_string(),
        crate::tool_execution::ToolExecutionResult {
            tool_call_id: "tool_list".to_string(),
            tool_id: "file.list_directory".to_string(),
            ok: true,
            preview: "file.list_directory entries=4".to_string(),
            detail_json: "{\"ok\":true}".to_string(),
            exit_code: None,
        },
    )];
    let hint =
        native_loop_continuation_hint("可以开始写了", &batch, &NativeAgentToolExposure::CodeEdit);
    assert!(hint.contains("proceed with implementation"));
    assert!(hint.contains("file.write"));
    assert!(hint.contains("file.edit"));
    assert!(hint.contains("Do not call more read/list/search tools"));
}

#[test]
fn native_loop_tool_inventory_continuation_answers_after_observation() {
    let batch = vec![(
        "toolu_list".to_string(),
        "file.list_directory".to_string(),
        "{\"path\":\".\"}".to_string(),
        crate::tool_execution::ToolExecutionResult {
            tool_call_id: "tool_list".to_string(),
            tool_id: "file.list_directory".to_string(),
            ok: true,
            preview: "file.list_directory entries=3".to_string(),
            detail_json: "{\"ok\":true}".to_string(),
            exit_code: None,
        },
    )];
    let hint = native_loop_continuation_hint(
        "请测试一下你拥有的所有工具，不依赖原始记忆",
        &batch,
        &NativeAgentToolExposure::ReadOnly,
    );
    assert!(hint.contains("tool-inventory/test request"));
    assert!(hint.contains("Produce the final answer now"));
    assert!(hint.contains("do not call more tools"));
}

#[test]
fn native_loop_requested_line_count_policy_parses_chinese_and_english() {
    let chinese = requested_line_count_policy("请写入一个30行左右的html小程序").unwrap();
    assert_eq!(
        chinese,
        RequestedLineCountPolicy {
            target: 30,
            min: 19,
            max: 48
        }
    );
    let english = requested_line_count_policy("write about 12 lines of HTML").unwrap();
    assert_eq!(
        english,
        RequestedLineCountPolicy {
            target: 12,
            min: 7,
            max: 20
        }
    );
    assert!(requested_line_count_policy("write an html page").is_none());
}

#[test]
fn native_loop_fastauto_write_line_count_gate_rejects_out_of_range() {
    let content = (0..61)
        .map(|index| format!("<p>line {index}</p>"))
        .collect::<Vec<_>>()
        .join("\n");
    let result = validate_fast_auto_write_runtime_constraints(
        "toolu_write",
        "file.write",
        &ParsedToolArguments {
            path: Some("demo.html".to_string()),
            content: Some(content),
            ..ParsedToolArguments::default()
        },
        "请使用写入工具在文件夹内部写入一个30行左右的html小程序",
    )
    .unwrap();
    assert!(!result.ok);
    assert!(result.preview.contains("line_count_out_of_range"));
    assert!(result.detail_json.contains("\"actual_lines\":61"));
    assert!(result.detail_json.contains("\"min_lines\":19"));
    assert!(result.detail_json.contains("\"max_lines\":48"));
}

#[test]
fn native_loop_permissioned_write_blocks_sensitive_path_without_writing() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-fastauto-helper-sensitive-{nonce}"));
    fs::create_dir_all(root.join("artifacts/session")).unwrap();
    let artifact_store = ArtifactStore::new(root.join("artifacts/session"));
    let mut session = AgentSession::new("proj", "sess", "task").unwrap();
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .unwrap();
    let mut permission_gate = native_loop_permission_gate(
        &artifact_store,
        &root,
        PermissionMode::BypassPermissions,
        "sess",
    );

    let result = execute_permissioned_write_collect(
        &mut session,
        &artifact_store,
        &mut permission_gate,
        &root,
        0,
        None,
        "file.write",
        &ParsedToolArguments {
            path: Some(".env".to_string()),
            content: Some("TOKEN=secret".to_string()),
            ..ParsedToolArguments::default()
        },
        "write a file",
        None,
        &PermissionMode::BypassPermissions,
        &[],
        None,
    )
    .unwrap();

    assert!(matches!(result, PermissionedWriteOutcome::Pending(_)));
    assert!(!root.join(".env").exists());
    let jsonl = session.export_events_jsonl();
    assert!(jsonl.contains("\"event_type\":\"permission.requested\""));
    assert!(!jsonl.contains("\"event_type\":\"tool.call_completed\""));
}

#[test]
fn native_agent_loop_runs_scripted_live_transport_to_tools() {
    let fixture = run_scripted_native_agent_loop_fixture().unwrap();
    let result = fixture.loop_result;
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 2);
    assert_eq!(result.tool_call_count, 1);
    assert!(fixture.final_file_hash.starts_with("fnv64_"));
    assert!(result.event_jsonl.contains("\"tool_id\":\"file.write\""));
    assert!(!result.event_jsonl.contains("sk-"));
    assert!(!result.event_jsonl.contains(".env"));
}

#[test]
fn native_agent_loop_v2_continues_after_tool_results() {
    let result = run_scripted_native_agent_loop_v2_continuation_fixture().unwrap();
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 4);
    assert_eq!(result.tool_call_count, 3);
    assert!(result.event_jsonl.contains("agent.turn.started"));
    assert!(result.event_jsonl.contains("agent.tool.pending"));
    assert!(result.event_jsonl.contains("agent.tool.completed"));
    assert!(result.event_jsonl.contains("model.context_budget"));
    assert!(!result.event_jsonl.contains("\"prompt_hash\":\"unknown\""));
    assert!(!result
        .event_jsonl
        .contains("\"tool_catalog_hash\":\"unknown\""));
    assert!(!result.event_jsonl.contains("\"prompt_tokens_estimate\":0"));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.executor.role_call\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"deepseek.role_split.flash_savings\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"deepseek.cache.zone_a.miss\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"deepseek.cache.zone_a.hit\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"deepseek.cache.zone_b.miss\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"deepseek.cache.zone_b.hit\""));
    assert!(result
        .event_jsonl
        .contains("\"source\":\"runtime_prefix_reuse_observer\""));
    assert!(result.event_jsonl.contains("\"tool_id\":\"file.read\""));
    assert!(result
        .event_jsonl
        .contains("\"provider_tool_call_id\":\"toolu_1\""));
    assert!(result
        .event_jsonl
        .contains("\"tool_id\":\"search.ripgrep\""));
    assert!(result
        .event_jsonl
        .contains("\"provider_tool_call_id\":\"toolu_2\""));
    assert!(result.event_jsonl.contains("\"tool_id\":\"git.status\""));
    assert!(result
        .event_jsonl
        .contains("\"provider_tool_call_id\":\"toolu_3\""));
    assert!(!result.event_jsonl.contains("\"tool_results\":2"));
    assert!(!result.event_jsonl.contains("\"tool_results\":3"));
    assert!(result.event_jsonl.contains("Read, searched, checked git"));
}

#[test]
fn native_agent_loop_v2_survives_long_deepseek_tool_run_under_context_guard() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-native-agent-loop-v2-long-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    let readme = (0..80)
        .map(|index| format!("line {index}: context guard fixture"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(root.join("README.md"), readme).unwrap();
    let artifact_root = root.join("artifacts");
    let mut responses = Vec::new();
    for index in 0..20usize {
        responses.push(LiveHttpResponse {
                status_code: 200,
                body: format!(
                    "data: {{\"model\":\"Qwen/Qwen3.6-27B\",\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"toolu_read_{index}\",\"function\":{{\"name\":\"file_read\",\"arguments\":\"{{\\\"path\\\":\\\"README.md\\\",\\\"offset\\\":{index},\\\"limit\\\":1}}\"}}}}]}}}}]}}\n\
 data: {{\"usage\":{{\"prompt_tokens\":120,\"completion_tokens\":8,\"total_tokens\":128}}}}\n\
 data: [DONE]"
                ),
            });
    }
    responses.push(LiveHttpResponse {
            status_code: 200,
            body: "data: {\"model\":\"Qwen/Qwen3.6-27B\",\"choices\":[{\"delta\":{\"content\":\"Long tool run completed under context guard.\"}}]}\ndata: [DONE]".to_string(),
        });
    let transport = ScriptedLiveHttpTransport::new(responses);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_long".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_qwen_endpoint(),
            prompt: "Read README in small slices and summarize.".to_string(),
            max_tokens: 1024,
            max_iterations: 24,
            max_tool_calls: 24,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(
        result.status,
        NativeAgentLoopStatus::Completed,
        "{}",
        result.event_jsonl
    );
    assert_eq!(result.tool_call_count, 20);
    assert!(result.event_jsonl.contains("agent.turn.started"));
    assert!(result.event_jsonl.contains("model.context_budget"));
    assert!(!result.event_jsonl.contains("context.compaction.blocked"));
}

#[test]
fn native_agent_loop_v2_emits_incremental_events_to_sink() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-native-agent-loop-v2-sink-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Sink fixture\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"我先读 README。"}}
data: {"type":"content_block_stop","index":0}
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_read","name":"file_read","input":{}}}
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"README.md\"}"}}
data: {"type":"content_block_stop","index":1}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"README 已读取，可以继续。"}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let mut streamed = Vec::<String>::new();
    let result = run_native_agent_loop_v2_deepseek_with_event_sink(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_sink".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Read README and summarize.".to_string(),
            max_tokens: 1024,
            max_iterations: 0,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
        &mut |line| streamed.push(line.to_string()),
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert!(streamed.iter().any(
        |line| line.contains("\"event_type\":\"model.stream_delta\"")
            && line.contains("我先读 README")
    ));
    assert!(!streamed.iter().any(|line| line
        .contains("\"event_type\":\"runtime.stream.preamble_suppressed\"")
        && line.contains("tool_call_stream_preamble")));
    assert!(streamed.iter().any(|line| line
        .contains("\"event_type\":\"runtime.stream.narration\"")
        && line.contains("我先读 README")));
    assert!(streamed.iter().any(
        |line| line.contains("\"event_type\":\"tool.call_requested\"")
            && line.contains("\"tool_id\":\"file.read\"")
    ));
    assert!(streamed.iter().any(
        |line| line.contains("\"event_type\":\"tool.result_recorded\"")
            && line.contains("\"tool_id\":\"file.read\"")
    ));
    assert!(streamed.iter().any(
        |line| line.contains("\"event_type\":\"model.stream_delta\"")
            && line.contains("README 已读取")
    ));
    assert!(streamed
        .iter()
        .any(|line| line.contains("\"event_type\":\"assistant.message\"")
            && line.contains("README 已读取，可以继续")));
    assert!(!streamed
        .iter()
        .any(|line| line.contains("\"event_type\":\"assistant.message\"")
            && line.contains("我先读 README")));
}

#[test]
fn native_agent_loop_v2_stream_observer_flushes_visible_deltas_before_completion() {
    #[derive(Debug)]
    struct StreamingObserverTransport {
        streamed: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl LiveHttpTransport for StreamingObserverTransport {
        fn send(&self, _request: &PreparedModelHttpRequest) -> Result<LiveHttpResponse, String> {
            Err("non_streaming_send_should_not_be_used".to_string())
        }

        fn send_with_stream_observer(
            &self,
            _request: &PreparedModelHttpRequest,
            observer: &mut dyn FnMut(LiveHttpStreamEvent),
            _interrupt: &AtomicBool,
        ) -> Result<LiveHttpResponse, String> {
            observer(LiveHttpStreamEvent::HttpStatus { status_code: 200 });
            observer(LiveHttpStreamEvent::VisibleTextDelta("Live ".to_string()));
            assert!(
                self.streamed.lock().unwrap().iter().any(|line| line
                    .contains("\"event_type\":\"model.stream_delta\"")
                    && line.contains("Live ")),
                "visible delta should be emitted to the event sink before the transport returns"
            );
            observer(LiveHttpStreamEvent::VisibleTextDelta("delta".to_string()));
            Ok(LiveHttpResponse {
                    status_code: 200,
                    body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Live "}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"delta"}}
data: {"type":"message_delta","usage":{"input_tokens":8,"output_tokens":2}}
data: {"type":"message_stop"}"#
                        .to_string(),
                })
        }
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-live-stream-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    let artifact_root = root.join("artifacts");
    let streamed = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let transport = StreamingObserverTransport {
        streamed: std::sync::Arc::clone(&streamed),
    };
    let mut sink = |line: &str| streamed.lock().unwrap().push(line.to_string());
    let result = run_native_agent_loop_v2_deepseek_with_event_sink(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_live_stream".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Answer briefly.".to_string(),
            max_tokens: 1024,
            max_iterations: 2,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
        &mut sink,
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    let streamed = streamed.lock().unwrap().clone();
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    let content_delta_indexes = streamed
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            line.contains("\"event_type\":\"model.stream_delta\"")
                && line.contains("\"delta_kind\":\"content\"")
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let completed_index = streamed
        .iter()
        .position(|line| line.contains("\"event_type\":\"model.stream_completed\""))
        .unwrap();
    assert_eq!(content_delta_indexes.len(), 2);
    assert!(content_delta_indexes
        .iter()
        .all(|index| *index < completed_index));
    assert_eq!(
        result
            .event_jsonl
            .lines()
            .filter(|line| {
                line.contains("\"event_type\":\"model.stream_delta\"")
                    && line.contains("\"delta_kind\":\"content\"")
            })
            .count(),
        2
    );
    assert!(streamed
        .iter()
        .any(|line| line.contains("\"event_type\":\"assistant.message\"")
            && line.contains("Live delta")));
}

#[test]
fn native_agent_loop_v2_executes_deepseek_dsml_fallback_tools() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-native-agent-loop-v2-dsml-{nonce}"));
    fs::create_dir_all(root.join("crates/cli")).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
    fs::write(
        root.join("crates/cli/Cargo.toml"),
        "[package]\nname=\"cli\"\n",
    )
    .unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"好的，我先读取关键文件。\n<｜｜DSML｜｜tool_calls>\n<｜｜DSML｜｜invoke name=\"file_read\">\n<｜｜DSML｜｜parameter name=\"path\" string=\"true\">README.md</｜｜DSML｜｜parameter>\n</｜｜DSML｜｜invoke>\n<｜｜DSML｜｜invoke name=\"file_read\">\n<｜｜DSML｜｜parameter name=\"path\" string=\"true\">crates/cli/Cargo.toml</｜｜DSML｜｜parameter>\n</｜｜DSML｜｜invoke>\n</｜｜DSML｜｜tool_calls>"}}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"已读取 README 和 CLI manifest。"}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_dsml".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Read this project.".to_string(),
            max_tokens: 1024,
            max_iterations: 8,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 2);
    assert_eq!(result.tool_call_count, 2);
    assert!(result.event_jsonl.contains("\"tool_id\":\"file.read\""));
    assert!(result.event_jsonl.contains("README.md"));
    assert!(result.event_jsonl.contains("crates/cli/Cargo.toml"));
    assert!(result.event_jsonl.contains("已读取 README"));
    assert!(result.event_jsonl.contains("deepseek.dsml.leak"));
    assert!(result
        .event_jsonl
        .contains("\"source\":\"fallback_markup_parsed\""));
    assert!(result.event_jsonl.contains("deepseek.tool_call.assembled"));
    assert!(result
        .event_jsonl
        .contains("\"source\":\"visible_content_parse\""));
}

#[test]
fn native_agent_loop_v2_concurrent_read_only_batch_preserves_evidence_ordering() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-concurrent-read-only-{nonce}"
    ));
    fs::create_dir_all(root.join("crates/cli")).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    fs::write(
        root.join("crates/cli/Cargo.toml"),
        "[package]\nname=\"cli\"\n",
    )
    .unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"{\"tool_calls\":[{\"name\":\"file_read\",\"arguments\":{\"path\":\"README.md\"}},{\"name\":\"file_read\",\"arguments\":{\"path\":\"crates/cli/Cargo.toml\",\"limit\":1}}]}"}}
data: [DONE]"#
                .to_string(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Concurrent read-only evidence was replayed in model order."}}
data: [DONE]"#
                .to_string(),
        },
    ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_concurrent_read_only".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Read README.md and crates/cli/Cargo.toml concurrently.".to_string(),
            max_tokens: 1024,
            max_iterations: 8,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: true,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.tool_call_count, 2);
    let readme_pos = result.event_jsonl.find("README.md").unwrap();
    let cargo_pos = result.event_jsonl.find("crates/cli/Cargo.toml").unwrap();
    assert!(
        readme_pos < cargo_pos,
        "concurrent execution should preserve model-facing evidence order"
    );
    let alias_pos = result
        .event_jsonl
        .find("tool.name.alias_resolved")
        .expect("concurrent path should record TCML alias resolution before dispatch");
    let requested_pos = result
        .event_jsonl
        .find("\"event_type\":\"tool.call_requested\"")
        .expect("concurrent path should record tool execution after TCML mediation");
    assert!(
        alias_pos < requested_pos,
        "TCML mediation telemetry must precede concurrent tool execution"
    );
    assert!(result
        .event_jsonl
        .contains("\"requested_tool\":\"file_read\""));
    assert!(result
        .event_jsonl
        .contains("\"resolved_tool\":\"file.read\""));
    let sent_requests = transport.sent_requests();
    assert_eq!(sent_requests.len(), 2);
    assert!(sent_requests[1]
        .body_json
        .contains("\"path\":\"README.md\""));
    assert!(sent_requests[1]
        .body_json
        .contains("\"path\":\"crates/cli/Cargo.toml\""));
    assert!(
        sent_requests[1].body_json.contains("\"limit\":1"),
        "concurrent evidence should preserve canonical numeric arguments: {}",
        sent_requests[1].body_json
    );
    assert!(
        !sent_requests[1]
            .body_json
            .contains("\"arguments_json\":\"{}\""),
        "concurrent evidence must preserve canonical tool arguments"
    );
    assert!(result.event_jsonl.contains("Concurrent read-only evidence"));
}

#[test]
fn native_agent_loop_v2_concurrent_read_only_mediation_error_is_model_readable() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-concurrent-mediation-error-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"<｜｜DSML｜｜tool_calls>\n<｜｜DSML｜｜invoke name=\"file_read\">\n</｜｜DSML｜｜invoke>\n<｜｜DSML｜｜invoke name=\"file_read\">\n<｜｜DSML｜｜parameter name=\"path\" string=\"true\">README.md</｜｜DSML｜｜parameter>\n</｜｜DSML｜｜invoke>\n</｜｜DSML｜｜tool_calls>"}}
data: [DONE]"#
                .to_string(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"I saw the malformed call as an error and then used the valid read result."}}
data: [DONE]"#
                .to_string(),
        },
    ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_concurrent_mediation_error".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Read README.md, but handle malformed tool calls as observations.".to_string(),
            max_tokens: 1024,
            max_iterations: 8,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: true,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert!(result
        .event_jsonl
        .contains("tool_contract_rejected_concurrent"));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"tool.result_recorded\""));
    assert!(result.event_jsonl.contains("schema_validation_failed"));
    assert!(result.event_jsonl.contains("README.md"));
    let sent_requests = transport.sent_requests();
    assert_eq!(sent_requests.len(), 2);
    assert!(
        sent_requests[1].body_json.contains("missing")
            || sent_requests[1].body_json.contains("required"),
        "the model continuation should include the schema error observation"
    );
    assert!(sent_requests[1]
        .body_json
        .contains("\"path\":\"README.md\""));
}

#[test]
fn native_agent_loop_v2_concurrent_branch_leaves_writes_to_serial_permission_path() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-concurrent-write-serial-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"{\"tool_calls\":[{\"name\":\"file_write\",\"arguments\":{\"path\":\"a.txt\",\"content\":\"A\\n\"}},{\"name\":\"file_write\",\"arguments\":{\"path\":\"b.txt\",\"content\":\"B\\n\"}}]}"}}
data: [DONE]"#
            .to_string(),
    }]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_concurrent_write_serial".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Create two files.".to_string(),
            max_tokens: 1024,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: true,
        },
    )
    .unwrap();
    let a_exists = root.join("a.txt").exists();
    let b_exists = root.join("b.txt").exists();
    let _ = fs::remove_dir_all(&root);

    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForToolApproval);
    assert!(!a_exists);
    assert!(!b_exists);
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"permission.requested\""));
    assert!(!result.event_jsonl.contains("NonReadOnlyTool"));
    assert!(!result
        .event_jsonl
        .contains("tool_contract_rejected_concurrent"));
    assert!(matches!(
        result.pending_tool,
        Some(PendingNativeToolExecution {
            tool_id,
            request_type: PermissionRequestType::FileWrite,
            ..
        }) if tool_id == "file.write"
    ));
}

#[test]
fn native_agent_loop_v2_concurrent_read_only_respects_write_ordering_barrier() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-concurrent-write-barrier-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.txt"), "A_CONTENT_READ_BEFORE_WRITE\n").unwrap();
    fs::write(root.join("c.txt"), "C_CONTENT_MUST_NOT_READ_BEFORE_WRITE\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"{\"tool_calls\":[{\"name\":\"file_read\",\"arguments\":{\"path\":\"a.txt\"}},{\"name\":\"file_write\",\"arguments\":{\"path\":\"b.txt\",\"content\":\"B\\n\"}},{\"name\":\"file_read\",\"arguments\":{\"path\":\"c.txt\"}}]}"}}
data: [DONE]"#
            .to_string(),
    }]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_concurrent_write_barrier".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Read a, write b, then read c.".to_string(),
            max_tokens: 1024,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: true,
        },
    )
    .unwrap();
    let b_exists = root.join("b.txt").exists();
    let _ = fs::remove_dir_all(&root);

    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForToolApproval);
    assert!(!b_exists);
    assert!(result.event_jsonl.contains("A_CONTENT_READ_BEFORE_WRITE"));
    assert!(!result
        .event_jsonl
        .contains("C_CONTENT_MUST_NOT_READ_BEFORE_WRITE"));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"permission.requested\""));
    assert!(matches!(
        result.pending_tool,
        Some(PendingNativeToolExecution {
            tool_id,
            request_type: PermissionRequestType::FileWrite,
            ..
        }) if tool_id == "file.write"
    ));
}

#[test]
fn native_agent_loop_v2_executes_multiple_native_tool_use_blocks() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-native-multi-tool-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"file_read","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"README.md\"}"}}
data: {"type":"content_block_stop","index":0}
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_2","name":"file_read","input":{}}}
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"Cargo.toml\"}"}}
data: {"type":"content_block_stop","index":1}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Both native tool_use blocks were executed and paired with tool_result blocks."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_native_multi_tool".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Read README.md and Cargo.toml.".to_string(),
            max_tokens: 1024,
            max_iterations: 8,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 2);
    assert_eq!(result.tool_call_count, 2);
    assert!(result.event_jsonl.contains("README.md"));
    assert!(result.event_jsonl.contains("Cargo.toml"));
    assert!(result
        .event_jsonl
        .contains("agent.tool.streaming_completed"));
    assert!(result
        .event_jsonl
        .contains("agent.tool.streaming_batch_ready"));
    assert!(result.event_jsonl.contains("deepseek.tool_call.partial"));
    assert!(result.event_jsonl.contains("deepseek.tool_call.assembled"));
    assert!(result
        .event_jsonl
        .contains("\"source\":\"streaming_assembler\""));
    let streaming_pos = result
        .event_jsonl
        .find("agent.tool.streaming_completed")
        .unwrap();
    let stream_completed_pos = result
        .event_jsonl
        .find("\"event_type\":\"model.stream_completed\"")
        .unwrap();
    assert!(
        streaming_pos < stream_completed_pos,
        "streamed tool should execute before final stream recording"
    );
}

#[test]
fn native_agent_loop_v2_incomplete_streamed_tool_call_becomes_model_readable_error() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-incomplete-stream-tool-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
	        LiveHttpResponse {
	            status_code: 200,
	            body: r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_incomplete_read","function":{"name":"file_read","arguments":"{\"path\":\"README"}}]}}]}
data: {"choices":[{"finish_reason":"tool_calls","delta":{}}]}
data: [DONE]"#
	                .to_string(),
	        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"choices":[{"delta":{"content":"I saw the malformed streamed tool result and will continue without assuming the read ran."}}]}
data: [DONE]"#
                .to_string(),
        },
    ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_incomplete_stream_tool".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Read README.md and continue.".to_string(),
            max_tokens: 1024,
            max_iterations: 8,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);

    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 2);
    assert!(result
        .event_jsonl
        .contains("deepseek.tool_call.incomplete_flushed"));
    assert!(result
        .event_jsonl
        .contains("\"provider_stop_reason\":\"tool_calls\""));
    assert!(result.event_jsonl.contains("call_incomplete_read"));
    assert!(result.event_jsonl.contains("tool.model_readable_error"));
    assert!(result.event_jsonl.contains("tool.result_recorded"));
    assert!(result.event_jsonl.contains("MALFORMED_TOOL_JSON"));
    assert!(result.event_jsonl.contains("malformed_tool_json"));
    assert!(result.event_jsonl.contains("without assuming the read ran"));
}

#[test]
fn native_agent_loop_v2_streamed_parsed_mismatch_gets_synthetic_tool_result() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-stream-mismatch-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"<｜｜DSML｜｜tool_calls>\n<｜｜DSML｜｜invoke name=\"file_read\">\n<｜｜DSML｜｜parameter name=\"path\" string=\"true\">README.md</｜｜DSML｜｜parameter>\n</｜｜DSML｜｜invoke>\n<｜｜DSML｜｜invoke name=\"file_read\">\n<｜｜DSML｜｜parameter name=\"path\" string=\"true\">Cargo.toml</｜｜DSML｜｜parameter>\n</｜｜DSML｜｜invoke>\n</｜｜DSML｜｜tool_calls>"}}
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_readme","name":"file_read","input":{"path":"README.md"}}}
data: {"type":"content_block_stop","index":1}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"I handled the streamed/parsed mismatch without repeating the missing call."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_stream_mismatch".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Read README.md and Cargo.toml.".to_string(),
            max_tokens: 1024,
            max_iterations: 0,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert!(result
        .event_jsonl
        .contains("continue_with_streamed_results_size_mismatch"));
    assert!(result.event_jsonl.contains("STREAMED_RESULT_MISSING"));
    assert!(result.event_jsonl.contains("\"mismatch_error_count\":1"));
    assert!(result.event_jsonl.contains("Cargo.toml"));
    assert!(result
        .event_jsonl
        .contains("without repeating the missing call"));
}

#[test]
fn native_agent_loop_v2_stream_duplicates_use_unique_ledger_ids() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-stream-duplicate-ledger-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_read_1","name":"file_read","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"README.md\"}"}}
data: {"type":"content_block_stop","index":0}
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_read_2","name":"file_read","input":{}}}
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"README.md\"}"}}
data: {"type":"content_block_stop","index":1}
data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_read_3","name":"file_read","input":{}}}
data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"README.md\"}"}}
data: {"type":"content_block_stop","index":2}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"README evidence was collected once and duplicate stream reads were suppressed."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_stream_duplicate_ledger".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Read README.md.".to_string(),
            max_tokens: 1024,
            max_iterations: 0,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.tool_call_count, 1);
    assert!(result
        .event_jsonl
        .contains("native_loop_v2_stream_ledger_0_0"));
    assert!(result
        .event_jsonl
        .contains("native_loop_v2_stream_ledger_0_1"));
    assert!(result
        .event_jsonl
        .contains("native_loop_v2_stream_ledger_0_2"));
    assert!(result
        .event_jsonl
        .contains("agent.tool.streaming_duplicate_suppressed"));
    assert!(!result.event_jsonl.contains("duplicate tool_call_id"));
}

#[test]
fn native_agent_loop_v2_recovers_unknown_tool_alias_without_crashing() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("researchcode-native-agent-loop-v2-unknown-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"<tool_call><name>made_up_read_tool</name><arguments>{\"path\":\"README.md\"}</arguments></tool_call>"}}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"I recovered from the unsupported tool name and will use the stable catalog next."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_unknown_tool".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Read this project.".to_string(),
            max_tokens: 1024,
            max_iterations: 8,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 2);
    assert_eq!(result.tool_call_count, 1);
    assert!(result.event_jsonl.contains("\"tool.name.unknown\""));
    assert!(result.event_jsonl.contains("\"tool.error.model_readable\""));
    assert!(result.event_jsonl.contains("UNKNOWN_TOOL"));
    assert!(result.event_jsonl.contains("made_up_read_tool"));
    assert!(result
        .event_jsonl
        .contains("I recovered from the unsupported tool name"));
}

#[test]
fn native_agent_loop_v2_shell_list_intent_requests_permission_without_replacement() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-manifest-reject-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"{\"tool_calls\":[{\"name\":\"shell.command\",\"arguments\":{\"command\":\"find . -maxdepth 1\"}}]}"}}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Recovered after manifest rejection; use repo.map in read-only mode."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_shell_list_permission".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Inspect repo read-only.".to_string(),
            max_tokens: 1024,
            max_iterations: 8,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForToolApproval);
    assert_eq!(result.model_call_count, 1);
    assert_eq!(result.tool_call_count, 0);
    assert!(result.pending_tool.is_some());
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"permission.requested\""));
    assert!(result.event_jsonl.contains("\"tool_id\":\"shell.command\""));
    assert!(!result
        .event_jsonl
        .contains("\"event_type\":\"tool.alias_shell_list_to_directory_tool\""));
    assert!(!result.event_jsonl.contains("TOOL_NOT_IN_MANIFEST"));
}

#[test]
fn native_agent_loop_v2_tool_inventory_stops_before_budget_after_gated_attempt() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-tool-inventory-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("hello_tools.txt"), "tool fixture\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_list","name":"file_list_directory","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\".\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_write","name":"file_write","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"should-not-write.txt\",\"content\":\"no\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"This response should not be needed."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_tool_inventory".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "请测试一下你拥有的所有工具，不依赖原始记忆".to_string(),
            max_tokens: 4096,
            max_iterations: 5,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForToolApproval);
    assert_eq!(result.tool_call_count, 1);
    assert!(result.pending_tool.is_some());
    assert!(!result
        .event_jsonl
        .contains("agent.tool_inventory.completed"));
    assert!(!result.event_jsonl.contains("tool_inventory_summary"));
    assert!(result.event_jsonl.contains("file.list_directory"));
    assert!(result.event_jsonl.contains("file.write"));
    assert!(!result.event_jsonl.contains("agent.loop_budget_reached"));
    assert!(!result
        .event_jsonl
        .contains("This response should not be needed"));
}

#[test]
fn native_agent_loop_v2_recovers_from_repeated_tool_batch() {
    let result = run_scripted_native_agent_loop_v2_repeated_tool_recovery_fixture().unwrap();
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 3);
    assert_eq!(result.tool_call_count, 1);
    assert!(result.event_jsonl.contains("\"agent.loop_recovery\""));
    assert!(result.event_jsonl.contains("\"tool.auto_recovery\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"tool.duplicate_observation_suppressed\""));
    assert!(result.event_jsonl.contains("\"skipped\":true"));
    assert!(result
        .event_jsonl
        .contains("\"reason\":\"duplicate_observation\""));
    assert!(
        result.event_jsonl.contains("\"auto_list_directory\"")
            || result
                .event_jsonl
                .contains("\"source\":\"streaming_tool_call\"")
    );
    assert!(result.event_jsonl.contains("repeated_tool_batch"));
    assert!(result.event_jsonl.contains("path_is_directory"));
    assert!(result.event_jsonl.contains("Recovered:"));
    assert!(!result.event_jsonl.contains("\"agent.loop_incomplete\""));
}

#[test]
fn native_agent_loop_v2_manifest_keeps_shell_without_manifest_recovery() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-shell-permission-recovery-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_shell_pwd","function":{"name":"shell.command","arguments":"{\"command\":\"pwd\"}"}}]}}]}
data: {"choices":[{"finish_reason":"tool_calls","delta":{}}]}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"目录结构已获取，我可以继续读取具体文件。"}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_shell_manifest_recovery".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "运行 pwd 并继续分析".to_string(),
            max_tokens: 1024,
            max_iterations: 8,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::FastAutoWrite,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForToolApproval);
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"tool.manifest.generated\""));
    assert!(result.event_jsonl.contains("shell.command"));
    assert!(result.event_jsonl.contains("patch.apply"));
    assert!(!result.event_jsonl.contains("TOOL_NOT_IN_MANIFEST"));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"permission.requested\""));
    assert!(!result.event_jsonl.contains("PermissionRequired"));
    assert!(matches!(
        result.pending_tool,
        Some(PendingNativeToolExecution {
            tool_id,
            request_type: PermissionRequestType::Command,
            ..
        }) if tool_id == "shell.command"
    ));
}

#[test]
fn native_agent_loop_v2_hard_denied_shell_is_model_readable_without_permission() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-shell-hard-deny-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_shell_bash","function":{"name":"shell.command","arguments":"{\"command\":\"/bin/bash -lc ls\"}"}}]}}]}
data: {"choices":[{"finish_reason":"tool_calls","delta":{}}]}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Blocked command handled; I will use read-only tools instead."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_shell_hard_deny".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Try a shell command, then recover safely.".to_string(),
            max_tokens: 1024,
            max_iterations: 8,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::FastAutoWrite,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert!(result.pending_tool.is_none());
    assert!(result.event_jsonl.contains("COMMAND_CLASSIFIER_BLOCKED"));
    assert!(!result
        .event_jsonl
        .contains("\"event_type\":\"permission.requested\""));
}

#[test]
fn native_agent_loop_v2_converts_tool_error_to_tool_result() {
    let result = run_scripted_native_agent_loop_v2_tool_error_continuation_fixture().unwrap();
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 2);
    assert_eq!(result.tool_call_count, 1);
    assert!(result.event_jsonl.contains("\"tool_id\":\"file.write\""));
    assert!(result.event_jsonl.contains("SCHEMA_VALIDATION_FAILED"));
    assert!(result
        .event_jsonl
        .contains("missing required field content"));
    assert!(result
        .event_jsonl
        .contains("final answer instead of stopping"));
    assert!(!result.event_jsonl.contains("\"agent.loop_incomplete\""));
}

#[test]
fn native_agent_loop_v2_permission_denial_returns_tool_result() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-loop-v2-permission-denial-result-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_denied_write","name":"file_write","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"denied.txt\",\"content\":\"blocked\n\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"The write was denied by policy, so I am stopping without retrying it."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_permission_denial".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Try a write, but permission mode should deny it.".to_string(),
            max_tokens: 1024,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::FastAutoWrite,
            permission_mode: PermissionMode::DontAsk,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let denied_file_exists = root.join("denied.txt").exists();
    let _ = fs::remove_dir_all(&root);

    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 2);
    assert!(!denied_file_exists);
    assert!(
        result.event_jsonl.contains("PERMISSION_DENIED"),
        "{}",
        result.event_jsonl
    );
    assert!(result.event_jsonl.contains("toolu_denied_write"));
    assert!(result.event_jsonl.contains("without retrying it"));
}

#[test]
fn native_agent_loop_v2_streaming_write_to_sensitive_path_requests_permission() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-loop-v2-sensitive-stream-write-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_sensitive_write","name":"file_write","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\".env\",\"content\":\"TOKEN=secret\n\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                .to_string(),
        }]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_sensitive_stream_write".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Try writing a sensitive file.".to_string(),
            max_tokens: 1024,
            max_iterations: 2,
            max_tool_calls: 4,
            tool_exposure: NativeAgentToolExposure::FastAutoWrite,
            permission_mode: PermissionMode::BypassPermissions,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let sensitive_file_exists = root.join(".env").exists();
    let _ = fs::remove_dir_all(&root);

    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForToolApproval);
    assert!(!sensitive_file_exists);
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"permission.requested\""));
    assert!(result.event_jsonl.contains("toolu_sensitive_write"));
    assert!(result.pending_tool.is_some());
}

#[test]
fn native_agent_loop_v2_read_only_streamed_write_requests_permission() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-loop-v2-read-only-stream-write-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_readonly_write","name":"file_write","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"read-only-stream-write.txt\",\"content\":\"hello\n\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                .to_string(),
        }]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_readonly_stream_write".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Try writing a file from read-only exposure.".to_string(),
            max_tokens: 1024,
            max_iterations: 2,
            max_tool_calls: 4,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let written_file_exists = root.join("read-only-stream-write.txt").exists();
    let _ = fs::remove_dir_all(&root);

    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForToolApproval);
    assert!(!written_file_exists);
    assert!(result.pending_tool.is_some());
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"permission.requested\""));
    assert!(result.event_jsonl.contains("toolu_readonly_write"));
    assert!(!result.event_jsonl.contains("TOOL_NOT_IN_MANIFEST"));
    assert!(!result.event_jsonl.contains("tool.name.unknown"));
}

#[test]
fn native_agent_loop_v2_read_only_openai_write_requests_permission() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-loop-v2-read-only-openai-write-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    let artifact_root = root.join("artifacts");
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = true;
    endpoint.api_key_env = "PATH".to_string();
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_readonly_write","function":{"name":"file_write","arguments":"{\"path\":\"read-only-openai-write.txt\",\"content\":\"hello\n\"}"}}]}}]}
data: {"choices":[{"finish_reason":"tool_calls","delta":{}}]}
data: [DONE]"#
                .to_string(),
        }]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_readonly_openai_write".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint,
            prompt: "Try writing a file from read-only exposure using OpenAI tool calls."
                .to_string(),
            max_tokens: 1024,
            max_iterations: 2,
            max_tool_calls: 4,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let written_file_exists = root.join("read-only-openai-write.txt").exists();
    let _ = fs::remove_dir_all(&root);

    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForToolApproval);
    assert!(!written_file_exists);
    assert!(result.pending_tool.is_some());
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"permission.requested\""));
    assert!(result.event_jsonl.contains("call_readonly_write"));
    assert!(!result.event_jsonl.contains("TOOL_NOT_IN_MANIFEST"));
    assert!(!result.event_jsonl.contains("tool.name.unknown"));
}

#[test]
fn native_agent_loop_v2_escalates_repeated_tool_contract_rejection() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-contract-escalation-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\nold\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"<｜｜DSML｜｜tool_calls>\n<｜｜DSML｜｜invoke name=\"file_edit\">\n<｜｜DSML｜｜parameter name=\"path\" string=\"true\">README.md</｜｜DSML｜｜parameter>\n</｜｜DSML｜｜invoke>\n</｜｜DSML｜｜tool_calls>"}}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"<｜｜DSML｜｜tool_calls>\n<｜｜DSML｜｜invoke name=\"file_edit\">\n<｜｜DSML｜｜parameter name=\"path\" string=\"true\">README-2.md</｜｜DSML｜｜parameter>\n</｜｜DSML｜｜invoke>\n</｜｜DSML｜｜tool_calls>"}}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"<｜｜DSML｜｜tool_calls>\n<｜｜DSML｜｜invoke name=\"file_edit\">\n<｜｜DSML｜｜parameter name=\"path\" string=\"true\">README-3.md</｜｜DSML｜｜parameter>\n</｜｜DSML｜｜invoke>\n</｜｜DSML｜｜tool_calls>"}}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"content":"I will stop retrying the invalid edit call."}}]}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_contract_escalation".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "请修改 README.md".to_string(),
            max_tokens: 1024,
            max_iterations: 5,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::FastAutoWrite,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.recovery.escalated\""));
    assert!(result.event_jsonl.contains("failure_count\":3"));
    assert!(result
        .event_jsonl
        .contains("Stop retrying file.edit with missing old_string/new_string"));
    assert!(result.event_jsonl.contains("retryable=false"));
    assert!(result
        .event_jsonl
        .contains("\"suggested_replacement\":\"file.write\""));
    assert!(result
        .event_jsonl
        .contains("I will stop retrying the invalid edit call."));
    assert!(result
        .event_jsonl
        .contains("\"reason\":\"same_tool_error_plateau\""));
    assert!(result.event_jsonl.contains("\"verdict\":\"soft_warning\""));
    assert!(!result
        .event_jsonl
        .contains("\"event_type\":\"agent.loop_plateau_stopped\""));
}

#[test]
fn native_agent_loop_injects_base_hash_for_file_edit_dispatch() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-edit-hash-injection-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "alpha\n").unwrap();
    let args = ToolExecutionArgs {
        path: Some("README.md".to_string()),
        old_string: Some("alpha".to_string()),
        new_string: Some("beta".to_string()),
        ..ToolExecutionArgs::default()
    };

    let prepared = prepare_exact_edit_execution_args(&root, "file.edit", &args).unwrap();
    let _ = fs::remove_dir_all(&root);

    let expected_hash = stable_text_hash("alpha\n");
    assert_eq!(prepared.base_hash.as_deref(), Some(expected_hash.as_str()));
    assert_eq!(prepared.old_string.as_deref(), Some("alpha"));
    assert_eq!(prepared.new_string.as_deref(), Some("beta"));
}

#[test]
fn native_agent_loop_injects_base_hash_for_file_write_existing_only() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("researchcode-native-file-write-base-hash-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("existing.txt"), "old\n").unwrap();

    let prepared = prepare_exact_edit_execution_args(
        &root,
        "file.write",
        &ToolExecutionArgs {
            path: Some("existing.txt".to_string()),
            content: Some("new\n".to_string()),
            ..ToolExecutionArgs::default()
        },
    )
    .unwrap();
    assert_eq!(
        prepared.base_hash.as_deref(),
        Some(stable_text_hash("old\n").as_str())
    );

    let create = prepare_exact_edit_execution_args(
        &root,
        "file.write",
        &ToolExecutionArgs {
            path: Some("created.txt".to_string()),
            content: Some("new\n".to_string()),
            ..ToolExecutionArgs::default()
        },
    )
    .unwrap();
    assert_eq!(create.base_hash, None);

    let stale = prepare_exact_edit_execution_args(
        &root,
        "file.edit",
        &ToolExecutionArgs {
            path: Some("existing.txt".to_string()),
            old_string: Some("old".to_string()),
            new_string: Some("new".to_string()),
            base_hash: Some("model_hash".to_string()),
            ..ToolExecutionArgs::default()
        },
    )
    .unwrap();
    assert_eq!(stale.base_hash.as_deref(), Some("model_hash"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_agent_loop_v2_fastauto_write_executes_file_write() {
    let (result, written) = run_scripted_native_agent_loop_v2_fastauto_write_fixture().unwrap();
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForToolApproval);
    assert!(written.is_empty(), "written={written:?}");
    assert!(result.event_jsonl.contains("\"tool_id\":\"file.write\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"permission.requested\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"tcml.pipeline.completed\""));
    assert!(result.pending_tool.is_some());
    assert!(!result.event_jsonl.contains("MissingArgument(\"content\")"));
}

#[test]
fn native_agent_loop_v2_fastauto_write_recovers_from_line_count_rejection() {
    fn html_lines(count: usize, label: &str) -> String {
        let mut lines = vec![
            "<!doctype html>".to_string(),
            "<html lang=\"en\">".to_string(),
            "<body>".to_string(),
        ];
        for index in 0..count.saturating_sub(5) {
            lines.push(format!("<p>{label} line {}</p>", index + 1));
        }
        lines.push("</body>".to_string());
        lines.push("</html>".to_string());
        lines.join("\n")
    }

    fn file_write_stream_body(path: &str, content: &str) -> String {
        let args = format!(
            "{{\"path\":{},\"content\":{}}}",
            json_string(path),
            json_string(content)
        );
        let escaped_args = args.replace('\\', "\\\\").replace('"', "\\\"");
        format!(
                "data: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"tool_use\",\"id\":\"toolu_write\",\"name\":\"file_write\",\"input\":{{}}}}}}\n\
data: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":\"{escaped_args}\"}}}}\n\
data: {{\"type\":\"content_block_stop\",\"index\":0}}\n\
data: [DONE]"
            )
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-line-gate-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    let artifact_root = root.join("artifacts");
    let bad_html = html_lines(61, "bad");
    let repaired_html = html_lines(30, "repaired");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: file_write_stream_body("line-count.html", &bad_html),
            },
            LiveHttpResponse {
                status_code: 200,
                body: file_write_stream_body("line-count.html", &repaired_html),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Created line-count.html after correcting the line count."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_line_gate".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "请使用写入工具在文件夹内部写入一个30行左右的html小程序".to_string(),
            max_tokens: 4096,
            max_iterations: 5,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::FastAutoWrite,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let written = fs::read_to_string(root.join("line-count.html")).unwrap_or_default();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForToolApproval);
    assert!(written.is_empty());
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"permission.requested\""));
    assert!(result.pending_tool.is_some());
    assert!(!written.contains("bad line 56"));
}

#[test]
fn native_agent_loop_v2_fastauto_write_repairs_stale_create_path() {
    fn html_lines(label: &str) -> String {
        let mut lines = vec![
            "<!doctype html>".to_string(),
            "<html lang=\"en\">".to_string(),
            "<body>".to_string(),
        ];
        for index in 0..25 {
            lines.push(format!("<p>{label} line {}</p>", index + 1));
        }
        lines.push("</body>".to_string());
        lines.push("</html>".to_string());
        lines.join("\n")
    }

    fn file_write_stream_body(path: &str, base_hash: &str, content: &str) -> String {
        let args = format!(
            "{{\"path\":{},\"base_hash\":{},\"content\":{}}}",
            json_string(path),
            json_string(base_hash),
            json_string(content)
        );
        let escaped_args = args.replace('\\', "\\\\").replace('"', "\\\"");
        format!(
                "data: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"tool_use\",\"id\":\"toolu_write\",\"name\":\"file_write\",\"input\":{{}}}}}}\n\
data: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":\"{escaped_args}\"}}}}\n\
data: {{\"type\":\"content_block_stop\",\"index\":0}}\n\
data: [DONE]"
            )
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-stale-create-repair-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("generated_app.html"), "existing\n").unwrap();
    let artifact_root = root.join("artifacts");
    let html = html_lines("repaired");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: file_write_stream_body("generated_app.html", "stale_hash", &html),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Created generated_app_2.html after avoiding the stale existing file."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_stale_create_repair".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "请使用写入工具在文件夹内部写入一个30行左右的html小程序".to_string(),
            max_tokens: 4096,
            max_iterations: 4,
            max_tool_calls: 4,
            tool_exposure: NativeAgentToolExposure::FastAutoWrite,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let original = fs::read_to_string(root.join("generated_app.html")).unwrap();
    let written = fs::read_to_string(root.join("generated_app_2.html")).unwrap_or_default();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForToolApproval);
    assert_eq!(original, "existing\n");
    assert!(written.is_empty());
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"permission.requested\""));
    assert!(result.pending_tool.is_some());
}

#[test]
fn native_agent_loop_v2_empty_write_followup_stops_without_runtime_write_fallback() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-empty-write-recovery-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "demo\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"name":"file.list_directory","arguments":"{\"path\":\".\"}"}}]}}]}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: "data: [DONE]".to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_empty_write_recovery".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_qwen_endpoint(),
            prompt: "请使用写入工具在文件夹内部写入一个30行左右的html小程序".to_string(),
            max_tokens: 4096,
            max_iterations: 4,
            max_tool_calls: 4,
            tool_exposure: NativeAgentToolExposure::FastAutoWrite,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let generated_path = root.join("generated_app.html");
    let generated_exists = generated_path.exists();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Failed);
    assert_eq!(result.final_state, AgentState::Failed);
    assert_eq!(result.tool_call_count, 1);
    assert!(!generated_exists);
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.loop_stopped\""));
    assert!(!result
        .event_jsonl
        .contains("agent.fast_auto_write.recovery"));
    assert!(!result
        .event_jsonl
        .contains("agent.fast_auto_write.completed"));
}

#[test]
fn qwen_native_agent_loop_v2_fastauto_write_executes_file_write() {
    let (result, written) =
        run_scripted_qwen_native_agent_loop_v2_fastauto_write_fixture().unwrap();
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForToolApproval);
    assert!(written.is_empty(), "written={written:?}");
    assert!(result.event_jsonl.contains("\"provider\":\"qwen\""));
    assert!(result.event_jsonl.contains("\"tool_id\":\"file.write\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"permission.requested\""));
    assert!(result.pending_tool.is_some());
    assert!(!result.event_jsonl.contains("MissingArgument(\"content\")"));
}

#[test]
fn native_agent_loop_v2_blocks_after_max_iterations_without_visible_finalizer() {
    let result = run_scripted_native_agent_loop_v2_max_iteration_structured_stop_fixture().unwrap();
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForUser);
    assert_eq!(result.model_call_count, 1);
    assert_eq!(result.tool_call_count, 1);
    assert!(result.event_jsonl.contains("\"agent.loop_budget_reached\""));
    assert!(result.event_jsonl.contains("\"reason\":\"max_iterations\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.loop_stopped\""));
    assert_eq!(
        event_payload_string(&result.event_jsonl, "agent.loop_stopped", "status"),
        "blocked"
    );
    assert_eq!(
        event_payload_string(&result.event_jsonl, "agent.loop_stopped", "category"),
        "turn_budget"
    );
    assert_eq!(
        event_payload_string(&result.event_jsonl, "agent.loop_stopped", "next_action"),
        "surface_blocked_stop_and_release_turn"
    );
    assert_eq!(
        event_count(&result.event_jsonl, "agent.loop_state.terminal"),
        1
    );
    assert!(result.event_jsonl.contains("\"status\":\"blocked\""));
    assert!(result
        .event_jsonl
        .contains("\"to_state\":\"WaitingForUser\""));
    assert!(!result.event_jsonl.contains("\"to_state\":\"Failed\""));
    assert!(!result.event_jsonl.contains("visible_finalizer"));
    assert!(!result.event_jsonl.contains("Max iteration structured stop"));
    assert!(!result.event_jsonl.contains("\"agent.loop_incomplete\""));
}

#[test]
fn native_agent_loop_v2_blocks_when_max_iteration_structured_stop_transport_fails() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-max-structured-stop-transport-fail-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "fallback evidence\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_read","name":"file_read","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"README.md\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                .to_string(),
        }]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_max_structured_stop_transport_fail".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Read README and summarize even when max iterations is low.".to_string(),
            max_tokens: 1024,
            max_iterations: 1,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForUser);
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.loop_budget_reached\""));
    assert!(result.event_jsonl.contains("\"reason\":\"max_iterations\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.loop_stopped\""));
    assert!(result
        .event_jsonl
        .contains("\"to_state\":\"WaitingForUser\""));
    assert!(!result
        .event_jsonl
        .contains("\"event_type\":\"assistant.message\""));
    assert!(!result.event_jsonl.contains("\"agent.loop_incomplete\""));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_agent_loop_v2_blocks_after_max_tool_calls_without_visible_finalizer() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-tool-limit-structured-stop-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "ResearchCode tool limit fixture\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_map","name":"repo_map","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"root\":\".\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Tool limit structured stop: repo map was enough to summarize without requesting more tools."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_tool_limit_structured_stop".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Map the repo and summarize even when the tool budget is low.".to_string(),
            max_tokens: 1024,
            max_iterations: 8,
            max_tool_calls: 1,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForUser);
    assert_eq!(result.model_call_count, 1);
    assert_eq!(result.tool_call_count, 1);
    assert!(result.event_jsonl.contains("\"agent.loop_budget_reached\""));
    assert!(result.event_jsonl.contains("\"reason\":\"max_tool_calls\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.loop_stopped\""));
    assert_eq!(
        event_payload_string(&result.event_jsonl, "agent.loop_stopped", "status"),
        "blocked"
    );
    assert_eq!(
        event_payload_string(&result.event_jsonl, "agent.loop_stopped", "category"),
        "turn_budget"
    );
    assert_eq!(
        event_payload_string(&result.event_jsonl, "agent.loop_stopped", "next_action"),
        "surface_blocked_stop_and_release_turn"
    );
    assert!(result
        .event_jsonl
        .contains("\"to_state\":\"WaitingForUser\""));
    assert!(!result.event_jsonl.contains("\"to_state\":\"Failed\""));
    assert!(!result.event_jsonl.contains("visible_finalizer"));
    assert!(!result.event_jsonl.contains("Tool limit structured stop"));
    assert!(!result.event_jsonl.contains("\"agent.loop_incomplete\""));
}

#[test]
fn native_agent_loop_v2_replaces_tool_budget_refusal_finalizer_with_evidence() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-tool-budget-refusal-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "ResearchCode tool budget fixture\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_map","name":"repo_map","input":{"root":"."}}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"由于工具调用次数已达上限，无法继续读取具体源代码。"}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_tool_budget_refusal".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Inspect the repository evidence and explain the runtime architecture."
                .to_string(),
            max_tokens: 1024,
            max_iterations: 8,
            max_tool_calls: 1,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForUser);
    assert_eq!(result.model_call_count, 1);
    assert_eq!(result.tool_call_count, 1);
    assert!(result.event_jsonl.contains("\"reason\":\"max_tool_calls\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.loop_stopped\""));
    assert!(result.event_jsonl.contains("\"category\":\"turn_budget\""));
    assert!(result
        .event_jsonl
        .contains("\"to_state\":\"WaitingForUser\""));
    assert!(!result
        .event_jsonl
        .contains("\"event_type\":\"assistant.message\""));
    assert!(!result.event_jsonl.contains("无法继续读取具体源代码"));
}

#[test]
fn native_agent_loop_v2_read_only_uses_requested_tool_budget() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-readonly-budget-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    for index in 0..9 {
        fs::write(
            root.join(format!("file_{index}.txt")),
            format!("file {index}\n"),
        )
        .unwrap();
    }
    let artifact_root = root.join("artifacts");
    let mut responses = Vec::new();
    for index in 0..9 {
        responses.push(LiveHttpResponse {
                status_code: 200,
                body: format!(
                    "data: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"tool_use\",\"id\":\"toolu_read_{index}\",\"name\":\"file_read\",\"input\":{{\"path\":\"file_{index}.txt\"}}}}}}\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\ndata: [DONE]"
                ),
            });
    }
    responses.push(LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Read-only analysis completed after reading more than eight files."}}
data: [DONE]"#
                .to_string(),
        });
    let transport = ScriptedLiveHttpTransport::new(responses);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_readonly_budget".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Inspect the repository evidence and explain the runtime architecture."
                .to_string(),
            max_tokens: 1024,
            max_iterations: 16,
            max_tool_calls: 12,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.tool_call_count, 9);
    assert!(result.tool_call_count > 8);
    assert!(!result.event_jsonl.contains("\"reason\":\"max_tool_calls\""));
    assert!(result
        .event_jsonl
        .contains("Read-only analysis completed after reading more than eight files"));
}

#[test]
fn native_agent_loop_v2_suppresses_duplicate_observation_calls() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-duplicate-observation-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_cargo_1","name":"file_read","input":{"path":"Cargo.toml"}}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_cargo_2","name":"file_read","input":{"path":"Cargo.toml"}}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Final answer from cached Cargo.toml evidence."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_duplicate_observation".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Inspect Cargo.toml and explain this workspace.".to_string(),
            max_tokens: 1024,
            max_iterations: 5,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.tool_call_count, 1);
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"tool.duplicate_observation_suppressed\""));
    assert!(result.event_jsonl.contains("\"skipped\":true"));
    assert!(result
        .event_jsonl
        .contains("\"reason\":\"duplicate_observation\""));
    assert!(result.event_jsonl.contains("final answer/report now"));
    assert!(!result.event_jsonl.contains("\"tool_results\":2"));
    assert!(!result.event_jsonl.contains("runtime.duplicate_suppression"));
    assert!(result
        .event_jsonl
        .contains("Final answer from cached Cargo.toml evidence"));
}

#[test]
fn native_agent_loop_v2_suppresses_covered_file_read_ranges() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-covered-read-range-{nonce}"
    ));
    fs::create_dir_all(root.join("plan")).unwrap();
    let plan = (0..260)
        .map(|index| format!("line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(root.join("plan/roadmap.md"), plan).unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_plan_broad","name":"file_read","input":{"path":"plan/roadmap.md","offset":0,"limit":200,"max_bytes":16000}}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_plan_overlap","name":"file_read","input":{"path":"./plan/roadmap.md","offset":50,"limit":25,"max_bytes":8000}}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Final answer from covered plan evidence."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_covered_file_range".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Inspect the plan once, then continue implementation.".to_string(),
            max_tokens: 1024,
            max_iterations: 5,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.tool_call_count, 1);
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"tool.duplicate_observation_suppressed\""));
    assert!(result
        .event_jsonl
        .contains("covered_by=file.read:plan/roadmap.md"));
    assert!(result
        .event_jsonl
        .contains("Final answer from covered plan evidence"));
}

#[test]
fn live_visible_stream_deltas_are_coalesced_before_emit() {
    let mut session = AgentSession::new("proj", "sess_stream_coalesce", "task").unwrap();
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .unwrap();
    let mut emitted_event_count = 0usize;
    let mut event_sink: Option<&mut dyn FnMut(&str)> = None;
    let mut stream_processor = StreamProcessor::default();

    for _ in 0..5 {
        let emitted = record_live_visible_stream_event(
            LiveHttpStreamEvent::VisibleTextDelta("small ".to_string()),
            &mut session,
            "stream_1",
            "deepseek",
            &mut emitted_event_count,
            &mut event_sink,
            &mut stream_processor,
        )
        .unwrap();
        assert!(!emitted);
    }
    assert!(!session
        .event_log()
        .export_jsonl()
        .contains("\"event_type\":\"model.stream_delta\""));

    let (mut pending_content, mut pending_content_chunks) = stream_processor.take_pending_content();
    flush_live_content_stream_event(
        &mut session,
        "stream_1",
        "deepseek",
        &mut emitted_event_count,
        &mut event_sink,
        &mut pending_content,
        &mut pending_content_chunks,
    )
    .unwrap();
    let jsonl = session.event_log().export_jsonl();
    assert_eq!(
        jsonl
            .matches("\"event_type\":\"model.stream_delta\"")
            .count(),
        1
    );
    assert!(jsonl.contains("\"event_type\":\"runtime.stream.coalesced\""));
}

#[test]
fn live_visible_stream_fallback_discards_failed_attempt_deltas() {
    let mut session =
        AgentSession::new("proj", "sess_stream_fallback_dirty_delta", "task").unwrap();
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .unwrap();
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 400,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"DIRTY_ANTHROPIC_DELTA"}}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"content":"CLEAN_OPENAI_DELTA"}}]}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let request = PreparedModelHttpRequest {
            method: "POST".to_string(),
            url: "https://api.example.test/v1/messages".to_string(),
            authorization_env: "PATH".to_string(),
            body_json: r#"{"model":"deepseek-test","max_tokens":64,"stream":true,"system":"system","messages":[{"role":"user","content":"hello"}],"tools":[],"tool_choice":{"type":"auto"}}"#.to_string(),
            stream: true,
        };
    let mut emitted_event_count = 0usize;
    let mut fallback = DualProtocolFallback::new();
    let mut captured_events = Vec::new();
    let mut sink = |event: &str| captured_events.push(event.to_string());
    let mut event_sink: Option<&mut dyn FnMut(&str)> = Some(&mut sink);

    let (response, record_content_deltas) = send_with_live_visible_stream_events(
        &transport,
        &request,
        &mut session,
        "stream_fallback",
        &NativeModelFamily::DeepSeek,
        &mut emitted_event_count,
        &mut event_sink,
        None,
        None,
        Some(&mut fallback),
        &AtomicBool::new(false),
    )
    .unwrap();

    assert_eq!(response.status_code, 200);
    assert!(!record_content_deltas);
    let jsonl = session.event_log().export_jsonl();
    assert!(jsonl.contains("CLEAN_OPENAI_DELTA"));
    assert!(!jsonl.contains("DIRTY_ANTHROPIC_DELTA"));
    let captured = captured_events.join("\n");
    assert!(captured.contains("CLEAN_OPENAI_DELTA"));
    assert!(!captured.contains("DIRTY_ANTHROPIC_DELTA"));
    assert_eq!(fallback.fallback_count, 1);
}

#[test]
fn native_agent_loop_v2_preserves_anthropic_text_block_before_tool_call() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-stream-preamble-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "hello\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Let我先检查项目中的现有计划和相关设计文档。"}}
data: {"type":"content_block_stop","index":0}
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_list","name":"file_list_directory","input":{}}}
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\".\"}"}}
data: {"type":"content_block_stop","index":1}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Final answer after directory evidence."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_stream_preamble".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Inspect this workspace.".to_string(),
            max_tokens: 1024,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.tool_call_count, 1);
    assert!(!result
        .event_jsonl
        .contains("\"event_type\":\"runtime.stream.preamble_suppressed\""));
    assert!(result.event_jsonl.lines().any(|line| {
        line.contains("\"event_type\":\"runtime.stream.narration\"")
            && line.contains("Let我先检查项目中的现有计划和相关设计文档")
    }));
    assert!(result.event_jsonl.lines().any(|line| {
        line.contains("\"event_type\":\"model.stream_delta\"")
            && line.contains("Let我先检查项目中的现有计划和相关设计文档")
    }));
    assert!(!result.event_jsonl.lines().any(|line| {
        line.contains("\"event_type\":\"assistant.message\"")
            && line.contains("Let我先检查项目中的现有计划和相关设计文档")
    }));
    assert!(result
        .event_jsonl
        .contains("Final answer after directory evidence"));
}

#[test]
fn native_agent_loop_v2_allows_expanded_tree_observation_bounds() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-expanded-tree-{nonce}"
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn answer() -> u8 { 42 }\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_tree_small","name":"file_list_tree","input":{"path":".","depth":2,"max_entries":20}}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_tree_large","name":"file_list_tree","input":{"path":".","depth":3,"max_entries":200}}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Expanded tree evidence was accepted, then I produced the final explanation."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_expanded_tree".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Inspect this workspace with progressively broader tree views.".to_string(),
            max_tokens: 1024,
            max_iterations: 5,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.tool_call_count, 2);
    assert!(!result
        .event_jsonl
        .contains("\"event_type\":\"tool.duplicate_observation_suppressed\""));
    assert!(result
        .event_jsonl
        .contains("Expanded tree evidence was accepted"));
}

#[test]
fn native_tool_result_continuation_uses_provider_names_and_openai_call_ids() {
    let batch = vec![(
        "toolu_v2_0_0".to_string(),
        "file.read".to_string(),
        "{\"path\":\"README.md\"}".to_string(),
        crate::tool_execution::ToolExecutionResult {
            tool_call_id: "native_loop_v2_tool_0_0".to_string(),
            tool_id: "file.read".to_string(),
            ok: true,
            preview: "README content".to_string(),
            detail_json: "{\"ok\":true}".to_string(),
            exit_code: None,
        },
    )];
    let view = ContinuationView::from_legacy_batch(batch);
    let openai = build_native_tool_result_continuation_request(
        &NativeProviderEndpoint::deepseek_v4_flash_openai(),
        "Read README.md",
        &view,
        256,
        "[]",
        None,
    )
    .unwrap();
    assert!(openai.body_json.contains("\"id\":\"call_toolu_v2_0_0\""));
    assert!(openai
        .body_json
        .contains("\"tool_call_id\":\"call_toolu_v2_0_0\""));
    assert!(openai.body_json.contains("\"name\":\"file_read\""));
    assert!(openai.body_json.contains("\"reasoning_content\""));
    assert!(openai.body_json.contains("placeholder retained"));
    assert!(!openai.body_json.contains("\"name\":\"file.read\""));

    let anthropic = build_native_tool_result_continuation_request(
        &NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
        "Read README.md",
        &view,
        256,
        r#"[{"name":"file_read","description":"Read","input_schema":{"type":"object"}}]"#,
        None,
    )
    .unwrap();
    assert!(anthropic.body_json.contains("\"id\":\"toolu_v2_0_0\""));
    assert!(anthropic.body_json.contains("\"name\":\"file_read\""));
    assert!(!anthropic.body_json.contains("\"name\":\"file.read\""));
}

#[test]
fn native_tool_result_continuation_preserves_provider_openai_call_id() {
    let batch = vec![(
        "call_provider_readme".to_string(),
        "file.read".to_string(),
        "{\"path\":\"README.md\"}".to_string(),
        crate::tool_execution::ToolExecutionResult {
            tool_call_id: "native_loop_v2_tool_0_0".to_string(),
            tool_id: "file.read".to_string(),
            ok: true,
            preview: "README content".to_string(),
            detail_json: "{\"ok\":true}".to_string(),
            exit_code: None,
        },
    )];
    let view = ContinuationView::from_legacy_batch(batch);
    let openai = build_native_tool_result_continuation_request(
        &NativeProviderEndpoint::deepseek_v4_flash_openai(),
        "Read README.md",
        &view,
        256,
        "[]",
        None,
    )
    .unwrap();
    assert!(openai.body_json.contains("\"id\":\"call_provider_readme\""));
    assert!(openai
        .body_json
        .contains("\"tool_call_id\":\"call_provider_readme\""));
    assert!(!openai.body_json.contains("call_toolu_v2_0_0"));
}

#[test]
fn native_tool_evidence_continuation_keeps_tools_without_tool_result_replay() {
    let batch = vec![(
        "toolu_evidence_0".to_string(),
        "file.list_tree".to_string(),
        "{\"path\":\".\"}".to_string(),
        crate::tool_execution::ToolExecutionResult {
            tool_call_id: "native_loop_tool_0".to_string(),
            tool_id: "file.list_tree".to_string(),
            ok: true,
            preview: "tree lines=10 files=8 omitted=0 root=.".to_string(),
            detail_json: "{\"tree_lines\":[\"Cargo.toml\",\"src/\"]}".to_string(),
            exit_code: None,
        },
    )];
    let view = ContinuationView::from_legacy_batch(batch);
    let request = build_native_tool_evidence_continuation_request(
            &NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
            "Explain this folder.",
            &view,
            256,
            r#"[{"name":"file_read","description":"Read","input_schema":{"type":"object"}},{"name":"file_list_tree","description":"Tree","input_schema":{"type":"object"}}]"#,
            &NativeAgentToolExposure::ReadOnly,
            "Continue safely.",
        )
        .unwrap();
    assert!(request.body_json.contains("\"tools\""));
    assert!(request.body_json.contains("Already Executed Tool Evidence"));
    assert!(request.body_json.contains("file.list_tree"));
    assert!(request.body_json.contains("tool_call_id=toolu_evidence_0"));
    assert!(request
        .body_json
        .contains("result_tool_call_id=native_loop_tool_0"));
    assert!(!request.body_json.contains("\"tool_use\""));
    assert!(!request.body_json.contains("\"tool_result\""));
}

#[test]
fn structured_tool_result_content_truncates_large_detail_for_provider_replay() {
    let result = crate::tool_execution::ToolExecutionResult {
        tool_call_id: "native_loop_tool_big".to_string(),
        tool_id: "file.read".to_string(),
        ok: true,
        preview: "large file read".to_string(),
        detail_json: format!(
            "{{\"path\":\"big.rs\",\"content\":\"{}\"}}",
            "x".repeat(20_000)
        ),
        exit_code: None,
    };
    let content = structured_tool_result_content(&result);
    assert!(content.len() < 8_000);
    assert!(content.contains("\"detail_truncated\":true"));
    assert!(content.contains("\"artifact_ref\""));
}

#[test]
fn native_agent_loop_v2_empty_visible_stops_structurally_without_tool_results() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-empty-visible-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    let artifact_root = root.join("artifacts");
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = true;
    endpoint.api_key_env = "PATH".to_string();
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{}}]}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"content":"Recovered with a visible final answer and no tool_result replay requirement."}}]}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_empty_visible".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint,
            prompt: "Explain this project.".to_string(),
            max_tokens: 1024,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Failed);
    assert_eq!(result.final_state, AgentState::Failed);
    assert_eq!(result.model_call_count, 1);
    assert_eq!(result.tool_call_count, 0);
    assert!(result.event_jsonl.contains("empty_visible_response"));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.loop_stopped\""));
    assert!(result
        .event_jsonl
        .contains("\"category\":\"no_visible_answer\""));
    assert!(!result
        .event_jsonl
        .contains("Recovered with a visible final answer"));
    assert!(!result.event_jsonl.contains("visible_finalizer"));
    assert!(!result.event_jsonl.contains("repeated_tool_batch"));
    assert!(!result
        .event_jsonl
        .contains("tool_calls and tool_results are required"));
}

#[test]
fn native_agent_loop_v2_empty_visible_after_tools_stops_without_visible_finalizer() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-empty-visible-after-tools-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\nA small workspace.\n").unwrap();
    let artifact_root = root.join("artifacts");
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = true;
    endpoint.api_key_env = "PATH".to_string();
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_readme","function":{"name":"file_read","arguments":"{\"path\":\"README.md\"}"}}]}}]}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{}}]}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"content":"Final-like answer: README.md describes a small Demo workspace."}}]}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_empty_visible_after_tools".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint,
            prompt: "Explain this project.".to_string(),
            max_tokens: 1024,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Failed);
    assert_eq!(result.final_state, AgentState::Failed);
    assert_eq!(result.model_call_count, 2);
    assert_eq!(result.tool_call_count, 1);
    assert!(result.event_jsonl.contains("empty_visible_response"));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.loop_stopped\""));
    assert!(result
        .event_jsonl
        .contains("\"category\":\"no_visible_answer\""));
    assert!(!result.event_jsonl.contains("visible_finalizer"));
    assert!(!result.event_jsonl.contains("agent.final_answer.called"));
    assert!(!result.event_jsonl.contains("agent_final_answer"));
    assert!(!result
        .event_jsonl
        .contains("Final-like answer: README.md describes a small Demo workspace."));
    assert!(!result
        .event_jsonl
        .contains("本轮已完成，但模型没有返回可展示文本"));
}

#[test]
fn native_agent_loop_v2_visible_only_transition_after_tools_is_telemetry_only() {
    // Repro of the production "preamble stop" failure mode:
    //   1. Model emits a tool_call (file_read).
    //   2. Tool result auto-injected.
    //   3. Model emits visible text "好的，让我查看当前项目的完整状态..." with NO tool_calls.
    //
    // Current contract: transition detection is telemetry only. The runtime
    // must not let a string heuristic force hidden loop continuation; only
    // structured tool calls can continue tool execution.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-visible-transition-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\nA small workspace.\n").unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
    let artifact_root = root.join("artifacts");
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = true;
    endpoint.api_key_env = "PATH".to_string();
    let transport = ScriptedLiveHttpTransport::new(vec![
            // Iteration 1: model calls file_read.
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_readme","function":{"name":"file_read","arguments":"{\"path\":\"README.md\"}"}}]}}]}
data: [DONE]"#
                    .to_string(),
            },
            // Iteration 2: visible-only transition statement, NO tool_calls.
            // This is the screenshot bug: the model says "let me check ..."
            // and forgets to actually issue the next tool call.
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"content":"好的，让我查看当前项目的完整状态，确认已完成的步骤和下一步工作。"}}]}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_visible_transition".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint,
            prompt: "Explain this project.".to_string(),
            max_tokens: 1024,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 2);
    assert_eq!(result.tool_call_count, 1);
    // The transition-detection telemetry event must fire, but it must not be a
    // loop-control authority.
    assert!(
        result
            .event_jsonl
            .contains("agent.visible_only_transition_detected"),
        "expected agent.visible_only_transition_detected event; jsonl: {}",
        result.event_jsonl
    );
    assert!(
        result.event_jsonl.contains("\"action\":\"telemetry_only\""),
        "expected telemetry-only action; jsonl: {}",
        result.event_jsonl
    );
    assert!(
        result.event_jsonl.contains("\"loop_control\":false"),
        "transition heuristic must not control the loop; jsonl: {}",
        result.event_jsonl
    );
    assert!(
        result
            .event_jsonl
            .contains("好的，让我查看当前项目的完整状态"),
        "transition narration text should not be swallowed; jsonl: {}",
        result.event_jsonl
    );
    assert!(
        !result
            .event_jsonl
            .contains("visible_only_transition_after_tool_work"),
        "transition narration must not force structured-stop fallback; jsonl: {}",
        result.event_jsonl
    );
    assert!(
        !result.event_jsonl.contains("Cargo.toml"),
        "telemetry-only transition detection must not force another model/tool iteration; jsonl: {}",
        result.event_jsonl
    );
    // The visible text is preserved as the ordinary assistant answer, not a
    // forced finalization tool or hidden loop continuation.
    assert!(!result.event_jsonl.contains("agent.final_answer.called"));
    assert!(result
        .event_jsonl
        .contains("好的，让我查看当前项目的完整状态"));
    assert!(result
        .event_jsonl
        .contains("\"reason\":\"model_visible_answer\""));
    assert_eq!(
        event_count(&result.event_jsonl, "agent.loop_state.terminal"),
        1
    );
    assert!(result.event_jsonl.contains("\"status\":\"completed\""));
    assert!(result.event_jsonl.contains("\"category\":\"model_answer\""));
}

#[test]
fn native_agent_loop_v2_output_token_truncation_continues_with_larger_budget() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-output-truncation-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\nA small workspace.\n").unwrap();
    let artifact_root = root.join("artifacts");
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = true;
    endpoint.api_key_env = "PATH".to_string();
    let transport = ScriptedLiveHttpTransport::new(vec![
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_readme","function":{"name":"file_read","arguments":"{\"path\":\"README.md\"}"}}]}}]}
data: [DONE]"#
                .to_string(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"choices":[{"delta":{"content":"Part one: README describes"}}]}
data: {"choices":[{"finish_reason":"length","delta":{}}],"usage":{"prompt_tokens":64,"completion_tokens":1024,"reasoning_tokens":0}}
data: [DONE]"#
                .to_string(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"choices":[{"delta":{"content":" Part two: a small workspace."}}]}
data: {"choices":[{"finish_reason":"stop","delta":{}}],"usage":{"prompt_tokens":68,"completion_tokens":12,"reasoning_tokens":0}}
data: [DONE]"#
                .to_string(),
        },
    ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_output_truncation".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint,
            prompt: "Read README.md and summarize completely.".to_string(),
            max_tokens: 1024,
            max_iterations: 5,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert!(result.model_call_count >= 3);
    assert!(result.event_jsonl.contains("\"stop_reason\":\"length\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.recovery.output_truncated\""));
    assert!(result.event_jsonl.contains("\"next_max_tokens\":65536"));
    assert!(result.event_jsonl.contains("Part one: README describes"));
    assert!(result.event_jsonl.contains("Part two: a small workspace."));
    let sent = transport.sent_requests();
    assert!(sent
        .iter()
        .any(|request| request.body_json.contains("\"max_tokens\":65536")));
    assert!(sent.iter().any(|request| request
        .body_json
        .contains("previous assistant output was cut off")));
}

#[test]
fn native_agent_loop_v2_legacy_final_answer_tool_call_does_not_stop_loop() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-legacy-final-answer-tool-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\nA small workspace.\n").unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
    let artifact_root = root.join("artifacts");
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = true;
    endpoint.api_key_env = "PATH".to_string();
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_readme","function":{"name":"file_read","arguments":"{\"path\":\"README.md\"}"}}]}}]}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_legacy_final","function":{"name":"agent_final_answer","arguments":"{\"message\":\"正在收集更多代码文件进行审核，请稍候...\",\"status\":\"completed\"}"}}]}}]}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_cargo","function":{"name":"file_read","arguments":"{\"path\":\"Cargo.toml\"}"}}]}}]}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"content":"README describes a small Demo workspace and Cargo.toml defines a workspace."}}]}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_legacy_final_answer_tool".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint,
            prompt: "Explain this project.".to_string(),
            max_tokens: 1024,
            max_iterations: 5,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert!(result.model_call_count >= 4);
    assert!(result.tool_call_count >= 3);
    assert!(
        result.event_jsonl.contains("\"unknown_tool_count\":1")
            || result.event_jsonl.contains("tool.name.unknown"),
        "expected legacy tool call to be recorded as an unknown unavailable tool; jsonl: {}",
        result.event_jsonl
    );
    assert!(
        result.event_jsonl.contains("agent_final_answer"),
        "expected legacy provider tool name to be preserved in recovery evidence; jsonl: {}",
        result.event_jsonl
    );
    assert!(
        !result.event_jsonl.contains("agent.final_answer.called"),
        "legacy final-answer tool must never terminate the loop; jsonl: {}",
        result.event_jsonl
    );
    assert!(result.event_jsonl.contains("Cargo.toml"));
    assert!(result
        .event_jsonl
        .contains("README describes a small Demo workspace and Cargo.toml defines a workspace."));
}

#[test]
fn native_agent_loop_v2_visible_only_without_tool_work_still_accepts_as_final_answer() {
    // Negative case: when the turn has NO prior tool work (e.g. the user asks
    // a simple factual question and the model answers directly), a short
    // visible-only response must still be accepted as the final answer. This
    // is the regression guard for the transition-detection fix above.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-direct-answer-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    let artifact_root = root.join("artifacts");
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = true;
    endpoint.api_key_env = "PATH".to_string();
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: r#"data: {"choices":[{"delta":{"content":"The answer is 4."}}]}
data: [DONE]"#
            .to_string(),
    }]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_direct_answer".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint,
            prompt: "What is 2 + 2?".to_string(),
            max_tokens: 256,
            max_iterations: 4,
            max_tool_calls: 4,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.model_call_count, 1);
    assert_eq!(result.tool_call_count, 0);
    // Must NOT have routed through the transition-detection fallback.
    assert!(
        !result
            .event_jsonl
            .contains("agent.visible_only_transition_detected"),
        "no prior tool work; must accept as direct final answer"
    );
    // Must have recorded the visible content as final answer normally.
    assert!(result.event_jsonl.contains("The answer is 4."));
}

#[test]
fn native_agent_loop_v2_deepseek_anthropic_prefers_structured_tool_results_after_tools() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-structured-continuation-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_tree","name":"file_list_tree","input":{"path":".","max_entries":20}}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"这个工具是一个 Rust workspace；运行路径从 CLI 进入 runtime agent loop，再由工具 manifest 暴露只读文件工具。"}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_structured_continuation".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Inspect the workspace and explain the runtime flow.".to_string(),
            max_tokens: 1024,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 2);
    assert_eq!(result.tool_call_count, 1);
    assert!(result
        .event_jsonl
        .contains("provider_tool_result_continuation"));
    assert!(!result.event_jsonl.contains("plain_evidence_continuation"));
    assert!(result.event_jsonl.contains("Rust workspace"));
    assert!(!result.event_jsonl.contains("model.http_failure_recovered"));
    assert!(!result
        .event_jsonl
        .contains("tool_result_continuation_http_failure"));
}

#[test]
fn native_agent_loop_v2_initial_http_400_fails_and_releases_turn() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-initial-http-400-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 400,
        body: r#"{"error":{"message":"bad request from provider"}}"#.to_string(),
    }]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_initial_http_400".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Delete the test file if it is safe.".to_string(),
            max_tokens: 1024,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::CodeEdit,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Failed);
    assert_eq!(result.final_state, AgentState::Failed);
    assert_eq!(result.model_call_count, 1);
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"model.call_blocked\""));
    assert!(result.event_jsonl.contains("\"gate\":\"http_status_400\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.loop_stopped\""));
    assert_eq!(
        event_payload_string(&result.event_jsonl, "agent.loop_stopped", "status"),
        "failed"
    );
    assert_eq!(
        event_payload_string(&result.event_jsonl, "agent.loop_stopped", "category"),
        "provider_failure"
    );
    assert_eq!(
        event_payload_string(&result.event_jsonl, "agent.loop_stopped", "next_action"),
        "surface_failure_and_release_turn"
    );
    assert!(result.event_jsonl.contains("\"to_state\":\"Failed\""));
}

#[test]
fn native_agent_loop_v2_recovers_http_400_after_tool_results() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-http-400-recovery-{nonce}"
    ));
    fs::create_dir_all(root.join("crates/cli/src")).unwrap();
    fs::write(
        root.join("crates/cli/src/main.rs"),
        format!("{}\n", "fn main() {}".repeat(20_000)),
    )
    .unwrap();
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_big_read","name":"file_read","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"crates/cli/src/main.rs\",\"max_bytes\":80000}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 400,
                body: "bad request: context/tool_result too large".to_string(),
            },
            LiveHttpResponse {
                status_code: 400,
                body: "bad request: compact evidence still too large".to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_http_400_recovery".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint: live_deepseek_endpoint(),
            prompt: "Inspect a large source file and explain the runtime flow.".to_string(),
            max_tokens: 1024,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Failed);
    assert_eq!(result.final_state, AgentState::Failed);
    assert_eq!(result.model_call_count, 3);
    assert_eq!(result.tool_call_count, 1);
    assert!(result.event_jsonl.contains("model.http_failure_recovered"));
    assert!(result
        .event_jsonl
        .contains("retry_plain_evidence_continuation"));
    assert!(result.event_jsonl.contains("stop_with_structured_failure"));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.loop_stopped\""));
    assert!(result
        .event_jsonl
        .contains("\"category\":\"provider_failure\""));
    assert!(result.event_jsonl.contains("crates/cli/src/main.rs"));
    assert!(!result.event_jsonl.contains("visible_finalizer"));
}

#[test]
fn native_agent_loop_v2_retries_plain_evidence_after_tool_result_http_400() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-http-400-evidence-retry-{nonce}"
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    let artifact_root = root.join("artifacts");
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = true;
    endpoint.api_key_env = "PATH".to_string();
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_readme","function":{"name":"file_read","arguments":"{\"path\":\"README.md\"}"}}]}}]}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 400,
                body: "bad request: rejected tool replay".to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"content":"Recovered through compact evidence continuation and produced a real answer."}}]}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_http_400_evidence_retry".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint,
            prompt: "Explain this folder.".to_string(),
            max_tokens: 1024,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 3);
    assert_eq!(result.tool_call_count, 1);
    assert!(result.event_jsonl.contains("model.http_failure_recovered"));
    assert!(result
        .event_jsonl
        .contains("retry_plain_evidence_continuation"));
    assert!(result
        .event_jsonl
        .contains("model.http_failure_recovery_succeeded"));
    assert!(result
        .event_jsonl
        .contains("Recovered through compact evidence continuation"));
    assert!(!result.event_jsonl.contains("缩小路径"));
}

#[test]
fn native_agent_loop_v2_recovers_trace_mixed_toolstorm() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-toolstorm-trace-{nonce}"
    ));
    fs::create_dir_all(root.join("crates/runtime/src")).unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    fs::write(
        root.join("crates/runtime/src/native_agent_loop.rs"),
        "pub fn loop_fixture() {}\n",
    )
    .unwrap();
    let artifact_root = root.join("artifacts");
    let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
    endpoint.live_calls_enabled_by_default = true;
    endpoint.api_key_env = "PATH".to_string();
    let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_todo","function":{"name":"todo_write","arguments":"{\"items\":[{\"content\":\"inspect runtime\",\"status\":\"in_progress\"}]}"}},{"index":1,"id":"call_read_many","function":{"name":"read_files","arguments":"{\"files\":[\"README.md\",\"crates/runtime/src/native_agent_loop.rs\"]}"}},{"index":2,"id":"call_bash","function":{"name":"bash","arguments":"{\"command\":\"find.-maxdepth3-typef|sort\"}"}}]}}]}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"content":"Recovered from todo/read_files/bash toolstorm and produced a final answer."}}]}
data: [DONE]"#
                    .to_string(),
            },
        ]);
    let result = run_native_agent_loop_v2_deepseek(
        &transport,
        NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_v2_toolstorm_trace".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: root.clone(),
            artifact_root,
            endpoint,
            prompt: "Inspect this project read-only.".to_string(),
            max_tokens: 1024,
            max_iterations: 8,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        },
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 2);
    assert_eq!(result.tool_call_count, 3);
    assert!(result.event_jsonl.contains("\"tool_id\":\"todo.write\""));
    assert!(result.event_jsonl.contains("\"tool_id\":\"file.read\""));
    assert!(result.event_jsonl.contains("\"tool_id\":\"shell.command\""));
    assert!(!result
        .event_jsonl
        .contains("\"event_type\":\"tool.alias_shell_list_to_directory_tool\""));
    assert!(result.event_jsonl.contains("README.md"));
    assert!(result
        .event_jsonl
        .contains("crates/runtime/src/native_agent_loop.rs"));
    assert!(result
        .event_jsonl
        .contains("Recovered from todo/read_files/bash toolstorm"));
    assert!(!result.event_jsonl.contains("SCHEMA_VALIDATION_FAILED"));
    assert!(!result
        .event_jsonl
        .contains("tool_calls and tool_results are required"));
}

#[test]
fn native_agent_loop_v2_routes_plan_enter_to_plan_approval() {
    let result = run_scripted_native_agent_loop_v2_plan_enter_fixture().unwrap();
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForPlanApproval);
    assert_eq!(result.model_call_count, 1);
    assert_eq!(result.tool_call_count, 1);
    assert!(result.event_jsonl.contains("\"plan.mode_entered\""));
    assert!(result.event_jsonl.contains("\"plan.approval_requested\""));
    assert_eq!(
        event_count(&result.event_jsonl, "agent.loop_state.terminal"),
        1
    );
    assert!(result
        .event_jsonl
        .contains("\"reason\":\"plan_approval_pending\""));
    assert!(result
        .event_jsonl
        .contains("\"category\":\"plan_approval\""));
    assert!(!result.event_jsonl.contains("\"permission.requested\""));
}

#[test]
fn native_agent_loop_v2_routes_ask_user_to_waiting_for_user() {
    let result = run_scripted_native_agent_loop_v2_ask_user_fixture().unwrap();
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForUser);
    assert_eq!(result.model_call_count, 1);
    assert_eq!(result.tool_call_count, 1);
    assert!(result.event_jsonl.contains("\"tool_id\":\"ask_user\""));
    assert!(result.event_jsonl.contains("\"user.question_requested\""));
    assert!(result
        .event_jsonl
        .contains("Which file should I inspect first"));
    assert_eq!(
        event_count(&result.event_jsonl, "agent.loop_state.terminal"),
        1
    );
    assert!(result
        .event_jsonl
        .contains("\"reason\":\"waiting_for_user\""));
    assert!(result.event_jsonl.contains("\"category\":\"ask_user\""));
    assert!(!result.event_jsonl.contains("\"permission.requested\""));
}

#[test]
fn native_agent_loop_blocks_with_event_log_when_permission_is_external() {
    let result = run_scripted_native_agent_loop_external_block_fixture().unwrap();
    assert_eq!(result.status, NativeAgentLoopStatus::Blocked);
    assert_eq!(result.final_state, AgentState::WaitingForToolApproval);
    assert!(result.pending_tool.is_some());
    assert!(result.event_jsonl.contains("\"permission.requested\""));
    assert_eq!(
        event_count(&result.event_jsonl, "agent.loop_state.terminal"),
        1
    );
    assert!(result
        .event_jsonl
        .contains("\"reason\":\"pending_permission\""));
    assert!(result.event_jsonl.contains("\"category\":\"permission\""));
    assert!(!result.event_jsonl.contains("\"permission.decided\""));
}

#[test]
fn native_agent_loop_uses_provided_permission_decisions() {
    let fixture = run_scripted_native_agent_loop_provided_permission_fixture().unwrap();
    let result = fixture.loop_result;
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert!(result.event_jsonl.contains("\"permission.decided\""));
    assert!(fixture.final_file_hash.starts_with("fnv64_"));
}

#[test]
fn native_agent_loop_resumes_from_event_log_after_external_decision() {
    let fixture = run_scripted_native_agent_loop_external_resume_fixture().unwrap();
    let result = fixture.loop_result;
    assert_eq!(result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(result.final_state, AgentState::Completed);
    assert_eq!(result.model_call_count, 0);
    assert_eq!(result.tool_call_count, 1);
    assert!(result.event_jsonl.contains("\"permission.decided\""));
    assert!(result
        .event_jsonl
        .contains("\"event_type\":\"agent.recovery.started\""));
    assert!(result.event_jsonl.contains("external_decision_resume"));
    assert!(result.event_jsonl.contains("\"tool_id\":\"file.write\""));
    assert!(fixture.final_file_hash.starts_with("fnv64_"));
}

#[test]
fn native_agent_loop_external_decision_package_round_trips() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let package_dir =
        std::env::temp_dir().join(format!("researchcode-native-agent-loop-package-{nonce}"));
    let package = write_scripted_native_agent_loop_external_decision_package(&package_dir).unwrap();
    assert!(package.event_log_path.exists());
    assert!(package.pending_tool_path.exists());
    assert!(package.manifest_path.exists());
    assert_eq!(
        package.blocked_result.status,
        NativeAgentLoopStatus::Blocked
    );
    let resume = resume_scripted_native_agent_loop_external_decision_package(
        &package_dir,
        PermissionDecisionKind::AllowOnce,
    )
    .unwrap();
    assert_eq!(resume.loop_result.status, NativeAgentLoopStatus::Completed);
    assert_eq!(resume.loop_result.model_call_count, 0);
    assert_eq!(resume.loop_result.tool_call_count, 1);
    assert!(resume
        .loop_result
        .event_jsonl
        .contains("\"event_type\":\"agent.recovery.started\""));
    assert!(resume.event_log_path.exists());
    assert!(resume.final_file_hash.starts_with("fnv64_"));
    let final_text = fs::read_to_string(package.workspace_root.join("src/parser.ts")).unwrap();
    assert_eq!(final_text, "export const retry_count = 5;\n");
    let _ = fs::remove_dir_all(package_dir);
}

#[test]
fn native_agent_loop_resume_does_not_rerun_completed_pending_tool() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let package_dir = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-exactly-once-{nonce}"
    ));
    let package = write_scripted_native_agent_loop_external_decision_package(&package_dir).unwrap();
    let pending_tool = package.blocked_result.pending_tool.clone().unwrap();
    let first_resume = resume_scripted_native_agent_loop_external_decision_package(
        &package_dir,
        PermissionDecisionKind::AllowOnce,
    )
    .unwrap();
    let second_resume = resume_native_agent_loop_after_external_decision(
        &ScriptedLiveHttpTransport::new(vec![]),
        NativeAgentLoopResumeRequest {
            previous_event_jsonl: first_resume.loop_result.event_jsonl,
            workspace_root: package.workspace_root.clone(),
            artifact_root: package.artifact_root.clone(),
            pending_tool,
            decision: PermissionDecisionKind::AllowOnce,
        },
    )
    .unwrap();
    assert_eq!(second_resume.status, NativeAgentLoopStatus::Completed);
    assert_eq!(second_resume.tool_call_count, 0);
    assert!(second_resume
        .event_jsonl
        .contains("\"event_type\":\"agent.resume.exactly_once_reused_recorded_result\""));
    let final_text = fs::read_to_string(package.workspace_root.join("src/parser.ts")).unwrap();
    assert_eq!(final_text, "export const retry_count = 5;\n");
    let _ = fs::remove_dir_all(package_dir);
}

#[test]
fn suggested_manifest_tool_does_not_replace_read_with_edit() {
    let allowed_tools = BTreeSet::from([
        "file.edit".to_string(),
        "file.list_directory".to_string(),
        "git.status".to_string(),
    ]);
    assert_eq!(suggested_manifest_tool(&allowed_tools, "file.read"), None);

    let allowed_tools = BTreeSet::from([
        "file.edit".to_string(),
        "file.read".to_string(),
        "file.list_directory".to_string(),
    ]);
    assert_eq!(
        suggested_manifest_tool(&allowed_tools, "file.read"),
        Some("file.read".to_string())
    );
}

#[test]
fn path_not_found_error_signature_includes_path() {
    let first = crate::tool_execution::ToolExecutionResult {
        tool_call_id: "a".to_string(),
        tool_id: "file.read".to_string(),
        ok: false,
        preview: "missing".to_string(),
        detail_json: r#"{"ok":false,"error_code":"path_not_found","path":"A.swift"}"#.to_string(),
        exit_code: None,
    };
    let second = crate::tool_execution::ToolExecutionResult {
        tool_call_id: "b".to_string(),
        tool_id: "file.read".to_string(),
        ok: false,
        preview: "missing".to_string(),
        detail_json: r#"{"ok":false,"error_code":"path_not_found","path":"B.swift"}"#.to_string(),
        exit_code: None,
    };
    assert_ne!(
        model_readable_error_signature(&first),
        model_readable_error_signature(&second)
    );
    assert_eq!(
        model_readable_error_signature(&first).as_deref(),
        Some("path_not_found:A.swift")
    );
}
