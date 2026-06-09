use super::*;

pub fn run_scripted_native_agent_loop_fixture(
) -> Result<ScriptedNativeAgentLoopFixtureResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("researchcode-native-agent-loop-v2-compat-{nonce}"));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let artifact_root = root.join("artifacts");
    let html = "<!doctype html><html><body><h1>Native V2 Compat</h1></body></html>";
    let escaped_html = html.replace('\\', "\\\\").replace('"', "\\\"");
    let transport = ScriptedLiveHttpTransport::new(vec![
        LiveHttpResponse {
            status_code: 200,
            body: format!(
                "data: {{\"model\":\"Qwen/Qwen3.6-27B\",\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"function\":{{\"name\":\"file_write\",\"arguments\":\"{{\\\"path\\\":\\\"native-v2-compat.html\\\",\\\"content\\\":\\\"{escaped_html}\\\"}}\"}}}}]}}}}]}}\n\
data: {{\"usage\":{{\"prompt_tokens\":64,\"completion_tokens\":16,\"total_tokens\":80}}}}\n\
data: [DONE]"
            ),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"content":"Created native-v2-compat.html."}}]}
data: {"usage":{"prompt_tokens":72,"completion_tokens":20,"total_tokens":92}}
data: [DONE]"#
                .to_string(),
        },
    ]);
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: "sess_native_loop_v2_compat".to_string(),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: root.clone(),
        artifact_root,
        endpoint: live_qwen_endpoint(),
        prompt: "Create native-v2-compat.html.".to_string(),
        max_tokens: 4096,
        max_iterations: 4,
        max_tool_calls: 8,
        tool_exposure: NativeAgentToolExposure::FastAutoWrite,
        permission_mode: PermissionMode::Default,
        provided_permission_decisions: vec![NativeAgentPermissionDecision {
            permission_id: "native_loop_v2_stream_perm_0".to_string(),
            decision: PermissionDecisionKind::AllowOnce,
        }],
        deepseek_adaptation: None,
        error_recovery: None,
        hook_dispatcher: None,
        concurrent_tool_execution: false,
    };
    let loop_result = crate::agent_kernel::AgentKernel::for_request(&request)
        .run_turn(&transport, request, None)?;
    let written = fs::read_to_string(root.join("native-v2-compat.html")).unwrap_or_default();
    let _ = fs::remove_dir_all(&root);
    Ok(ScriptedNativeAgentLoopFixtureResult {
        loop_result,
        final_file_hash: stable_text_hash(&written),
    })
}

pub fn run_scripted_native_agent_loop_v2_continuation_fixture(
) -> Result<NativeAgentLoopResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-native-agent-loop-v2-{nonce}"));
    fs::create_dir_all(root.join("src")).map_err(|error| error.to_string())?;
    fs::write(
        root.join("src/lib.rs"),
        "pub const NEEDLE: &str = \"needle\";\n",
    )
    .map_err(|error| error.to_string())?;
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"file.read","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"src/lib.rs\"}"}}
data: {"type":"content_block_stop","index":0}
data: {"usage":{"input_tokens":20,"output_tokens":5,"cache_read_input_tokens":4,"cache_creation_input_tokens":1}}
data: [DONE]"#
                .to_string(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_2","name":"search.ripgrep","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"pattern\":\"NEEDLE\",\"root\":\"src\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                .to_string(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_3","name":"git.status","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"root\":\".\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                .to_string(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Read, searched, checked git, and finished."}}
data: [DONE]"#
                .to_string(),
        },
    ]);
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: "sess_native_loop_v2".to_string(),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: root.clone(),
        artifact_root,
        endpoint: live_deepseek_endpoint(),
        prompt: "Inspect src/lib.rs and summarize status.".to_string(),
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
    };
    let result = crate::agent_kernel::AgentKernel::for_request(&request)
        .run_turn(&transport, request, None)?;
    let _ = fs::remove_dir_all(&root);
    Ok(result)
}

