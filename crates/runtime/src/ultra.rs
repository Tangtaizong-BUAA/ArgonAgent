//! DeepSeek-only UltraPlan / UltraReview fixture runtime.

use crate::agent_team::{validate_final_claims, AgentTeamMode, AgentTeamRun, EvidenceLedger};
use crate::patch::stable_text_hash;
use researchcode_kernel::model::NativeModelFamily;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UltraPlanSpec {
    pub plan_id: String,
    pub goal: String,
    pub non_goals: Vec<String>,
    pub chosen_strategy: String,
    pub rejected_strategies: Vec<String>,
    pub assumptions: Vec<String>,
    pub phases: Vec<String>,
    pub executable_tasks: Vec<String>,
    pub model_strategy: String,
    pub multi_agent_strategy: String,
    pub risk_register: Vec<String>,
    pub test_eval_plan: Vec<String>,
    pub rollback_plan: Vec<String>,
    pub go_no_go_gates: Vec<String>,
    pub open_questions: Vec<String>,
    pub evidence_refs: Vec<String>,
}

impl UltraPlanSpec {
    pub fn validate(&self, ledger: &EvidenceLedger) -> Result<(), String> {
        validate_deepseek_only(NativeModelFamily::DeepSeek)?;
        validate_final_claims(&self.evidence_refs, ledger)?;
        if self.rejected_strategies.is_empty() {
            return Err("UltraPlan requires rejected_strategies".to_string());
        }
        if self.test_eval_plan.is_empty() {
            return Err("UltraPlan requires test_eval_plan".to_string());
        }
        if self.rollback_plan.is_empty() {
            return Err("UltraPlan requires rollback_plan".to_string());
        }
        if self.go_no_go_gates.is_empty() {
            return Err("UltraPlan requires go_no_go_gates".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UltraReviewFinding {
    pub finding_id: String,
    pub file: String,
    pub expected: String,
    pub actual: String,
    pub confidence: String,
    pub recommended_test: String,
    pub evidence_refs: Vec<String>,
    pub reproduced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UltraReviewReport {
    pub report_id: String,
    pub reviewed_files: Vec<String>,
    pub verified_findings: Vec<UltraReviewFinding>,
    pub unverified_concerns: Vec<UltraReviewFinding>,
    pub rejected_findings: Vec<String>,
    pub tests_run: Vec<String>,
    pub reproduction_artifacts: Vec<String>,
    pub model_usage: String,
    pub overall_status: String,
}

impl UltraReviewReport {
    pub fn validate(&self, ledger: &EvidenceLedger) -> Result<(), String> {
        validate_deepseek_only(NativeModelFamily::DeepSeek)?;
        for finding in &self.verified_findings {
            if !finding.reproduced {
                return Err("No reproduction, no verified finding".to_string());
            }
            validate_final_claims(&finding.evidence_refs, ledger)?;
        }
        Ok(())
    }
}

pub fn validate_deepseek_only(family: NativeModelFamily) -> Result<(), String> {
    if family != NativeModelFamily::DeepSeek {
        return Err("UltraPlan/UltraReview v1 is DeepSeek-native only".to_string());
    }
    Ok(())
}

pub fn build_ultraplan_fixture(goal: &str) -> (AgentTeamRun, EvidenceLedger, UltraPlanSpec) {
    let mut ledger = EvidenceLedger::default();
    let evidence = ledger.add_note(
        "codebase_scout",
        "AGENTS.md",
        "RuntimeFacade/EventLog is the TUI/GUI boundary.",
    );
    let plan_id = format!("ultraplan_{}", stable_text_hash(goal));
    let team = AgentTeamRun {
        team_id: format!("team_{plan_id}"),
        parent_session_id: "fixture_parent".to_string(),
        mode: AgentTeamMode::UltraPlan,
        max_agents: 5,
        required_evidence: true,
        required_integrator: true,
        required_judge: true,
        allow_full_mesh: false,
        status: "completed".to_string(),
    };
    let spec = UltraPlanSpec {
        plan_id,
        goal: goal.to_string(),
        non_goals: vec!["Do not start implementation before plan approval.".to_string()],
        chosen_strategy: "Fix Runtime loop/session memory before AgentTeams.".to_string(),
        rejected_strategies: vec![
            "Do not implement multi-agent on top of a brittle loop.".to_string()
        ],
        assumptions: vec!["DeepSeek native mode is available for UltraPlan v1.".to_string()],
        phases: vec![
            "Loop recovery".to_string(),
            "Subagent runtime".to_string(),
            "AgentTeams evidence flow".to_string(),
        ],
        executable_tasks: vec!["Add loop-recovery smoke.".to_string()],
        model_strategy: "DeepSeek Pro for judge/integrator; Flash for scouts.".to_string(),
        multi_agent_strategy: "Small team only; no full-mesh chat.".to_string(),
        risk_register: vec![
            "AgentTeams can amplify context drift if loop memory is weak.".to_string(),
        ],
        test_eval_plan: vec!["Run ultraplan-fixture-smoke and loop recovery smokes.".to_string()],
        rollback_plan: vec!["Disable /ultraplan command and keep Subagent read-only.".to_string()],
        go_no_go_gates: vec!["EvidenceLedger validation must pass.".to_string()],
        open_questions: Vec::new(),
        evidence_refs: vec![evidence],
    };
    (team, ledger, spec)
}

pub fn build_ultrareview_fixture(
    target: &str,
) -> (AgentTeamRun, EvidenceLedger, UltraReviewReport) {
    let mut ledger = EvidenceLedger::default();
    let evidence = ledger.add_note(
        "reproducer",
        target,
        "Fixture reproduction artifact confirms the reviewed issue path.",
    );
    let finding = UltraReviewFinding {
        finding_id: format!("finding_{}", stable_text_hash(target)),
        file: target.to_string(),
        expected: "reviewed behavior should be reproducible".to_string(),
        actual: "fixture reproduced the concern".to_string(),
        confidence: "high".to_string(),
        recommended_test: "ultrareview-fixture-smoke".to_string(),
        evidence_refs: vec![evidence.clone()],
        reproduced: true,
    };
    let team = AgentTeamRun {
        team_id: format!("team_ultrareview_{}", stable_text_hash(target)),
        parent_session_id: "fixture_parent".to_string(),
        mode: AgentTeamMode::UltraReview,
        max_agents: 4,
        required_evidence: true,
        required_integrator: true,
        required_judge: true,
        allow_full_mesh: false,
        status: "completed".to_string(),
    };
    let report = UltraReviewReport {
        report_id: format!("ultrareview_{}", stable_text_hash(target)),
        reviewed_files: vec![target.to_string()],
        verified_findings: vec![finding],
        unverified_concerns: Vec::new(),
        rejected_findings: Vec::new(),
        tests_run: vec!["fixture reproduction".to_string()],
        reproduction_artifacts: vec![evidence],
        model_usage: "DeepSeek Pro judge + Flash reviewer fixture".to_string(),
        overall_status: "pass_with_warnings".to_string(),
    };
    (team, ledger, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ultraplan_fixture_passes_gates() {
        let (_, ledger, plan) = build_ultraplan_fixture("ship runtime");
        plan.validate(&ledger).unwrap();
    }

    #[test]
    fn ultrareview_requires_reproduction_for_verified_findings() {
        let (_, ledger, mut report) = build_ultrareview_fixture("src/lib.rs");
        report.validate(&ledger).unwrap();
        report.verified_findings[0].reproduced = false;
        assert!(report.validate(&ledger).is_err());
    }
}
