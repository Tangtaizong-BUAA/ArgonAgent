//! Policy-driven AgentTeams v1.
//!
//! AgentTeams use a blackboard and evidence ledger instead of free-form
//! full-mesh chat. This keeps multi-agent runs auditable and GUI-replayable.

use crate::patch::stable_text_hash;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentTeamMode {
    Solo,
    Pair,
    SmallTeam,
    UltraPlan,
    UltraReview,
}

impl AgentTeamMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Solo => "solo",
            Self::Pair => "pair",
            Self::SmallTeam => "small_team",
            Self::UltraPlan => "ultraplan",
            Self::UltraReview => "ultrareview",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentTeamMessageKind {
    EvidenceNote,
    QuestionToTeam,
    Challenge,
    ReproductionRequest,
    ReproductionResult,
    ConflictNotice,
    DecisionRecord,
    SummaryForIntegrator,
}

impl AgentTeamMessageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EvidenceNote => "evidence_note",
            Self::QuestionToTeam => "question_to_team",
            Self::Challenge => "challenge",
            Self::ReproductionRequest => "reproduction_request",
            Self::ReproductionResult => "reproduction_result",
            Self::ConflictNotice => "conflict_notice",
            Self::DecisionRecord => "decision_record",
            Self::SummaryForIntegrator => "summary_for_integrator",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTeamRun {
    pub team_id: String,
    pub parent_session_id: String,
    pub mode: AgentTeamMode,
    pub max_agents: usize,
    pub required_evidence: bool,
    pub required_integrator: bool,
    pub required_judge: bool,
    pub allow_full_mesh: bool,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTeamMessage {
    pub message_id: String,
    pub team_id: String,
    pub from_agent_id: String,
    pub to_agent_id: Option<String>,
    pub kind: AgentTeamMessageKind,
    pub content: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceNote {
    pub evidence_id: String,
    pub source_agent_id: String,
    pub source_path: String,
    pub excerpt: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EvidenceLedger {
    pub notes: Vec<EvidenceNote>,
}

impl EvidenceLedger {
    pub fn add_note(
        &mut self,
        source_agent_id: impl Into<String>,
        source_path: impl Into<String>,
        excerpt: impl Into<String>,
    ) -> String {
        let source_agent_id = source_agent_id.into();
        let source_path = source_path.into();
        let excerpt = excerpt.into();
        let content_hash = stable_text_hash(&format!("{source_agent_id}:{source_path}:{excerpt}"));
        let evidence_id = format!("ev_{content_hash}");
        self.notes.push(EvidenceNote {
            evidence_id: evidence_id.clone(),
            source_agent_id,
            source_path,
            excerpt,
            content_hash,
        });
        evidence_id
    }

    pub fn contains(&self, evidence_id: &str) -> bool {
        self.notes
            .iter()
            .any(|note| note.evidence_id == evidence_id)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TeamBlackboard {
    pub hypotheses: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub open_questions: Vec<String>,
    pub candidate_outputs: Vec<String>,
    pub conflicts: Vec<String>,
    pub decisions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusDecision {
    pub decision_id: String,
    pub team_id: String,
    pub status: String,
    pub rationale: String,
    pub evidence_refs: Vec<String>,
}

pub fn validate_team_message(
    message: &AgentTeamMessage,
    ledger: &EvidenceLedger,
) -> Result<(), String> {
    if message.to_agent_id.is_some() {
        return Err("AgentTeams v1 forbids direct full-mesh agent messages".to_string());
    }
    if matches!(
        message.kind,
        AgentTeamMessageKind::EvidenceNote
            | AgentTeamMessageKind::ReproductionResult
            | AgentTeamMessageKind::DecisionRecord
            | AgentTeamMessageKind::SummaryForIntegrator
    ) && message.evidence_refs.is_empty()
    {
        return Err("message kind requires evidence_refs".to_string());
    }
    for evidence_ref in &message.evidence_refs {
        if !ledger.contains(evidence_ref) {
            return Err(format!("unknown evidence_ref {evidence_ref}"));
        }
    }
    Ok(())
}

pub fn validate_final_claims(
    evidence_refs: &[String],
    ledger: &EvidenceLedger,
) -> Result<(), String> {
    if evidence_refs.is_empty() {
        return Err("final claims require at least one evidence_ref".to_string());
    }
    for evidence_ref in evidence_refs {
        if !ledger.contains(evidence_ref) {
            return Err(format!(
                "final claim has unknown evidence_ref {evidence_ref}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_claim_requires_evidence() {
        let ledger = EvidenceLedger::default();
        assert!(validate_final_claims(&[], &ledger).is_err());
    }

    #[test]
    fn full_mesh_message_is_rejected() {
        let mut ledger = EvidenceLedger::default();
        let evidence = ledger.add_note("a", "README.md", "excerpt");
        let message = AgentTeamMessage {
            message_id: "m1".to_string(),
            team_id: "t1".to_string(),
            from_agent_id: "a".to_string(),
            to_agent_id: Some("b".to_string()),
            kind: AgentTeamMessageKind::EvidenceNote,
            content: "note".to_string(),
            evidence_refs: vec![evidence],
        };
        assert!(validate_team_message(&message, &ledger).is_err());
    }
}
