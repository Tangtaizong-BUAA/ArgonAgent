#![allow(unused_imports, dead_code)]

use crate::agent_smokes::*;
use crate::deepseek::*;
use crate::fixtures::*;
use crate::helpers::*;
use crate::live_model::*;
use crate::prelude::*;
use crate::qwen_tools::*;
use crate::runtime_smokes::*;
pub(crate) fn plan_smoke() -> Result<(), String> {
    let plan = Plan {
        plan_id: "plan_smoke".to_string(),
        task_id: "task_smoke".to_string(),
        summary: "Harden bottom-layer agent tools".to_string(),
        steps: vec![
            PlanStep {
                step_id: "inspect".to_string(),
                title: "Inspect current runtime boundary".to_string(),
                goal: "Gather command, patch, model, and context gaps".to_string(),
                allowed_tools: vec!["file.read".to_string(), "search.ripgrep".to_string()],
                expected_artifacts: vec!["notes".to_string()],
                status: PlanStepStatus::Completed,
            },
            PlanStep {
                step_id: "implement".to_string(),
                title: "Implement focused runtime hardening".to_string(),
                goal: "Patch one bottom-layer capability with tests".to_string(),
                allowed_tools: vec!["patch.apply".to_string()],
                expected_artifacts: vec!["diff".to_string()],
                status: PlanStepStatus::InProgress,
            },
            PlanStep {
                step_id: "verify".to_string(),
                title: "Run verification gates".to_string(),
                goal: "Prove the change through deterministic checks".to_string(),
                allowed_tools: vec!["shell.command".to_string()],
                expected_artifacts: vec!["test output".to_string()],
                status: PlanStepStatus::Pending,
            },
        ],
    };
    plan.validate()
        .map_err(|error| format!("plan invalid: {error:?}"))?;
    let mut builder = ContextBundleBuilder::new("plan_smoke_bundle", "qwen", 4_000);
    if !builder.add_plan(&plan) {
        return Err("plan did not fit context bundle".to_string());
    }
    let bundle = builder.build();
    let progress = plan.progress();
    let next = plan
        .next_actionable_step()
        .map(|step| step.step_id.as_str())
        .unwrap_or("none");
    println!(
        "plan ok=true steps={} completed={} in_progress={} pending={} next={} context_items={} tokens={}",
        progress.total,
        progress.completed,
        progress.in_progress,
        progress.pending,
        next,
        bundle.items.len(),
        bundle.token_estimate()
    );
    Ok(())
}

pub(crate) fn memory_smoke() -> Result<(), String> {
    let memory = MemoryItem {
        memory_id: "mem_qwen_patch_hash".to_string(),
        scope: MemoryScope::ModelFailure,
        source: "eval/qwen/executor".to_string(),
        content: "Qwen executor patches must include base_hash and stay patch-sized.".to_string(),
        privacy_class: "internal".to_string(),
        content_hash: stable_text_hash("qwen-base-hash-memory"),
    };
    memory
        .validate()
        .map_err(|error| format!("memory invalid: {error:?}"))?;
    let secret_memory = MemoryItem {
        memory_id: "mem_secret".to_string(),
        scope: MemoryScope::Project,
        source: "bad".to_string(),
        content: "api_key=sk-secret".to_string(),
        privacy_class: "secret".to_string(),
        content_hash: "fnv64_secret".to_string(),
    };
    if secret_memory.validate().is_ok() {
        return Err("secret-like memory was accepted".to_string());
    }
    let mut builder = ContextBundleBuilder::new("memory_smoke_bundle", "qwen", 4_000);
    if !builder.add_memory(&memory) {
        return Err("memory did not fit context bundle".to_string());
    }
    let bundle = builder.build();
    println!(
        "memory ok=true scope=model_failure context_items={} tokens={} hash={}",
        bundle.items.len(),
        bundle.token_estimate(),
        memory.content_hash
    );
    Ok(())
}

pub(crate) fn run_coding_fixture_smoke() -> Result<(), String> {
    run_coding_fixture(None)
}

pub(crate) fn run_coding_fixture(output_path: Option<PathBuf>) -> Result<(), String> {
    let result = run_no_model_coding_fixture(&NoModelCodingFixtureConfig::default())?;
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&output_path, &result.event_jsonl).map_err(|error| error.to_string())?;
    }
    println!(
        "coding fixture completed state={:?} events={} matches={}",
        result.final_state, result.event_count, result.matches_count
    );
    Ok(())
}

