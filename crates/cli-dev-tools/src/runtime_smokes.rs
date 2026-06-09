#![allow(unused_imports, dead_code)]

use crate::agent_smokes::*;
use crate::core_smokes::*;
use crate::deepseek::*;
use crate::fixtures::*;
use crate::helpers::*;
use crate::live_model::*;
use crate::prelude::*;
use crate::qwen_tools::*;
pub(crate) fn provider_tool_schema_smoke() -> Result<(), String> {
    let tui_schema = deepseek_tui_tool_schema_json();
    let runtime_schema = native_readonly_provider_tool_schema_json();
    for required in [
        "\"name\":\"file_read\"",
        "\"name\":\"repo_map\"",
        "\"name\":\"search_ripgrep\"",
        "\"name\":\"git_status\"",
        "\"name\":\"plan_enter\"",
    ] {
        if !tui_schema.contains(required) {
            return Err(format!("TUI provider schema missing {required}"));
        }
        if !runtime_schema.contains(required) {
            return Err(format!("runtime provider schema missing {required}"));
        }
    }
    for required in [
        "\"name\":\"file_write\"",
        "\"name\":\"file_edit\"",
        "\"name\":\"file_multi_edit\"",
    ] {
        if !tui_schema.contains(required) {
            return Err(format!("TUI FastAuto provider schema missing {required}"));
        }
        if runtime_schema.contains(required) {
            return Err(format!(
                "runtime read-only provider schema unexpectedly exposes {required}"
            ));
        }
    }
    for forbidden in [
        "artifact_export",
        "worktree_create",
        "mcp_tool",
        "shell_command",
        "patch_apply",
    ] {
        if tui_schema.contains(forbidden) || runtime_schema.contains(forbidden) {
            return Err(format!(
                "provider schema exposed forbidden tool {forbidden}"
            ));
        }
    }
    println!(
        "provider tool schema smoke ok tui_bytes={} runtime_bytes={}",
        tui_schema.len(),
        runtime_schema.len()
    );
    Ok(())
}