pub fn run_scripted_native_agent_loop_v2_repeated_tool_recovery_fixture(
) -> Result<NativeAgentLoopResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-recovery-{nonce}"
    ));
    fs::create_dir_all(root.join("src")).map_err(|error| error.to_string())?;
    fs::write(root.join("src/lib.rs"), "pub fn answer() -> u8 { 42 }\n")
        .map_err(|error| error.to_string())?;
    let artifact_root = root.join("artifacts");
    let repeated_directory_read = r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_repeat","name":"file.read","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\".\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#;
    let transport = ScriptedLiveHttpTransport::new(vec![
        LiveHttpResponse {
            status_code: 200,
            body: repeated_directory_read.to_string(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: repeated_directory_read.to_string(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Recovered: the repeated directory read was corrected and I can continue with repo mapping or a concrete file."}}
data: [DONE]"#
                .to_string(),
        },
    ]);
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: "sess_native_loop_v2_recovery".to_string(),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: root.clone(),
        artifact_root,
        endpoint: live_deepseek_endpoint(),
        prompt: "Inspect this workspace without getting stuck on directory reads.".to_string(),
        max_tokens: 1024,
        max_iterations: 6,
        max_tool_calls: 8,
        tool_exposure: NativeAgentToolExposure::ReadOnly,
        permission_mode: PermissionMode::Default,
        provided_permission_decisions: Vec::new(),
        deepseek_adaptation: None,
        error_recovery: None,
        hook_dispatcher: None,
        concurrent_tool_execution: false,
    };
    let result = crate::agent_kernel::AgentKernel::for_request(&request)
        .run_turn(&transport, request, None)?;
    let _ = fs::remove_dir_all(&root);
    Ok(result)
}

pub fn run_scripted_native_agent_loop_v2_tool_error_continuation_fixture(
) -> Result<NativeAgentLoopResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-tool-error-{nonce}"
    ));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_write","name":"file_write","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"demo.html\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                .to_string(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"The write tool was not executed because this turn is read-only; I will provide the complete HTML content in the final answer instead of stopping."}}
data: [DONE]"#
                .to_string(),
        },
    ]);
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: "sess_native_loop_v2_tool_error".to_string(),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: root.clone(),
        artifact_root,
        endpoint: live_deepseek_endpoint(),
        prompt: "Create a small HTML demo without aborting on tool errors.".to_string(),
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
    };
    let result = crate::agent_kernel::AgentKernel::for_request(&request)
        .run_turn(&transport, request, None)?;
    let _ = fs::remove_dir_all(&root);
    Ok(result)
}

pub fn run_scripted_native_agent_loop_v2_fastauto_write_fixture(
) -> Result<(NativeAgentLoopResult, String), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-fastauto-write-{nonce}"
    ));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let artifact_root = root.join("artifacts");
    let html = "<!doctype html><html><body><h1>ResearchCode</h1><p>created</p></body></html>";
    let escaped_html = html.replace('\\', "\\\\").replace('"', "\\\"");
    let transport = ScriptedLiveHttpTransport::new(vec![
        LiveHttpResponse {
            status_code: 200,
            body: format!(
                "data: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"tool_use\",\"id\":\"toolu_write\",\"name\":\"file_write\",\"input\":{{}}}}}}\n\
data: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":\"{{\\\"path\\\":\\\"taskboard.html\\\",\\\"content\\\":\\\"{escaped_html}\\\"}}\"}}}}\n\
data: {{\"type\":\"content_block_stop\",\"index\":0}}\n\
data: [DONE]"
            ),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Created taskboard.html with the requested HTML."}}
data: [DONE]"#
                .to_string(),
        },
    ]);
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: "sess_native_loop_v2_fastauto_write".to_string(),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: root.clone(),
        artifact_root,
        endpoint: live_deepseek_endpoint(),
        prompt: "Create an html page taskboard.html.".to_string(),
        max_tokens: 4096,
        max_iterations: 4,
        max_tool_calls: 8,
        tool_exposure: NativeAgentToolExposure::FastAutoWrite,
        permission_mode: PermissionMode::Default,
        provided_permission_decisions: Vec::new(),
        deepseek_adaptation: None,
        error_recovery: None,
        hook_dispatcher: None,
        concurrent_tool_execution: false,
    };
    let result = crate::agent_kernel::AgentKernel::for_request(&request)
        .run_turn(&transport, request, None)?;
    let written = fs::read_to_string(root.join("taskboard.html")).unwrap_or_default();
    let _ = fs::remove_dir_all(&root);
    Ok((result, written))
}