pub(crate) fn run_failure_repair_fixture_smoke() -> Result<(), String> {
    let result = run_failure_repair_fixture(&NoModelCodingFixtureConfig::default())?;
    println!(
        "failure repair fixture completed state={:?} events={} first_exit={} repaired_exit={}",
        result.final_state, result.event_count, result.first_exit_code, result.repaired_exit_code
    );
    Ok(())
}

pub(crate) fn event_replay_smoke() -> Result<(), String> {
    let completed = run_no_model_coding_fixture(&NoModelCodingFixtureConfig::default())?;
    let completed_snapshot =
        replay_jsonl(&completed.event_jsonl).map_err(|error| format!("{error:?}"))?;
    let blocked = run_scripted_native_agent_loop_external_block_fixture()?;
    let blocked_snapshot =
        replay_jsonl(&blocked.event_jsonl).map_err(|error| format!("{error:?}"))?;
    if completed_snapshot.inferred_state != AgentState::Completed {
        return Err(format!(
            "completed fixture replayed to wrong state: {:?}",
            completed_snapshot.inferred_state
        ));
    }
    if blocked_snapshot.inferred_state != AgentState::WaitingForToolApproval
        || blocked_snapshot.pending_permission_ids.len() != 1
    {
        return Err(format!(
            "blocked fixture replay mismatch: {:?}",
            blocked_snapshot
        ));
    }
    println!(
        "event replay completed={} blocked_pending={} completed_tools={}",
        completed_snapshot.sequence,
        blocked_snapshot.pending_permission_ids.len(),
        completed_snapshot.tool_calls_completed
    );
    Ok(())
}

pub(crate) fn runtime_harness_smoke() -> Result<(), String> {
    let suite = run_runtime_harness_suite()?;
    for case in &suite.cases {
        println!(
            "harness case={} passed={} events={} tools={} models={} health={:?} detail={}",
            case.case_id,
            case.passed,
            case.events,
            case.tools,
            case.models,
            case.health,
            case.detail
        );
    }
    if !suite.passed {
        return Err(format!("runtime harness failed: {:?}", suite));
    }
    println!("{}", suite.to_summary_line());
    Ok(())
}

pub(crate) fn event_invariant_smoke() -> Result<(), String> {
    let coding = run_no_model_coding_fixture(&NoModelCodingFixtureConfig::default())?;
    let coding_log =
        EventLog::import_jsonl(&coding.event_jsonl).map_err(|error| format!("{error:?}"))?;
    let coding_report = validate_event_invariants(&coding_log);
    if !coding_report.ok {
        return Err(format!(
            "coding fixture invariant failure: {:?}",
            coding_report
        ));
    }
    let blocked = run_scripted_native_agent_loop_external_block_fixture()?;
    let blocked_log =
        EventLog::import_jsonl(&blocked.event_jsonl).map_err(|error| format!("{error:?}"))?;
    let blocked_report = validate_event_invariants(&blocked_log);
    if !blocked_report.ok {
        return Err(format!(
            "blocked fixture invariant failure: {:?}",
            blocked_report
        ));
    }
    let resumed = run_scripted_native_agent_loop_provided_permission_fixture()?;
    let resumed_log = EventLog::import_jsonl(&resumed.loop_result.event_jsonl)
        .map_err(|error| format!("{error:?}"))?;
    let resumed_report = validate_event_invariants(&resumed_log);
    if !resumed_report.ok {
        return Err(format!(
            "resumed fixture invariant failure: {:?}",
            resumed_report
        ));
    }
    println!(
        "event invariant smoke coding={} blocked={} resumed={}",
        coding_report.checked_events, blocked_report.checked_events, resumed_report.checked_events
    );
    Ok(())
}

