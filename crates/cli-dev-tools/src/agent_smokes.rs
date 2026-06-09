#![allow(unused_imports, dead_code)]

use crate::core_smokes::*;
use crate::deepseek::*;
use crate::fixtures::*;
use crate::helpers::*;
use crate::live_model::*;
use crate::prelude::*;
use crate::qwen_tools::*;
use crate::runtime_smokes::*;
pub(crate) fn session_memory_continuation_smoke() -> Result<(), String> {
    let root = temp_smoke_root("researchcode-session-memory-continuation")?;
    fs::write(root.join("README.md"), "session memory\n").map_err(|error| error.to_string())?;
    let facade = RuntimeFacade::new(&root, root.join(".researchcode"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::FastAuto,
    )?;
    facade.submit_user_message(&handle.session_id, "remember claw-code-main nested root")?;
    facade.execute_session_tool(
        &handle.session_id,
        "read_1",
        "file.read",
        ToolExecutionArgs {
            path: Some("README.md".to_string()),
            ..ToolExecutionArgs::default()
        },
    )?;
    let bundle = facade.build_context_bundle(&handle.session_id)?;
    let _ = fs::remove_dir_all(&root);
    if !bundle
        .items
        .iter()
        .any(|item| item.content.contains("remember claw-code-main"))
        || !bundle
            .items
            .iter()
            .any(|item| item.content.contains("read file README.md"))
    {
        return Err("session memory did not continue into context bundle".to_string());
    }
    println!("session memory continuation smoke passed");
    Ok(())
}

pub(crate) fn deepseek_multi_tool_continuation_smoke() -> Result<(), String> {
    let result = run_scripted_native_agent_loop_v2_continuation_fixture()?;
    if result.status != NativeAgentLoopStatus::Completed
        || result.tool_call_count < 3
        || !result.event_jsonl.contains("\"tool_id\":\"file.read\"")
        || !result
            .event_jsonl
            .contains("\"tool_id\":\"search.ripgrep\"")
        || !result.event_jsonl.contains("\"tool_id\":\"git.status\"")
        || result.prompt_tokens == 0
        || result.completion_tokens == 0
        || result.prompt_cache_hit_tokens == 0
    {
        return Err("DeepSeek multi-tool continuation did not complete expected chain".to_string());
    }
    println!(
        "deepseek multi tool continuation smoke passed tools={} models={} tokens={}/{} cache={}/{}",
        result.tool_call_count,
        result.model_call_count,
        result.prompt_tokens,
        result.completion_tokens,
        result.prompt_cache_hit_tokens,
        result.prompt_cache_miss_tokens
    );
    Ok(())
}

pub(crate) fn native_loop_v2_repeated_tool_recovery_smoke() -> Result<(), String> {
    let result = run_scripted_native_agent_loop_v2_repeated_tool_recovery_fixture()?;
    if result.status != NativeAgentLoopStatus::Completed
        || !result.event_jsonl.contains("\"agent.loop_recovery\"")
        || !result.event_jsonl.contains("\"tool.auto_recovery\"")
        || !(result.event_jsonl.contains("\"auto_list_directory\"")
            || result
                .event_jsonl
                .contains("\"source\":\"streaming_tool_call\""))
        || !result.event_jsonl.contains("repeated_tool_batch")
        || !result.event_jsonl.contains("Recovered:")
        || result.event_jsonl.contains("\"agent.loop_incomplete\"")
    {
        return Err("native loop v2 repeated tool recovery did not complete".to_string());
    }
    println!(
        "native loop v2 repeated tool recovery smoke passed tools={} models={} status={:?}",
        result.tool_call_count, result.model_call_count, result.status
    );
    Ok(())
}

pub(crate) fn native_loop_v2_tool_error_continuation_smoke() -> Result<(), String> {
    let result = run_scripted_native_agent_loop_v2_tool_error_continuation_fixture()?;
    if result.status != NativeAgentLoopStatus::Completed
        || result.final_state != AgentState::Completed
        || !result.event_jsonl.contains("\"tool_id\":\"file.write\"")
        || !(result.event_jsonl.contains("tool_execution_error")
            || result.event_jsonl.contains("SCHEMA_VALIDATION_FAILED"))
        || !result
            .event_jsonl
            .contains("final answer instead of stopping")
        || result.event_jsonl.contains("\"agent.loop_incomplete\"")
    {
        return Err(
            "native loop v2 tool execution error did not continue through tool_result".to_string(),
        );
    }
    println!(
        "native loop v2 tool error continuation smoke passed tools={} models={} status={:?}",
        result.tool_call_count, result.model_call_count, result.status
    );
    Ok(())
}

pub(crate) fn native_loop_v2_fastauto_write_smoke() -> Result<(), String> {
    let (result, written) = run_scripted_native_agent_loop_v2_fastauto_write_fixture()?;
    if result.status != NativeAgentLoopStatus::Completed
        || result.final_state != AgentState::Completed
        || result.tool_call_count != 1
        || !written.contains("<h1>ResearchCode</h1>")
        || !result.event_jsonl.contains("\"tool_id\":\"file.write\"")
        || !result.event_jsonl.contains("file.write wrote")
        || result.event_jsonl.contains("MissingArgument(\"content\")")
    {
        return Err("native loop v2 FastAuto write did not create expected file".to_string());
    }
    println!(
        "native loop v2 FastAuto write smoke passed tools={} models={} status={:?}",
        result.tool_call_count, result.model_call_count, result.status
    );
    Ok(())
}

pub(crate) fn qwen_native_loop_v2_fastauto_write_smoke() -> Result<(), String> {
    let (result, written) = run_scripted_qwen_native_agent_loop_v2_fastauto_write_fixture()?;
    if result.status != NativeAgentLoopStatus::Completed
        || result.final_state != AgentState::Completed
        || result.tool_call_count != 1
        || !written.contains("<h1>Qwen Native</h1>")
        || !result.event_jsonl.contains("\"provider\":\"qwen\"")
        || !result.event_jsonl.contains("\"tool_id\":\"file.write\"")
        || !result.event_jsonl.contains("file.write wrote")
        || result.event_jsonl.contains("MissingArgument(\"content\")")
    {
        return Err("Qwen native loop v2 FastAuto write did not create expected file".to_string());
    }
    println!(
        "Qwen native loop v2 FastAuto write smoke passed tools={} models={} status={:?}",
        result.tool_call_count, result.model_call_count, result.status
    );
    Ok(())
}

pub(crate) fn native_loop_v2_max_iteration_structured_stop_smoke() -> Result<(), String> {
    let result = run_scripted_native_agent_loop_v2_max_iteration_structured_stop_fixture()?;
    if result.status != NativeAgentLoopStatus::Blocked
        || result.final_state != AgentState::Failed
        || !result.event_jsonl.contains("\"agent.loop_budget_reached\"")
        || !result.event_jsonl.contains("\"reason\":\"max_iterations\"")
        || !result
            .event_jsonl
            .contains("\"event_type\":\"agent.loop_stopped\"")
        || !result.event_jsonl.contains("\"category\":\"turn_budget\"")
        || result.event_jsonl.contains("visible_finalizer")
        || result.event_jsonl.contains("agent.final_answer")
        || result.event_jsonl.contains("\"agent.loop_incomplete\"")
    {
        return Err("native loop v2 max-iteration budget did not stop structurally".to_string());
    }
    println!(
        "native loop v2 max-iteration structured stop smoke passed tools={} models={} status={:?}",
        result.tool_call_count, result.model_call_count, result.status
    );
    Ok(())
}

pub(crate) fn deepseek_natural_visible_answer_smoke() -> Result<(), String> {
    let result = run_scripted_native_agent_loop_v2_continuation_fixture()?;
    if result.status != NativeAgentLoopStatus::Completed
        || result.final_state != AgentState::Completed
        || !result
            .event_jsonl
            .contains("\"event_type\":\"assistant.message\"")
        || result.event_jsonl.contains("visible_finalizer")
        || result.event_jsonl.contains("agent.final_answer")
        || result
            .event_jsonl
            .contains("\"event_type\":\"agent.loop_stopped\"")
        || result.event_jsonl.contains("\"agent.loop_incomplete\"")
        || result.event_jsonl.contains("<｜｜DSML｜｜tool_calls>")
    {
        return Err("DeepSeek natural loop did not produce a clean visible answer".to_string());
    }
    println!(
        "deepseek natural visible answer smoke passed tools={} models={} status={:?}",
        result.tool_call_count, result.model_call_count, result.status
    );
    Ok(())
}

pub(crate) fn deepseek_reasoning_replay_smoke() -> Result<(), String> {
    let native = decide_reasoning_replay(
        "Need tool continuation with sk-secret in .env",
        ReasoningReplayMode::NativeField,
        ReasoningReplayTarget::DeepSeekNativeRequest,
    );
    let ReasoningReplayDecision::AllowNativeReplay { sanitized } = native else {
        return Err("DeepSeek native reasoning replay was not allowed".to_string());
    };
    if sanitized.contains("sk-secret") || sanitized.contains(".env") {
        return Err(format!("reasoning replay was not sanitized: {sanitized}"));
    }
    let generic = decide_reasoning_replay(
        "Do not replay this as user text",
        ReasoningReplayMode::NativeField,
        ReasoningReplayTarget::GenericChatMessage,
    );
    if generic != ReasoningReplayDecision::BlockIncompatibleReplay {
        return Err("generic chat reasoning replay was not blocked".to_string());
    }
    let artifact = decide_reasoning_replay(
        "Persist AKIA123456 in id_rsa",
        ReasoningReplayMode::SummarizedOnly,
        ReasoningReplayTarget::Artifact,
    );
    let ReasoningReplayDecision::PersistSanitizedSummary {
        sanitized: artifact_summary,
    } = artifact
    else {
        return Err(
            "artifact reasoning summary was not persisted as sanitized summary".to_string(),
        );
    };
    if artifact_summary.contains("AKIA123456") || artifact_summary.contains("id_rsa") {
        return Err(format!(
            "artifact reasoning summary was not sanitized: {artifact_summary}"
        ));
    }
    println!("deepseek reasoning replay smoke passed");
    Ok(())
}

pub(crate) fn native_loop_v2_plan_enter_smoke() -> Result<(), String> {
    let result = run_scripted_native_agent_loop_v2_plan_enter_fixture()?;
    if result.status != NativeAgentLoopStatus::Blocked
        || result.final_state != AgentState::WaitingForPlanApproval
        || !result.event_jsonl.contains("\"plan.mode_entered\"")
        || !result.event_jsonl.contains("\"plan.approval_requested\"")
        || result.event_jsonl.contains("\"permission.requested\"")
    {
        return Err("native loop v2 plan.enter did not route to PlanApproval".to_string());
    }
    println!(
        "native loop v2 plan.enter smoke passed models={} tools={} state={:?}",
        result.model_call_count, result.tool_call_count, result.final_state
    );
    Ok(())
}

pub(crate) fn native_loop_v2_ask_user_smoke() -> Result<(), String> {
    let result = run_scripted_native_agent_loop_v2_ask_user_fixture()?;
    if result.status != NativeAgentLoopStatus::Blocked
        || result.final_state != AgentState::WaitingForUser
        || !result.event_jsonl.contains("\"user.question_requested\"")
        || result.event_jsonl.contains("\"permission.requested\"")
    {
        return Err("native loop v2 ask_user did not route to WaitingForUser".to_string());
    }
    println!(
        "native loop v2 ask_user smoke passed models={} tools={} state={:?}",
        result.model_call_count, result.tool_call_count, result.final_state
    );
    Ok(())
}

pub(crate) fn qwen_tool_continuation_fixture_smoke() -> Result<(), String> {
    qwen_tool_result_continuation_smoke()?;
    let raw = r#"{"reasoning":"Need exact file.","tool_calls":[{"name":"file.read","arguments":{"path":"README.md"}}]}"#;
    let parsed = classify_qwen_output(raw);
    if parsed.action != ParserAction::Execute || parsed.tool_id.as_deref() != Some("file.read") {
        return Err("Qwen native parser did not preserve tool execution path".to_string());
    }
    println!("qwen tool continuation fixture smoke passed");
    Ok(())
}

pub(crate) fn planmode_smoke() -> Result<(), String> {
    let root = temp_smoke_root("researchcode-planmode")?;
    let facade = RuntimeFacade::new(&root, root.join(".researchcode"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::FastAuto,
    )?;
    let outcome = facade.execute_session_tool(
        &handle.session_id,
        "plan_enter_1",
        "plan.enter",
        ToolExecutionArgs {
            content: Some("Plan: inspect first.".to_string()),
            ..ToolExecutionArgs::default()
        },
    )?;
    if !matches!(outcome, FacadeToolOutcome::RequiresPlanApproval { .. }) {
        let _ = fs::remove_dir_all(&root);
        return Err("plan.enter did not request plan approval".to_string());
    }
    let snapshot = facade.get_session_snapshot(&handle.session_id)?;
    let _ = fs::remove_dir_all(&root);
    if !snapshot.plan_mode_active || snapshot.pending_plan_approval_count != 1 {
        return Err("PlanMode snapshot did not expose active pending approval".to_string());
    }
    println!("planmode smoke passed");
    Ok(())
}

pub(crate) fn planmode_denies_write_smoke() -> Result<(), String> {
    let root = temp_smoke_root("researchcode-planmode-deny")?;
    fs::write(root.join("README.md"), "before\n").map_err(|error| error.to_string())?;
    let facade = RuntimeFacade::new(&root, root.join(".researchcode"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::FastAuto,
    )?;
    facade.execute_session_tool(
        &handle.session_id,
        "plan_enter_1",
        "plan.enter",
        ToolExecutionArgs::default(),
    )?;
    let outcome = facade.execute_session_tool(
        &handle.session_id,
        "write_1",
        "file.write",
        ToolExecutionArgs {
            path: Some("README.md".to_string()),
            content: Some("after\n".to_string()),
            ..ToolExecutionArgs::default()
        },
    )?;
    let contents = fs::read_to_string(root.join("README.md")).map_err(|error| error.to_string())?;
    let _ = fs::remove_dir_all(&root);
    if !matches!(outcome, FacadeToolOutcome::BlockedByPolicy(_)) || contents != "before\n" {
        return Err("PlanMode failed to block file.write".to_string());
    }
    println!("planmode denies write smoke passed");
    Ok(())
}

pub(crate) fn subagent_smoke() -> Result<(), String> {
    let root = temp_smoke_root("researchcode-subagent")?;
    let facade = RuntimeFacade::new(&root, root.join(".researchcode"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::FastAuto,
    )?;
    let request = SubagentRequest::readonly(
        &handle.session_id,
        SubagentType::Explorer,
        "scan docs",
        NativeModelFamily::DeepSeek,
    );
    let subagent = facade.spawn_subagent(&handle.session_id, request)?;
    let summary = facade.run_subagent_task(&subagent.subagent_id, "inspect README only")?;
    let parent_events = facade.stream_agent_events(&handle.session_id)?.jsonl;
    let child_events = facade.stream_subagent_events(&subagent.subagent_id)?.jsonl;
    let _ = fs::remove_dir_all(&root);
    if !parent_events.contains("subagent.spawned")
        || parent_events.contains("subagent.tool_completed")
        || !parent_events.contains("subagent.summary_recorded")
        || !child_events.contains("subagent.child_created")
        || !child_events.contains("subagent.tool_completed")
        || !child_events.contains("subagent.completed")
        || !summary.evidence_refs[0].contains("subagent:")
    {
        return Err("subagent lifecycle did not keep child events isolated".to_string());
    }
    println!("subagent smoke passed");
    Ok(())
}

pub(crate) fn task_dispatch_llm_smoke() -> Result<(), String> {
    let root = temp_smoke_root("researchcode-task-dispatch-llm")?;
    fs::write(root.join("README.md"), "task dispatch cli fixture\n")
        .map_err(|error| error.to_string())?;
    let facade = RuntimeFacade::new(&root, root.join(".researchcode"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::FastAuto,
    )?;
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Task dispatch child complete."}}
data: [DONE]"#
            .to_string(),
    }]);
    let summary = facade.run_task_dispatch_with_transport(
        &transport,
        &handle.session_id,
        "task_dispatch_llm_smoke",
        ToolExecutionArgs {
            content: Some("inspect README.md".to_string()),
            model_role: Some("reviewer".to_string()),
            ..ToolExecutionArgs::default()
        },
        NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
    )?;
    let parent_events = facade.stream_agent_events(&handle.session_id)?.jsonl;
    let _ = fs::remove_dir_all(&root);
    if summary.status != SubagentStatus::Completed
        || !parent_events.contains("tool.call_requested")
        || !parent_events.contains("task_dispatch_llm_smoke")
        || !parent_events.contains("subagent.summary_recorded")
        || !parent_events.contains("tool.call_completed")
        || parent_events.contains("subagent.model_turn_started")
    {
        return Err("task.dispatch LLM smoke did not keep child model events isolated".to_string());
    }
    println!("task dispatch llm smoke passed");
    Ok(())
}

pub(crate) fn task_dispatch_worker_smoke() -> Result<(), String> {
    let root = temp_smoke_root("researchcode-task-dispatch-worker")?;
    fs::create_dir_all(root.join("src")).map_err(|error| error.to_string())?;
    let facade = RuntimeFacade::new(&root, root.join(".researchcode"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::FastAuto,
    )?;
    let transport = ScriptedLiveHttpTransport::new(vec![
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_write","name":"file_write","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"src/generated.txt\",\"content\":\"worker smoke ok\\n\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                .to_string(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Worker smoke complete."}}
data: [DONE]"#
                .to_string(),
        },
    ]);
    let summary = facade.run_task_dispatch_with_transport(
        &transport,
        &handle.session_id,
        "task_dispatch_worker_smoke",
        ToolExecutionArgs {
            content: Some("create src/generated.txt".to_string()),
            model_role: Some("executor".to_string()),
            write_scope_json: Some(r#"{"paths":["src"]}"#.to_string()),
            ..ToolExecutionArgs::default()
        },
        NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
    )?;
    let parent_events = facade.stream_agent_events(&handle.session_id)?.jsonl;
    let written =
        fs::read_to_string(root.join("src/generated.txt")).map_err(|error| error.to_string())?;
    let _ = fs::remove_dir_all(&root);
    if summary.status != SubagentStatus::Completed
        || written != "worker smoke ok\n"
        || !parent_events.contains("task_dispatch_worker_smoke")
        || !parent_events.contains("subagent.summary_recorded")
        || parent_events.contains("subagent.model_turn_started")
    {
        return Err("task.dispatch worker smoke did not complete scoped child edit".to_string());
    }
    println!("task dispatch worker smoke passed");
    Ok(())
}

pub(crate) fn agentteam_smoke() -> Result<(), String> {
    let (team, ledger, plan) =
        researchcode_runtime::ultra::build_ultraplan_fixture("agentteam smoke");
    plan.validate(&ledger)?;
    if team.allow_full_mesh || team.max_agents != 5 || ledger.notes.is_empty() {
        return Err("agentteam fixture violated v1 policy".to_string());
    }
    println!("agentteam smoke passed team={}", team.team_id);
    Ok(())
}

pub(crate) fn agentteam_messagebus_smoke() -> Result<(), String> {
    let mut ledger = researchcode_runtime::agent_team::EvidenceLedger::default();
    let evidence = ledger.add_note("agent_a", "README.md", "fact");
    let ok = researchcode_runtime::agent_team::AgentTeamMessage {
        message_id: "m_ok".to_string(),
        team_id: "team".to_string(),
        from_agent_id: "agent_a".to_string(),
        to_agent_id: None,
        kind: researchcode_runtime::agent_team::AgentTeamMessageKind::EvidenceNote,
        content: "fact".to_string(),
        evidence_refs: vec![evidence.clone()],
    };
    researchcode_runtime::agent_team::validate_team_message(&ok, &ledger)?;
    let bad = researchcode_runtime::agent_team::AgentTeamMessage {
        to_agent_id: Some("agent_b".to_string()),
        ..ok
    };
    if researchcode_runtime::agent_team::validate_team_message(&bad, &ledger).is_ok() {
        return Err("message bus allowed full-mesh direct message".to_string());
    }
    println!("agentteam messagebus smoke passed");
    Ok(())
}

pub(crate) fn evidence_ledger_smoke() -> Result<(), String> {
    let mut ledger = researchcode_runtime::agent_team::EvidenceLedger::default();
    let evidence = ledger.add_note("scout", "AGENTS.md", "RuntimeFacade boundary");
    researchcode_runtime::agent_team::validate_final_claims(&[evidence], &ledger)?;
    if researchcode_runtime::agent_team::validate_final_claims(&[], &ledger).is_ok() {
        return Err("EvidenceLedger allowed final claim without evidence".to_string());
    }
    println!("evidence ledger smoke passed");
    Ok(())
}

pub(crate) fn ultraplan_fixture_smoke() -> Result<(), String> {
    let root = temp_smoke_root("researchcode-ultraplan")?;
    let facade = RuntimeFacade::new(&root, root.join(".researchcode"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::FastAuto,
    )?;
    let plan = facade.run_ultraplan_fixture(&handle.session_id, "build tool loop v3")?;
    let snapshot = facade.get_session_snapshot(&handle.session_id)?;
    let _ = fs::remove_dir_all(&root);
    if plan.evidence_refs.is_empty() || snapshot.pending_plan_approval_count != 1 {
        return Err("UltraPlan fixture did not create evidence-backed plan approval".to_string());
    }
    println!("ultraplan fixture smoke passed plan={}", plan.plan_id);
    Ok(())
}

pub(crate) fn ultrareview_fixture_smoke() -> Result<(), String> {
    let root = temp_smoke_root("researchcode-ultrareview")?;
    let facade = RuntimeFacade::new(&root, root.join(".researchcode"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::FastAuto,
    )?;
    let report = facade.run_ultrareview_fixture(&handle.session_id, "README.md")?;
    let events = facade.stream_agent_events(&handle.session_id)?.jsonl;
    let _ = fs::remove_dir_all(&root);
    if report.verified_findings.is_empty() || !events.contains("ultrareview.completed") {
        return Err("UltraReview fixture did not record verified finding".to_string());
    }
    println!(
        "ultrareview fixture smoke passed report={}",
        report.report_id
    );
    Ok(())
}

pub(crate) fn temp_smoke_root(prefix: &str) -> Result<PathBuf, String> {
    let root = std::env::temp_dir().join(format!(
        "{}-{}",
        prefix,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    Ok(root)
}
