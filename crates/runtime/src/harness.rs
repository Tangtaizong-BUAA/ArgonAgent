//! Runtime harness suite aggregating deterministic bottom-layer fixtures.
//!
//! The individual fixtures prove narrow contracts. This module gives CLI,
//! automation, and future CI one stable summary across coding, repair, model
//! loop, approval blocking, research, parser, and replay boundaries.

use crate::executor::{
    run_failure_repair_fixture, run_no_model_coding_fixture, run_recorded_model_planned_fixture,
    run_recorded_patch_fixture, NoModelCodingFixtureConfig,
};
use crate::native_agent_loop::{
    run_scripted_native_agent_loop_external_block_fixture,
    run_scripted_native_agent_loop_external_resume_fixture, run_scripted_native_agent_loop_fixture,
    run_scripted_native_agent_loop_provided_permission_fixture,
};
use crate::recorded_agent_loop::{run_recorded_agent_loop_fixture, RecordedAgentLoopConfig};
use crate::recorded_research_loop::{
    run_recorded_research_loop_fixture, RecordedResearchLoopConfig,
};
use crate::replay::{replay_jsonl, ReplayHealth};
use crate::state::AgentState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHarnessCaseResult {
    pub case_id: String,
    pub passed: bool,
    pub events: usize,
    pub tools: usize,
    pub models: usize,
    pub health: ReplayHealth,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHarnessSuiteResult {
    pub passed: bool,
    pub cases: Vec<RuntimeHarnessCaseResult>,
}

impl RuntimeHarnessSuiteResult {
    pub fn total_events(&self) -> usize {
        self.cases.iter().map(|case| case.events).sum()
    }

    pub fn passed_count(&self) -> usize {
        self.cases.iter().filter(|case| case.passed).count()
    }

    pub fn to_summary_line(&self) -> String {
        format!(
            "runtime harness passed={}/{} events={} ok={}",
            self.passed_count(),
            self.cases.len(),
            self.total_events(),
            self.passed
        )
    }
}

pub fn run_runtime_harness_suite() -> Result<RuntimeHarnessSuiteResult, String> {
    let mut cases = Vec::new();
    cases.push(run_coding_case()?);
    cases.push(run_failure_repair_case()?);
    cases.push(run_recorded_model_case()?);
    cases.push(run_recorded_patch_case()?);
    cases.push(run_recorded_agent_loop_case()?);
    cases.push(run_native_agent_loop_case()?);
    cases.push(run_blocked_permission_case()?);
    cases.push(run_provided_permission_resume_case()?);
    cases.push(run_external_decision_resume_case()?);
    cases.push(run_research_case()?);
    let passed = cases.iter().all(|case| case.passed);
    Ok(RuntimeHarnessSuiteResult { passed, cases })
}

fn run_coding_case() -> Result<RuntimeHarnessCaseResult, String> {
    let result = run_no_model_coding_fixture(&NoModelCodingFixtureConfig::default())?;
    let snapshot = replay_jsonl(&result.event_jsonl).map_err(|error| format!("{error:?}"))?;
    Ok(RuntimeHarnessCaseResult {
        case_id: "coding_no_model".to_string(),
        passed: result.final_state == AgentState::Completed
            && snapshot.health == ReplayHealth::Completed
            && snapshot.tool_results_recorded >= 1,
        events: result.event_count,
        tools: snapshot.tool_calls_completed,
        models: snapshot.model_calls_completed,
        health: snapshot.health,
        detail: format!("matches={}", result.matches_count),
    })
}

fn run_failure_repair_case() -> Result<RuntimeHarnessCaseResult, String> {
    let result = run_failure_repair_fixture(&NoModelCodingFixtureConfig::default())?;
    let snapshot = replay_jsonl(&result.event_jsonl).map_err(|error| format!("{error:?}"))?;
    Ok(RuntimeHarnessCaseResult {
        case_id: "failure_repair".to_string(),
        passed: result.final_state == AgentState::Completed
            && result.first_exit_code != 0
            && result.repaired_exit_code == 0
            && snapshot.health == ReplayHealth::Completed,
        events: result.event_count,
        tools: snapshot.tool_calls_completed,
        models: snapshot.model_calls_completed,
        health: snapshot.health,
        detail: format!(
            "first_exit={} repaired_exit={}",
            result.first_exit_code, result.repaired_exit_code
        ),
    })
}

fn run_recorded_model_case() -> Result<RuntimeHarnessCaseResult, String> {
    let result = run_recorded_model_planned_fixture(&NoModelCodingFixtureConfig::default())?;
    let snapshot = replay_jsonl(&result.event_jsonl).map_err(|error| format!("{error:?}"))?;
    Ok(RuntimeHarnessCaseResult {
        case_id: "recorded_model_planned".to_string(),
        passed: result.final_state == AgentState::Completed
            && snapshot.health == ReplayHealth::Completed
            && result.qwen_mismatch_action == crate::parser::ParserAction::BlockNativeSession,
        events: result.event_count,
        tools: snapshot.tool_calls_completed,
        models: snapshot.model_calls_completed,
        health: snapshot.health,
        detail: format!(
            "deepseek_tool={} qwen_tool={}",
            result.deepseek_tool_id, result.qwen_tool_id
        ),
    })
}

fn run_recorded_patch_case() -> Result<RuntimeHarnessCaseResult, String> {
    let result = run_recorded_patch_fixture(&NoModelCodingFixtureConfig::default())?;
    let snapshot = replay_jsonl(&result.event_jsonl).map_err(|error| format!("{error:?}"))?;
    Ok(RuntimeHarnessCaseResult {
        case_id: "recorded_patch_validation".to_string(),
        passed: result.final_state == AgentState::Completed
            && snapshot.health == ReplayHealth::Completed
            && matches!(
                result.qwen_stale_validation,
                crate::patch::PatchValidation::FailStale
            )
            && matches!(
                result.qwen_ambiguous_validation,
                crate::patch::PatchValidation::FailAmbiguous
            ),
        events: result.event_count,
        tools: snapshot.tool_calls_completed,
        models: snapshot.model_calls_completed,
        health: snapshot.health,
        detail: format!(
            "stale={:?} ambiguous={:?}",
            result.qwen_stale_validation, result.qwen_ambiguous_validation
        ),
    })
}

fn run_recorded_agent_loop_case() -> Result<RuntimeHarnessCaseResult, String> {
    let result = run_recorded_agent_loop_fixture(&RecordedAgentLoopConfig::default())?;
    let snapshot = replay_jsonl(&result.event_jsonl).map_err(|error| format!("{error:?}"))?;
    Ok(RuntimeHarnessCaseResult {
        case_id: "recorded_agent_loop".to_string(),
        passed: result.final_state == AgentState::Completed
            && snapshot.health == ReplayHealth::Completed
            && result.command_exit_code == 0,
        events: result.event_count,
        tools: snapshot.tool_calls_completed,
        models: snapshot.model_calls_completed,
        health: snapshot.health,
        detail: format!("file_hash={}", result.final_file_hash),
    })
}

fn run_native_agent_loop_case() -> Result<RuntimeHarnessCaseResult, String> {
    let result = run_scripted_native_agent_loop_fixture()?;
    let snapshot =
        replay_jsonl(&result.loop_result.event_jsonl).map_err(|error| format!("{error:?}"))?;
    Ok(RuntimeHarnessCaseResult {
        case_id: "native_agent_loop_scripted".to_string(),
        passed: result.loop_result.final_state == AgentState::Completed
            && snapshot.health == ReplayHealth::Completed
            && result.loop_result.tool_call_count >= 1,
        events: result.loop_result.event_count,
        tools: snapshot.tool_calls_completed,
        models: snapshot.model_calls_completed,
        health: snapshot.health,
        detail: format!("file_hash={}", result.final_file_hash),
    })
}

fn run_blocked_permission_case() -> Result<RuntimeHarnessCaseResult, String> {
    let result = run_scripted_native_agent_loop_external_block_fixture()?;
    let snapshot = replay_jsonl(&result.event_jsonl).map_err(|error| format!("{error:?}"))?;
    Ok(RuntimeHarnessCaseResult {
        case_id: "blocked_permission_boundary".to_string(),
        passed: result.final_state == AgentState::WaitingForToolApproval
            && snapshot.health == ReplayHealth::BlockedForPermission
            && snapshot.pending_permission_ids.len() == 1,
        events: result.event_count,
        tools: snapshot.tool_calls_completed,
        models: snapshot.model_calls_completed,
        health: snapshot.health,
        detail: format!("pending={:?}", snapshot.pending_permission_ids),
    })
}

fn run_provided_permission_resume_case() -> Result<RuntimeHarnessCaseResult, String> {
    let result = run_scripted_native_agent_loop_provided_permission_fixture()?;
    let snapshot =
        replay_jsonl(&result.loop_result.event_jsonl).map_err(|error| format!("{error:?}"))?;
    Ok(RuntimeHarnessCaseResult {
        case_id: "provided_permission_resume".to_string(),
        passed: result.loop_result.final_state == AgentState::Completed
            && snapshot.health == ReplayHealth::Completed
            && snapshot.permissions_requested == 1
            && snapshot.permissions_decided == 1
            && snapshot.tool_calls_completed == 1,
        events: result.loop_result.event_count,
        tools: snapshot.tool_calls_completed,
        models: snapshot.model_calls_completed,
        health: snapshot.health,
        detail: format!("file_hash={}", result.final_file_hash),
    })
}

fn run_external_decision_resume_case() -> Result<RuntimeHarnessCaseResult, String> {
    let result = run_scripted_native_agent_loop_external_resume_fixture()?;
    let snapshot =
        replay_jsonl(&result.loop_result.event_jsonl).map_err(|error| format!("{error:?}"))?;
    Ok(RuntimeHarnessCaseResult {
        case_id: "external_decision_resume".to_string(),
        passed: result.loop_result.final_state == AgentState::Completed
            && snapshot.health == ReplayHealth::Completed
            && snapshot.permissions_requested == 1
            && snapshot.permissions_decided == 1
            && snapshot.tool_calls_completed == 1
            && result.loop_result.model_call_count == 0
            && result.loop_result.tool_call_count == 1,
        events: result.loop_result.event_count,
        tools: snapshot.tool_calls_completed,
        models: snapshot.model_calls_completed,
        health: snapshot.health,
        detail: format!("file_hash={}", result.final_file_hash),
    })
}

fn run_research_case() -> Result<RuntimeHarnessCaseResult, String> {
    let result = run_recorded_research_loop_fixture(&RecordedResearchLoopConfig::default())?;
    let snapshot = replay_jsonl(&result.event_jsonl).map_err(|error| format!("{error:?}"))?;
    Ok(RuntimeHarnessCaseResult {
        case_id: "recorded_research_loop".to_string(),
        passed: result.final_state == AgentState::Completed
            && snapshot.health == ReplayHealth::Completed
            && result.artifact_count >= 5,
        events: result.event_count,
        tools: snapshot.tool_calls_completed,
        models: snapshot.model_calls_completed,
        health: snapshot.health,
        detail: format!("manifest_hash={}", result.manifest_hash),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_harness_suite_passes_core_cases() {
        let result = run_runtime_harness_suite().unwrap();
        assert!(result.passed, "{result:?}");
        assert_eq!(result.cases.len(), 10);
        assert!(result.total_events() > 100);
        assert!(result
            .cases
            .iter()
            .any(|case| case.health == ReplayHealth::BlockedForPermission));
    }
}