pub(crate) fn approval_queue_smoke() -> Result<(), String> {
    let blocked = run_scripted_native_agent_loop_external_block_fixture()?;
    let blocked_log =
        EventLog::import_jsonl(&blocked.event_jsonl).map_err(|error| format!("{error:?}"))?;
    let blocked_queue = extract_approval_queue(&blocked_log);
    if blocked_queue.permissions.len() != 1 || !blocked_queue.plan_approvals.is_empty() {
        return Err(format!(
            "blocked permission queue mismatch: {blocked_queue:?}"
        ));
    }

    let mut session = AgentSession::new("proj", "sess_plan_queue", "task_plan_queue")
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .map_err(|error| format!("{error:?}"))?;
    session
        .request_plan_approval("plan_queue_1", None)
        .map_err(|error| format!("{error:?}"))?;
    let plan_log = EventLog::import_jsonl(&session.export_events_jsonl())
        .map_err(|error| format!("{error:?}"))?;
    let plan_queue = extract_approval_queue(&plan_log);
    if plan_queue.plan_approvals.len() != 1 || !plan_queue.permissions.is_empty() {
        return Err(format!("plan approval queue mismatch: {plan_queue:?}"));
    }
    println!(
        "approval queue blocked={} plan={}",
        blocked_queue.to_summary_line(),
        plan_queue.to_summary_line()
    );
    Ok(())
}

pub(crate) fn permission_policy_smoke() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!(
        "researchcode-permission-policy-cli-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    let policy_path = root.join("permission_policy.tsv");
    let store = PermissionRuleStore::new(&policy_path);
    store
        .add_rule(PermissionRule {
            rule_id: "cli_project_test_rule".to_string(),
            scope: PermissionRuleScope::Project,
            request_type: researchcode_kernel::PermissionRequestType::Command,
            tool_id: "shell.command".to_string(),
            pattern_kind: PermissionPatternKind::Exact,
            pattern: "command: cargo test".to_string(),
            decision: PermissionRuleDecision::Allow,
            reason: "CLI smoke project allow".to_string(),
        })
        .map_err(|error| error.to_string())?;
    let loaded = store.load()?;
    let matched = loaded
        .find_match(
            &researchcode_kernel::PermissionRequestType::Command,
            "shell.command",
            "command: cargo test",
        )
        .ok_or_else(|| "persisted permission rule did not match".to_string())?;
    if matched.decision != PermissionRuleDecision::Allow {
        return Err("persisted permission rule decision mismatch".to_string());
    }

    let facade = RuntimeFacade::new(&root, root.join("facade_artifacts"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::ManualReview,
    )?;
    let args = ToolExecutionArgs {
        command: Some("find . -maxdepth 0".to_string()),
        ..ToolExecutionArgs::default()
    };
    let pending = facade.execute_session_tool(
        &handle.session_id,
        "permission_policy_cli_find_1",
        "shell.command",
        args.clone(),
    )?;
    if !matches!(pending, FacadeToolOutcome::RequiresPermission { .. }) {
        return Err("manual review should request permission before rule".to_string());
    }
    facade.continue_session_tool_after_permission(
        &handle.session_id,
        "permission_policy_cli_find_1",
        "shell.command",
        args.clone(),
        PermissionDecisionKind::AllowSession,
    )?;
    let session_rule = facade.execute_session_tool(
        &handle.session_id,
        "permission_policy_cli_find_2",
        "shell.command",
        args,
    )?;
    if !matches!(session_rule, FacadeToolOutcome::Executed(_)) {
        return Err("session allow rule did not auto-execute".to_string());
    }
    let _ = fs::remove_dir_all(&root);
    println!(
        "permission policy smoke rules={} facade_session_rule=true",
        loaded.rules.len()
    );
    Ok(())
}

pub(crate) fn runtime_facade_v2_smoke() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!(
        "researchcode-runtime-facade-v2-cli-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    fs::write(
        root.join("README.md"),
        "ResearchCode RuntimeFacade v2 smoke\n",
    )
    .map_err(|error| error.to_string())?;
    let facade = RuntimeFacade::new(&root, root.join("artifacts"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::FastAuto,
    )?;
    facade.submit_user_message(&handle.session_id, "runtime facade v2 smoke")?;
    let read = facade.execute_session_tool(
        &handle.session_id,
        "facade_v2_read",
        "file.read",
        ToolExecutionArgs {
            path: Some("README.md".to_string()),
            ..ToolExecutionArgs::default()
        },
    )?;
    if !matches!(read, FacadeToolOutcome::Executed(_)) {
        return Err("facade v2 read did not execute".to_string());
    }
    let transport = ScriptedLiveHttpTransport::new(vec![
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"file.read","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"README.md\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                .to_string(),
        },
        LiveHttpResponse {
            status_code: 200,
            body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Facade loop completed."}}
data: [DONE]"#
                .to_string(),
        },
    ]);
    let result = facade.run_deepseek_agent_loop_with_transport(
        &transport,
        &handle.session_id,
        "Read README through the runtime facade and finish.",
        NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
        4,
        4,
    )?;
    if result.status != NativeAgentLoopStatus::Completed {
        return Err(format!(
            "facade native loop did not complete: {:?}",
            result.status
        ));
    }
    let events = facade.stream_agent_events(&handle.session_id)?;
    if !events.jsonl.contains("tool.call_completed") || !events.jsonl.contains("model.call_started")
    {
        return Err("facade v2 event stream missing model/tool events".to_string());
    }
    let bundle = facade.build_context_bundle(&handle.session_id)?;
    if !bundle
        .items
        .iter()
        .any(|item| item.source == "runtime.session_memory")
    {
        return Err("facade v2 context missing session memory".to_string());
    }
    let _ = fs::remove_dir_all(&root);
    println!(
        "runtime facade v2 ok events={} models={} tools={} context_items={}",
        result.event_count,
        result.model_call_count,
        result.tool_call_count,
        bundle.items.len()
    );
    Ok(())
}