pub fn run_scripted_qwen_native_agent_loop_v2_fastauto_write_fixture(
) -> Result<(NativeAgentLoopResult, String), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-qwen-native-agent-loop-v2-fastauto-write-{nonce}"
    ));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let artifact_root = root.join("artifacts");
    let html = "<!doctype html><html><body><h1>Qwen Native</h1><p>created</p></body></html>";
    let escaped_html = html.replace('\\', "\\\\").replace('"', "\\\"");
    let transport = ScriptedLiveHttpTransport::new(vec![
        LiveHttpResponse {
            status_code: 200,
            body: format!(
                "data: {{\"model\":\"Qwen/Qwen3.6-27B\",\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"function\":{{\"name\":\"file_write\",\"arguments\":\"{{\\\"path\\\":\\\"qwen-taskboard.html\\\",\\\"content\\\":\\\"{escaped_html}\\\"}}\"}}}}]}}}}]}}\n\
data: {{\"usage\":{{\"prompt_tokens\":64,\"completion_tokens\":16,\"total_tokens\":80}}}}\n\
data: [DONE]"
            ),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"content":"Created qwen-taskboard.html with the requested HTML."}}]}
data: {"usage":{"prompt_tokens":72,"completion_tokens":20,"total_tokens":92}}
data: [DONE]"#
                .to_string(),
        },
    ]);
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: "sess_qwen_native_loop_v2_fastauto_write".to_string(),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: root.clone(),
        artifact_root,
        endpoint: live_qwen_endpoint(),
        prompt: "Create an html page qwen-taskboard.html.".to_string(),
        max_tokens: 4096,
        max_iterations: 4,
        max_tool_calls: 8,
        tool_exposure: NativeAgentToolExposure::FastAutoWrite,
        permission_mode: PermissionMode::Default,
        provided_permission_decisions: Vec::new(),
        deepseek_adaptation: None,
        error_recovery: None,
        hook_dispatcher: None,
        concurrent_tool_execution: false,
    };
    let result = crate::agent_kernel::AgentKernel::for_request(&request)
        .run_turn(&transport, request, None)?;
    let written = fs::read_to_string(root.join("qwen-taskboard.html")).unwrap_or_default();
    let _ = fs::remove_dir_all(&root);
    Ok((result, written))
}

pub fn run_scripted_native_agent_loop_v2_max_iteration_structured_stop_fixture(
) -> Result<NativeAgentLoopResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-max-structured-stop-{nonce}"
    ));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    fs::write(
        root.join("README.md"),
        "ResearchCode max iteration fixture\n",
    )
    .map_err(|error| error.to_string())?;
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_read","name":"file_read","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"README.md\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                .to_string(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Max iteration structured stop: README.md was read, so I can summarize without requesting more tools."}}
data: [DONE]"#
                .to_string(),
        },
    ]);
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: "sess_native_loop_v2_max_structured_stop".to_string(),
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
    };
    let result = crate::agent_kernel::AgentKernel::for_request(&request)
        .run_turn(&transport, request, None)?;
    let _ = fs::remove_dir_all(&root);
    Ok(result)
}

pub fn run_scripted_native_agent_loop_v2_plan_enter_fixture(
) -> Result<NativeAgentLoopResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-plan-enter-{nonce}"
    ));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_plan","name":"plan_enter","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"plan\":\"1. inspect context\\n2. propose implementation\\n3. wait for approval\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
            .to_string(),
    }]);
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: "sess_native_loop_v2_plan_enter".to_string(),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: root.clone(),
        artifact_root,
        endpoint: live_deepseek_endpoint(),
        prompt: "Enter plan mode for this implementation.".to_string(),
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
    };
    let result = crate::agent_kernel::AgentKernel::for_request(&request)
        .run_turn(&transport, request, None)?;
    let _ = fs::remove_dir_all(&root);
    Ok(result)
}

pub fn run_scripted_native_agent_loop_v2_ask_user_fixture() -> Result<NativeAgentLoopResult, String>
{
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-v2-ask-user-{nonce}"
    ));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let artifact_root = root.join("artifacts");
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_ask","name":"ask_user","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"question\":\"Which file should I inspect first?\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
            .to_string(),
    }]);
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: "sess_native_loop_v2_ask_user".to_string(),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: root.clone(),
        artifact_root,
        endpoint: live_deepseek_endpoint(),
        prompt: "Ask for clarification only if the target file is ambiguous.".to_string(),
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
    };
    let result = crate::agent_kernel::AgentKernel::for_request(&request)
        .run_turn(&transport, request, None)?;
    let _ = fs::remove_dir_all(&root);
    Ok(result)
}