pub(crate) fn tool_contract_mediation_smoke() -> Result<(), String> {
    let call = mediate_tool_call("read_source_code", r#"{"path":"README.md"}"#);
    if call.tool_id != "file.read" || call.error.is_some() {
        return Err(format!("alias mediation failed: {:?}", call));
    }
    if !call
        .events
        .iter()
        .any(|event| event.event_type == "tool.name.alias_resolved")
    {
        return Err("alias mediation did not emit tool.name.alias_resolved".to_string());
    }
    println!(
        "tool contract mediation ok tool={} events={}",
        call.tool_id,
        call.events.len()
    );
    Ok(())
}

pub(crate) fn tool_manifest_doctor_smoke() -> Result<(), String> {
    let manifest = build_tool_manifest();
    let report = run_tool_manifest_doctor();
    if !report.ok {
        return Err(format!(
            "tool manifest doctor failed: {:?}",
            report.failures
        ));
    }
    println!(
        "tool manifest doctor ok hash={} tools={} provider_names={}",
        report.manifest_hash,
        report.checked_tools,
        manifest.provider_tool_names.len()
    );
    Ok(())
}

pub(crate) fn unknown_tool_recovery_smoke() -> Result<(), String> {
    let call = mediate_tool_call("made_up_reader", r#"{"path":"README.md"}"#);
    if call.status != ToolMediationStatus::Rejected {
        return Err("unknown tool was not rejected".to_string());
    }
    let Some(error) = call.error else {
        return Err("unknown tool did not produce model-readable error".to_string());
    };
    if error.error_code != "UNKNOWN_TOOL" || !error.retryable {
        return Err(format!("unexpected unknown tool error: {:?}", error));
    }
    println!(
        "unknown tool recovery ok code={} suggestion={}",
        error.error_code,
        error
            .suggested_replacement
            .unwrap_or_else(|| "none".to_string())
    );
    Ok(())
}

pub(crate) fn tool_input_repair_smoke() -> Result<(), String> {
    let call = mediate_tool_call("file_read", r#"{"path":"README.md","limit":2000}"#);
    if call.arguments.offset != Some(0) {
        return Err(format!("limit without offset was not repaired: {:?}", call));
    }
    if !call
        .events
        .iter()
        .any(|event| event.event_type == "tool.relational_default_applied")
    {
        return Err("repair did not emit relational default event".to_string());
    }
    let write = mediate_tool_call("file_write", r#"{"path":"x.html","content":null}"#);
    if write.error.is_none()
        || write
            .repairs
            .iter()
            .any(|repair| repair.issue_path == "content")
    {
        return Err("file.write.content was improperly auto-repaired".to_string());
    }
    let shell = mediate_tool_call("shell_command", r#"{"command":null}"#);
    if shell.error.is_none()
        || shell
            .repairs
            .iter()
            .any(|repair| repair.issue_path == "command")
    {
        return Err("shell.command.command was improperly auto-repaired".to_string());
    }
    println!("tool input repair ok repairs={}", call.repairs.len());
    Ok(())
}

pub(crate) fn eventlog_dsml_braces_smoke() -> Result<(), String> {
    let mut session = AgentSession::new("proj_tool_contract", "sess_tool_contract", "task")
        .map_err(|error| format!("{error:?}"))?;
    session
        .record_runtime_event(
            "model.stream_delta",
            researchcode_kernel::Actor::Model,
            "{\"stream_id\":\"s\",\"provider\":\"deepseek\",\"delta_kind\":\"content\",\"preview\":\"<｜｜DSML｜｜tool_calls>{\\\"path\\\":\\\"README.md\\\",\\\"nested\\\":{\\\"limit\\\":2000}}</｜｜DSML｜｜tool_calls>\"}".to_string(),
        )
        .map_err(|error| format!("{error:?}"))?;
    let jsonl = session.export_events_jsonl();
    let imported = EventLog::import_jsonl(&jsonl).map_err(|error| format!("{error:?}"))?;
    if imported.len() < 1 || !imported.export_jsonl().contains("<｜｜DSML｜｜tool_calls>") {
        return Err(format!(
            "expected imported DSML stream event, got {} events",
            imported.len()
        ));
    }
    println!("eventlog dsml braces ok events={}", imported.len());
    Ok(())
}

pub(crate) fn deepseek_content_tool_fallback_smoke() -> Result<(), String> {
    let raw = r#"<｜｜DSML｜｜tool_calls><｜｜DSML｜｜invoke name="file_read"><｜｜DSML｜｜parameter name="path" string="true">README.md</｜｜DSML｜｜parameter><｜｜DSML｜｜parameter name="limit" string="false">2000</｜｜DSML｜｜parameter></｜｜DSML｜｜invoke></｜｜DSML｜｜tool_calls>"#;
    let calls = extract_content_tool_call_candidates(raw);
    if calls.len() != 1 {
        return Err(format!(
            "expected one DeepSeek fallback candidate, got {}",
            calls.len()
        ));
    }
    let mediated = mediate_tool_call(&calls[0].tool_id, &calls[0].arguments_json);
    if mediated.tool_id != "file.read" || mediated.arguments.offset != Some(0) {
        return Err(format!(
            "DeepSeek fallback mediation failed: {:?}",
            mediated
        ));
    }
    println!("deepseek content fallback ok tool={}", mediated.tool_id);
    Ok(())
}

pub(crate) fn qwen_tool_mediation_fixture_smoke() -> Result<(), String> {
    let call = mediate_tool_call(
        "file_read",
        r#"{"path":"README.md","root":null,"limit":1200}"#,
    );
    if call.tool_id != "file.read" || call.arguments.offset != Some(0) {
        return Err(format!("Qwen fixture mediation failed: {:?}", call));
    }
    if !call
        .events
        .iter()
        .any(|event| event.event_type == "tool.input_repaired")
    {
        return Err("Qwen fixture did not record repair telemetry".to_string());
    }
    let mut accumulator = StreamingToolCallAccumulator::default();
    let delta_event = accumulator.push_delta("qwen_call_0", r#"{"path":"README"#);
    let (assembled, done_event) = accumulator.complete("qwen_call_0");
    if !delta_event.event_type.ends_with("delta_received")
        || !done_event.event_type.ends_with("assembly_completed")
        || assembled.is_empty()
    {
        return Err("Qwen accumulator telemetry failed".to_string());
    }
    println!("qwen tool mediation fixture ok tool={}", call.tool_id);
    Ok(())
}

pub(crate) fn tool_ledger_exactly_once_smoke() -> Result<(), String> {
    let mut ledger = ToolCallLedger::default();
    ledger.propose("toolu_1");
    ledger.propose("toolu_2");
    if !ledger.record_result("toolu_1") {
        return Err("first result was marked duplicate".to_string());
    }
    if ledger.record_result("toolu_1") {
        return Err("duplicate result was not detected".to_string());
    }
    if ledger.missing_results() != vec!["toolu_2".to_string()] {
        return Err(format!(
            "missing results mismatch: {:?}",
            ledger.missing_results()
        ));
    }
    println!(
        "tool ledger exactly-once ok missing={} duplicate={}",
        ledger.missing_results().len(),
        ledger.duplicate_results().len()
    );
    Ok(())
}

pub(crate) fn session_terminal_reopen_smoke() -> Result<(), String> {
    let mut session = AgentSession::new(
        "proj_terminal_reopen",
        "sess_terminal_reopen",
        "task_terminal_reopen",
    )
    .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .and_then(|_| session.start_review())
        .and_then(|_| session.complete_after_review())
        .map_err(|error| format!("{error:?}"))?;
    if session.state() != AgentState::Completed {
        return Err(format!(
            "session did not reach Completed before reopen: {:?}",
            session.state()
        ));
    }
    session
        .begin_interactive_turn("terminal_reopen_smoke", "next_user_turn")
        .map_err(|error| format!("{error:?}"))?;
    let jsonl = session.export_events_jsonl();
    if session.state() != AgentState::Executing
        || !jsonl.contains("\"session.turn_started\"")
        || !jsonl.contains("\"from_state\":\"Completed\"")
    {
        return Err("terminal state reopen did not emit expected turn event".to_string());
    }
    println!("session terminal reopen smoke passed");
    Ok(())
}

pub(crate) fn loop_recovery_directory_smoke() -> Result<(), String> {
    let root = temp_smoke_root("researchcode-loop-recovery-directory")?;
    fs::create_dir_all(root.join("nested")).map_err(|error| error.to_string())?;
    fs::write(root.join("nested").join("README.md"), "nested readme\n")
        .map_err(|error| error.to_string())?;
    let facade = RuntimeFacade::new(&root, root.join(".researchcode"));
    let handle = facade.start_session(
        Some(root.clone()),
        RuntimeModelMode::DeepSeek,
        AutonomyMode::FastAuto,
    )?;
    let result = facade.execute_session_tool(
        &handle.session_id,
        "dir_read_1",
        "file.read",
        ToolExecutionArgs {
            path: Some("nested".to_string()),
            ..ToolExecutionArgs::default()
        },
    )?;
    let FacadeToolOutcome::Executed(result) = result else {
        let _ = fs::remove_dir_all(&root);
        return Err("directory read did not return executed structured result".to_string());
    };
    let context = facade.build_context_bundle(&handle.session_id)?;
    let jsonl = facade.stream_agent_events(&handle.session_id)?.jsonl;
    let _ = fs::remove_dir_all(&root);
    if result.ok
        || !result.detail_json.contains("path_is_directory")
        || !context
            .items
            .iter()
            .any(|item| item.content.contains("path correction"))
        || !jsonl.contains("tool.result_recorded")
    {
        return Err("directory read recovery did not record path correction memory".to_string());
    }
    println!("loop recovery directory smoke passed");
    Ok(())
}