pub(crate) fn runtime_facade_ask_user_smoke() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!(
        "researchcode-runtime-facade-ask-user-cli-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let facade = RuntimeFacade::new(&root, root.join("artifacts"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::FastAuto,
    )?;
    let outcome = facade.execute_session_tool(
        &handle.session_id,
        "runtime_ask_user_1",
        "ask_user",
        ToolExecutionArgs {
            query: Some("Which file should I inspect first?".to_string()),
            ..ToolExecutionArgs::default()
        },
    )?;
    let snapshot = facade.get_session_snapshot(&handle.session_id)?;
    let events = facade.stream_agent_events(&handle.session_id)?.jsonl;
    let _ = fs::remove_dir_all(&root);
    if !matches!(outcome, FacadeToolOutcome::Executed(_))
        || snapshot.state != AgentState::WaitingForUser
        || snapshot.pending_permission_count != 0
        || snapshot.pending_plan_approval_count != 0
        || !events.contains("user.question_requested")
        || events.contains("permission.requested")
    {
        return Err("RuntimeFacade ask_user did not route to WaitingForUser".to_string());
    }
    println!(
        "runtime facade ask_user smoke passed events={} state={:?}",
        snapshot.event_count, snapshot.state
    );
    Ok(())
}

pub(crate) fn runtime_facade_event_delta_smoke() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!(
        "researchcode-runtime-facade-event-delta-cli-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let facade = RuntimeFacade::new(&root, root.join("artifacts"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::FastAuto,
    )?;
    let first = facade.stream_agent_events_since(&handle.session_id, 0, Some(2))?;
    facade.submit_user_message(&handle.session_id, "event delta smoke")?;
    let second = facade.stream_agent_events_since(&handle.session_id, first.next_cursor, None)?;
    let _ = fs::remove_dir_all(&root);
    if first.next_cursor != 2
        || first.jsonl.lines().count() != 2
        || second.next_cursor <= second.from_cursor
        || !second.jsonl.contains("event delta smoke")
        || second.has_more
    {
        return Err("RuntimeFacade event delta cursor did not return expected slices".to_string());
    }
    println!(
        "runtime facade event delta smoke passed first={} second={} cursor={}",
        first.jsonl.lines().count(),
        second.jsonl.lines().count(),
        second.next_cursor
    );
    Ok(())
}

pub(crate) fn tool_harness_smoke() -> Result<(), String> {
    let suite = run_core_tool_harness_suite();
    for case in &suite.cases {
        println!(
            "tool case={} tool={} passed={} detail={}",
            case.case_id, case.tool_id, case.passed, case.detail
        );
    }
    if !suite.passed {
        return Err(format!("tool harness failed: {:?}", suite));
    }
    println!("{}", suite.to_summary_line());
    Ok(())
}