pub fn run_scripted_native_agent_loop_external_block_fixture(
) -> Result<NativeAgentLoopResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("researchcode-native-agent-loop-external-{nonce}"));
    fs::create_dir_all(root.join("src")).map_err(|error| error.to_string())?;
    fs::write(
        root.join("src/parser.ts"),
        "export const retry_count = 3;\n",
    )
    .map_err(|error| error.to_string())?;
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: qwen_file_write_parser_sse_body(),
    }]);
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: "sess_native_loop_external".to_string(),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: root.clone(),
        artifact_root: root.join("artifacts"),
        endpoint: live_qwen_endpoint(),
        prompt: "patch file".to_string(),
        max_tokens: 1024,
        max_iterations: 2,
        max_tool_calls: 4,
        tool_exposure: NativeAgentToolExposure::CodeEdit,
        permission_mode: PermissionMode::Default,
        provided_permission_decisions: Vec::new(),
        deepseek_adaptation: None,
        error_recovery: None,
        hook_dispatcher: None,
        concurrent_tool_execution: false,
    };
    let result = crate::agent_kernel::AgentKernel::for_request(&request)
        .run_turn(&transport, request, None)?;
    let _ = fs::remove_dir_all(&root);
    Ok(result)
}

pub fn run_scripted_native_agent_loop_provided_permission_fixture(
) -> Result<ScriptedNativeAgentLoopFixtureResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("researchcode-native-agent-loop-provided-{nonce}"));
    fs::create_dir_all(root.join("src")).map_err(|error| error.to_string())?;
    let target = root.join("src/parser.ts");
    fs::write(&target, "export const retry_count = 3;\n").map_err(|error| error.to_string())?;
    let transport = ScriptedLiveHttpTransport::new(vec![
        LiveHttpResponse {
            status_code: 200,
            body: qwen_file_write_parser_sse_body(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: "data: {\"model\":\"Qwen/Qwen3.6-27B\",\"choices\":[{\"delta\":{\"content\":\"Updated parser retry count.\"}}]}\ndata: [DONE]".to_string(),
        },
    ]);
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: "sess_native_loop_provided".to_string(),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: root.clone(),
        artifact_root: root.join("artifacts"),
        endpoint: live_qwen_endpoint(),
        prompt: "patch file".to_string(),
        max_tokens: 1024,
        max_iterations: 2,
        max_tool_calls: 4,
        tool_exposure: NativeAgentToolExposure::CodeEdit,
        permission_mode: PermissionMode::Default,
        provided_permission_decisions: vec![NativeAgentPermissionDecision {
            permission_id: "native_loop_v2_stream_perm_0".to_string(),
            decision: PermissionDecisionKind::AllowOnce,
        }],
        deepseek_adaptation: None,
        error_recovery: None,
        hook_dispatcher: None,
        concurrent_tool_execution: false,
    };
    let loop_result = crate::agent_kernel::AgentKernel::for_request(&request)
        .run_turn(&transport, request, None)?;
    let final_text = fs::read_to_string(&target).map_err(|error| error.to_string())?;
    let final_file_hash = stable_text_hash(&final_text);
    if final_text != "export const retry_count = 5;\n" {
        return Err(format!(
            "native provided-permission loop final file mismatch: {final_text}"
        ));
    }
    let _ = fs::remove_dir_all(&root);
    Ok(ScriptedNativeAgentLoopFixtureResult {
        loop_result,
        final_file_hash,
    })
}

