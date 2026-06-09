use super::EvidenceLedger;

#[derive(Debug, Clone, PartialEq)]
pub enum ConvergenceVerdict {
    Continue,
    BatchNoveltyPlateau {
        novelty_ratio: f32,
        threshold: f32,
        window: u32,
    },
    DuplicateDominance {
        ratio: f32,
        window: u32,
    },
    InformationStagnation {
        distinct_keys_growth: u32,
        window: u32,
    },
    BudgetExhausted,
}

#[derive(Debug, Clone)]
pub struct ConvergenceEnforcer {
    pub duplicate_ratio_threshold: f32,
    pub duplicate_window: u32,
    pub no_new_evidence_window: u32,
    pub batch_window: usize,
}

impl Default for ConvergenceEnforcer {
    fn default() -> Self {
        Self {
            duplicate_ratio_threshold: 0.7,
            duplicate_window: 2,
            no_new_evidence_window: 3,
            batch_window: 6,
        }
    }
}

impl ConvergenceEnforcer {
    pub fn observe_iteration(
        &self,
        ledger: &EvidenceLedger,
        current_batch_signature: &str,
        recent_signatures: &[String],
        distinct_keys_growth: u32,
    ) -> ConvergenceVerdict {
        let total = ledger.current_total_count();
        if total > 0 {
            let duplicate_ratio = ledger.suppressed_count() as f32 / total as f32;
            if duplicate_ratio >= self.duplicate_ratio_threshold
                && ledger.duplicate_dominated_streak() >= self.duplicate_window
            {
                return ConvergenceVerdict::DuplicateDominance {
                    ratio: duplicate_ratio,
                    window: self.duplicate_window,
                };
            }

            if distinct_keys_growth == 0
                && ledger.no_new_evidence_streak() >= self.no_new_evidence_window
            {
                return ConvergenceVerdict::InformationStagnation {
                    distinct_keys_growth,
                    window: self.no_new_evidence_window,
                };
            }
        }

        if !current_batch_signature.is_empty()
            && recent_signatures
                .iter()
                .rev()
                .take(self.batch_window)
                .filter(|signature| signature.as_str() == current_batch_signature)
                .count()
                >= 2
        {
            return ConvergenceVerdict::BatchNoveltyPlateau {
                novelty_ratio: 0.0,
                threshold: 0.05,
                window: self.batch_window as u32,
            };
        }

        ConvergenceVerdict::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_kernel::EvidenceClass;
    use crate::tool_execution::ToolExecutionResult;

    fn ok_result() -> ToolExecutionResult {
        ToolExecutionResult {
            tool_call_id: "tool".to_string(),
            tool_id: "file.read".to_string(),
            ok: true,
            preview: "ok".to_string(),
            detail_json: "{}".to_string(),
            exit_code: None,
        }
    }

    #[test]
    fn duplicate_dominance_fires_after_two_dominated_iterations() {
        let enforcer = ConvergenceEnforcer::default();
        let mut ledger = EvidenceLedger::default();
        ledger.begin_iteration(0);
        ledger.record_suppressed();
        ledger.begin_iteration(1);
        ledger.record_suppressed();

        let verdict = enforcer.observe_iteration(&ledger, "sig", &[], 0);
        assert!(matches!(
            verdict,
            ConvergenceVerdict::DuplicateDominance { .. }
        ));
    }

    #[test]
    fn new_evidence_prevents_duplicate_dominance() {
        let enforcer = ConvergenceEnforcer::default();
        let mut ledger = EvidenceLedger::default();
        ledger.begin_iteration(0);
        ledger.record_suppressed();
        ledger.begin_iteration(1);
        ledger.push(
            "toolu".to_string(),
            "file.read".to_string(),
            "{}".to_string(),
            ok_result(),
            EvidenceClass::NewEvidence,
        );

        let verdict = enforcer.observe_iteration(&ledger, "sig", &[], 1);
        assert_eq!(verdict, ConvergenceVerdict::Continue);
    }
}