pub(crate) fn patch_set_smoke() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!(
        "researchcode-patch-set-smoke-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).map_err(|error| error.to_string())?;
    fs::write(root.join("src/a.rs"), "pub const A: u8 = 1;\n")
        .map_err(|error| error.to_string())?;
    fs::write(root.join("src/b.rs"), "pub const B: u8 = 2;\n")
        .map_err(|error| error.to_string())?;
    let stale = PatchSetProposal {
        patch_set_id: "patch_set_stale".to_string(),
        operations: vec![
            PatchSetOperation {
                path: "src/a.rs".to_string(),
                old_string: "A: u8 = 1".to_string(),
                new_string: "A: u8 = 10".to_string(),
                base_hash: stable_text_hash("pub const A: u8 = 1;\n"),
            },
            PatchSetOperation {
                path: "src/b.rs".to_string(),
                old_string: "B: u8 = 2".to_string(),
                new_string: "B: u8 = 20".to_string(),
                base_hash: "stale_hash".to_string(),
            },
        ],
    };
    let stale_result = apply_patch_set_atomic(&root, &stale);
    if !matches!(stale_result, Err(PatchSetError::Validation(_))) {
        return Err(format!(
            "stale patch set unexpectedly applied: {stale_result:?}"
        ));
    }
    if fs::read_to_string(root.join("src/a.rs")).map_err(|error| error.to_string())?
        != "pub const A: u8 = 1;\n"
    {
        return Err("patch set wrote before full validation passed".to_string());
    }
    let proposal = PatchSetProposal {
        patch_set_id: "patch_set_ok".to_string(),
        operations: vec![
            PatchSetOperation {
                path: "src/a.rs".to_string(),
                old_string: "A: u8 = 1".to_string(),
                new_string: "A: u8 = 10".to_string(),
                base_hash: stable_text_hash("pub const A: u8 = 1;\n"),
            },
            PatchSetOperation {
                path: "src/b.rs".to_string(),
                old_string: "B: u8 = 2".to_string(),
                new_string: "B: u8 = 20".to_string(),
                base_hash: stable_text_hash("pub const B: u8 = 2;\n"),
            },
        ],
    };
    let validation = validate_patch_set(&root, &proposal).map_err(|error| format!("{error:?}"))?;
    let applied = apply_patch_set_atomic(&root, &proposal).map_err(|error| format!("{error:?}"))?;
    let a = fs::read_to_string(root.join("src/a.rs")).map_err(|error| error.to_string())?;
    let b = fs::read_to_string(root.join("src/b.rs")).map_err(|error| error.to_string())?;
    let _ = fs::remove_dir_all(root);
    if !a.contains("A: u8 = 10") || !b.contains("B: u8 = 20") {
        return Err("patch set final content mismatch".to_string());
    }
    println!(
        "{} applied_paths={}",
        validation.to_summary_line(),
        applied.applied_paths.len()
    );
    Ok(())
}

pub(crate) fn research_harness_smoke() -> Result<(), String> {
    let suite = run_research_harness_suite();
    for case in &suite.cases {
        println!(
            "research case={} passed={} detail={}",
            case.case_id, case.passed, case.detail
        );
    }
    if !suite.passed {
        return Err(format!("research harness failed: {:?}", suite));
    }
    println!("{}", suite.to_summary_line());
    Ok(())
}

pub(crate) fn foundation_harness_smoke() -> Result<(), String> {
    let runtime = run_runtime_harness_suite()?;
    if !runtime.passed {
        return Err(format!("runtime harness failed: {:?}", runtime));
    }
    let tools = run_core_tool_harness_suite();
    if !tools.passed {
        return Err(format!("tool harness failed: {:?}", tools));
    }
    let research = run_research_harness_suite();
    if !research.passed {
        return Err(format!("research harness failed: {:?}", research));
    }
    event_invariant_smoke()?;
    approval_queue_smoke()?;
    patch_set_smoke()?;
    fast_auto_policy_smoke()?;
    deepseek_tool_result_continuation_smoke()?;
    qwen_tool_result_continuation_smoke()?;
    println!(
        "foundation harness ok runtime_cases={} tool_cases={} research_cases={} runtime_events={}",
        runtime.cases.len(),
        tools.cases.len(),
        research.cases.len(),
        runtime.total_events()
    );
    Ok(())
}

pub(crate) fn context_budget_smoke() -> Result<(), String> {
    let budgets = vec![
        allocate_native_context_budget(NativeModelFamily::DeepSeek, ModelRole::Planner, None),
        allocate_native_context_budget(NativeModelFamily::Qwen, ModelRole::Executor, None),
        allocate_native_context_budget(NativeModelFamily::Qwen, ModelRole::Planner, Some(128_000)),
    ];
    for budget in &budgets {
        let validation = validate_context_budget(budget);
        println!(
            "{} {}",
            budget.to_summary_line(),
            validation.to_summary_line()
        );
        if !validation.ok {
            return Err(format!("invalid context budget: {:?}", validation.errors));
        }
    }
    let deepseek = &budgets[0];
    let qwen = &budgets[1];
    if deepseek.prompt_scaffold_tokens() <= qwen.prompt_scaffold_tokens() {
        return Err("DeepSeek full scaffold should be richer than Qwen fast scaffold".to_string());
    }
    if qwen.prompt_scaffold_tokens() >= qwen.max_context_tokens / 10 {
        return Err("Qwen prompt scaffold exceeded 10% guardrail".to_string());
    }
    Ok(())
}