pub fn run_scripted_native_agent_loop_external_resume_fixture(
) -> Result<ScriptedNativeAgentLoopFixtureResult, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "researchcode-native-agent-loop-real-resume-{nonce}"
    ));
    fs::create_dir_all(root.join("src")).map_err(|error| error.to_string())?;
    let target = root.join("src/parser.ts");
    fs::write(&target, "export const retry_count = 3;\n").map_err(|error| error.to_string())?;
    let block_transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: qwen_file_write_parser_sse_body(),
    }]);
    let request = NativeAgentLoopV2Request {
        project_id: "proj".to_string(),
        session_id: "sess_native_loop_real_resume".to_string(),
        task_id: "task".to_string(),
        turn_id: None,
        workspace_root: root.clone(),
        artifact_root: root.join("artifacts"),
        endpoint: live_qwen_endpoint(),
        prompt: "patch file".to_string(),
        max_tokens: 1024,
        max_iterations: 2,
        max_tool_calls: 4,
        tool_exposure: NativeAgentToolExposure::CodeEdit,
        permission_mode: PermissionMode::Default,
        provided_permission_decisions: Vec::new(),
        deepseek_adaptation: None,
        error_recovery: None,
        hook_dispatcher: None,
        concurrent_tool_execution: false,
    };
    let blocked = crate::agent_kernel::AgentKernel::for_request(&request).run_turn(
        &block_transport,
        request,
        None,
    )?;
    let Some(pending_tool) = blocked.pending_tool.clone() else {
        return Err("external resume fixture did not produce pending tool".to_string());
    };
    let resume_transport = ScriptedLiveHttpTransport::new(vec![]);
    let loop_result = resume_native_agent_loop_after_external_decision(
        &resume_transport,
        NativeAgentLoopResumeRequest {
            previous_event_jsonl: blocked.event_jsonl,
            workspace_root: root.clone(),
            artifact_root: root.join("artifacts"),
            pending_tool,
            decision: PermissionDecisionKind::AllowOnce,
        },
    )?;
    let final_text = fs::read_to_string(&target).map_err(|error| error.to_string())?;
    let final_file_hash = stable_text_hash(&final_text);
    if final_text != "export const retry_count = 5;\n" {
        return Err(format!(
            "native external-resume loop final file mismatch: {final_text}"
        ));
    }
    let _ = fs::remove_dir_all(&root);
    Ok(ScriptedNativeAgentLoopFixtureResult {
        loop_result,
        final_file_hash,
    })
}

pub fn write_scripted_native_agent_loop_external_decision_package(
    package_dir: &Path,
) -> Result<NativeAgentLoopExternalDecisionPackage, String> {
    fs::create_dir_all(package_dir).map_err(|error| error.to_string())?;
    let workspace_root = package_dir.join("workspace");
    let artifact_root = package_dir.join("artifacts");
    fs::create_dir_all(workspace_root.join("src")).map_err(|error| error.to_string())?;
    fs::write(
        workspace_root.join("src/parser.ts"),
        "export const retry_count = 3;\n",
    )
    .map_err(|error| error.to_string())?;
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: qwen_file_write_parser_sse_body(),
    }]);
    let blocked_result = {
        let request = NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_native_loop_package".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: workspace_root.clone(),
            artifact_root: artifact_root.clone(),
            endpoint: live_qwen_endpoint(),
            prompt: "patch file".to_string(),
            max_tokens: 1024,
            max_iterations: 2,
            max_tool_calls: 4,
            tool_exposure: NativeAgentToolExposure::CodeEdit,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        };
        crate::agent_kernel::AgentKernel::for_request(&request)
            .run_turn(&transport, request, None)?
    };
    write_native_agent_loop_external_decision_package(
        package_dir,
        workspace_root,
        artifact_root,
        blocked_result,
        "scripted_external_decision_fixture",
    )
}

fn qwen_file_write_parser_sse_body() -> String {
    let base_hash = stable_text_hash("export const retry_count = 3;\n");
    format!(
        "data: {{\"model\":\"Qwen/Qwen3.6-27B\",\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"function\":{{\"name\":\"file_write\",\"arguments\":\"{{\\\"path\\\":\\\"src/parser.ts\\\",\\\"content\\\":\\\"export const retry_count = 5;\\\\n\\\",\\\"base_hash\\\":\\\"{base_hash}\\\"}}\"}}}}]}}}}]}}\n\
data: {{\"usage\":{{\"prompt_tokens\":16,\"completion_tokens\":8,\"reasoning_tokens\":0}}}}\n\
data: [DONE]"
    )
}