pub(crate) fn context_budget_show(family: &str, role: &str) -> Result<(), String> {
    let family = parse_native_family(family)?;
    let role = parse_model_role(role)?;
    let budget = allocate_native_context_budget(family, role, None);
    let validation = validate_context_budget(&budget);
    println!("{}", budget.to_summary_line());
    println!("{}", context_budget_json(&budget));
    if validation.ok {
        Ok(())
    } else {
        Err(format!("invalid context budget: {:?}", validation.errors))
    }
}

pub(crate) fn parse_native_family(value: &str) -> Result<NativeModelFamily, String> {
    match value {
        "deepseek" => Ok(NativeModelFamily::DeepSeek),
        "qwen" => Ok(NativeModelFamily::Qwen),
        other => Err(format!("unknown native family {other}")),
    }
}

pub(crate) fn parse_model_role(value: &str) -> Result<ModelRole, String> {
    match value {
        "planner" => Ok(ModelRole::Planner),
        "executor" => Ok(ModelRole::Executor),
        "reviewer" => Ok(ModelRole::Reviewer),
        "researcher" => Ok(ModelRole::Researcher),
        "summarizer" => Ok(ModelRole::Summarizer),
        other => Err(format!("unknown role {other}")),
    }
}

pub(crate) fn context_budget_json(budget: &ContextBudget) -> String {
    format!(
        "{{\"model_id\":\"{}\",\"scaffold_level\":\"{:?}\",\"max_context_tokens\":{},\"prompt_scaffold_tokens\":{},\"dynamic_context_tokens\":{},\"output_reserve_tokens\":{},\"emergency_reserve_tokens\":{},\"reasoning_replay_budget\":{},\"compaction_threshold\":{},\"compaction_floor\":{},\"max_active_tools\":{},\"max_files_per_turn\":{}}}",
        budget.model_id,
        budget.scaffold_level,
        budget.max_context_tokens,
        budget.prompt_scaffold_tokens(),
        budget.dynamic_context_tokens(),
        budget.output_reserve_tokens,
        budget.emergency_reserve_tokens,
        budget.reasoning_replay_budget,
        budget.compaction_threshold,
        budget.compaction_floor,
        budget.max_active_tools,
        budget.max_files_per_turn
    )
}

pub(crate) fn run_recorded_model_fixture_smoke() -> Result<(), String> {
    let result = run_recorded_model_planned_fixture(&NoModelCodingFixtureConfig::default())?;
    println!(
        "recorded model fixture completed state={:?} events={} deepseek_tool={} qwen_tool={} qwen_mismatch={:?}",
        result.final_state,
        result.event_count,
        result.deepseek_tool_id,
        result.qwen_tool_id,
        result.qwen_mismatch_action
    );
    Ok(())
}

pub(crate) fn run_recorded_patch_fixture_smoke() -> Result<(), String> {
    let result = run_recorded_patch_fixture(&NoModelCodingFixtureConfig::default())?;
    println!(
        "recorded patch fixture completed state={:?} events={} stale={:?} ambiguous={:?} deepseek={:?}",
        result.final_state,
        result.event_count,
        result.qwen_stale_validation,
        result.qwen_ambiguous_validation,
        result.deepseek_patch_validation
    );
    Ok(())
}

pub(crate) fn run_recorded_live_response_fixture_smoke() -> Result<(), String> {
    run_recorded_live_response_fixture_cli(None)
}

pub(crate) fn run_recorded_live_response_fixture_cli(
    output_path: Option<PathBuf>,
) -> Result<(), String> {
    let result =
        run_recorded_live_response_fixture_runtime(&NoModelCodingFixtureConfig::default())?;
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&output_path, &result.event_jsonl).map_err(|error| error.to_string())?;
    }
    println!(
        "recorded live response fixture completed state={:?} events={} deepseek={} qwen={}",
        result.final_state,
        result.event_count,
        result.deepseek_transcript_hash,
        result.qwen_transcript_hash
    );
    Ok(())
}