pub fn write_native_agent_loop_external_decision_package(
    package_dir: &Path,
    workspace_root: PathBuf,
    artifact_root: PathBuf,
    blocked_result: NativeAgentLoopResult,
    source: &str,
) -> Result<NativeAgentLoopExternalDecisionPackage, String> {
    fs::create_dir_all(package_dir).map_err(|error| error.to_string())?;
    fs::create_dir_all(&workspace_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&artifact_root).map_err(|error| error.to_string())?;
    if blocked_result.status != NativeAgentLoopStatus::Blocked {
        return Err(format!(
            "pending package requires blocked loop result, got {:?}",
            blocked_result.status
        ));
    }
    let pending_tool = blocked_result
        .pending_tool
        .clone()
        .ok_or_else(|| "blocked loop did not produce pending tool".to_string())?;
    let event_log_path = package_dir.join("events.jsonl");
    let pending_tool_path = package_dir.join("pending_tool.json");
    let manifest_path = package_dir.join("resume_package.json");
    fs::write(&event_log_path, &blocked_result.event_jsonl).map_err(|error| error.to_string())?;
    fs::write(&pending_tool_path, pending_tool_to_json(&pending_tool))
        .map_err(|error| error.to_string())?;
    fs::write(
        &manifest_path,
        format!(
            "{{\"schema_version\":\"researchcode.native_resume_package.v0\",\"workspace_root\":{},\"artifact_root\":{},\"event_log_path\":{},\"pending_tool_path\":{},\"source\":{}}}\n",
            json_string(&workspace_root.to_string_lossy()),
            json_string(&artifact_root.to_string_lossy()),
            json_string(&event_log_path.to_string_lossy()),
            json_string(&pending_tool_path.to_string_lossy()),
            json_string(source)
        ),
    )
    .map_err(|error| error.to_string())?;
    Ok(NativeAgentLoopExternalDecisionPackage {
        package_dir: package_dir.to_path_buf(),
        workspace_root,
        artifact_root,
        event_log_path,
        pending_tool_path,
        manifest_path,
        blocked_result,
    })
}

pub fn resume_scripted_native_agent_loop_external_decision_package(
    package_dir: &Path,
    decision: PermissionDecisionKind,
) -> Result<NativeAgentLoopExternalDecisionPackageResumeResult, String> {
    let manifest_path = package_dir.join("resume_package.json");
    let manifest = fs::read_to_string(&manifest_path).map_err(|error| error.to_string())?;
    let workspace_root = PathBuf::from(
        extract_json_string(&manifest, "workspace_root")
            .ok_or_else(|| "resume package missing workspace_root".to_string())?,
    );
    let artifact_root = PathBuf::from(
        extract_json_string(&manifest, "artifact_root")
            .ok_or_else(|| "resume package missing artifact_root".to_string())?,
    );
    let event_log_path = PathBuf::from(
        extract_json_string(&manifest, "event_log_path")
            .ok_or_else(|| "resume package missing event_log_path".to_string())?,
    );
    let pending_tool_path = PathBuf::from(
        extract_json_string(&manifest, "pending_tool_path")
            .ok_or_else(|| "resume package missing pending_tool_path".to_string())?,
    );
    let previous_event_jsonl =
        fs::read_to_string(&event_log_path).map_err(|error| error.to_string())?;
    let pending_tool_json =
        fs::read_to_string(&pending_tool_path).map_err(|error| error.to_string())?;
    let pending_tool = pending_tool_from_json(&pending_tool_json)?;
    let transport = ScriptedLiveHttpTransport::new(vec![]);
    let loop_result = resume_native_agent_loop_after_external_decision(
        &transport,
        NativeAgentLoopResumeRequest {
            previous_event_jsonl,
            workspace_root: workspace_root.clone(),
            artifact_root,
            pending_tool,
            decision,
        },
    )?;
    let event_log_path = package_dir.join("resumed_events.jsonl");
    fs::write(&event_log_path, &loop_result.event_jsonl).map_err(|error| error.to_string())?;
    let final_text = fs::read_to_string(workspace_root.join("src/parser.ts"))
        .unwrap_or_else(|_| loop_result.event_jsonl.clone());
    let final_file_hash = stable_text_hash(&final_text);
    Ok(NativeAgentLoopExternalDecisionPackageResumeResult {
        loop_result,
        event_log_path,
        final_file_hash,
    })
}
