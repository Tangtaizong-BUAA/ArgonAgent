use std::time::Instant;

use crate::tool_execution::ToolExecutionResult;

/// Classification of a tool result for plateau and convergence tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceClass {
    /// Genuinely new observation — distinct key added to ObservationCache.
    NewEvidence,
    /// Recovery result (synthetic error, alias, transport fixup).
    Recovery,
    /// Genuine execution error.
    Error,
    /// Deduplication-suppressed (not shown in continuation).
    Suppressed,
}

/// One tool result stored in the evidence ledger.
#[derive(Debug, Clone)]
pub struct EvidenceItem {
    pub provider_tool_call_id: String,
    pub tool_id: String,
    pub arguments_json: String,
    pub result: ToolExecutionResult,
    pub classification: EvidenceClass,
}

#[derive(Debug, Clone, Default)]
pub struct NoveltyScore {
    /// Count of NewEvidence-classified items in this iteration.
    pub new_evidence_count: u32,
    /// Count of Suppressed-classified items in this iteration.
    pub suppressed_count: u32,
    /// Count of Error-classified items in this iteration.
    pub error_count: u32,
    /// Count of Recovery-classified items in this iteration.
    pub recovery_count: u32,
}

/// An immutable snapshot of one iteration's evidence (sealed when next iteration begins).
#[derive(Debug, Clone)]
pub struct IterationEvidence {
    pub iter_index: u32,
    pub started_at: Instant,
    pub items: Vec<EvidenceItem>,
    pub novelty: NoveltyScore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryDigestEntry {
    pub iter_index: u32,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersededEntry {
    pub older_key: String,
    pub superseded_by: String,
}

/// The model-facing view of collected evidence.
///
/// Structured provider continuations must replay only the immediately previous
/// tool-use batch. Older iterations are summarized as text for plain evidence
/// continuations and diagnostics, never replayed as provider tool_result blocks.
#[derive(Debug, Clone, Default)]
pub struct ContinuationView {
    pub current_iteration_items: Vec<EvidenceItem>,
    pub history_digest: Vec<HistoryDigestEntry>,
    pub superseded: Vec<SupersededEntry>,
    pub suppressed_count: u32,
}

impl ContinuationView {
    pub fn from_legacy_batch(batch: Vec<(String, String, String, ToolExecutionResult)>) -> Self {
        let current_iteration_items = batch
            .into_iter()
            .map(|(provider_tool_call_id, tool_id, arguments_json, result)| {
                let classification = if result.ok {
                    EvidenceClass::NewEvidence
                } else {
                    EvidenceClass::Error
                };
                EvidenceItem {
                    provider_tool_call_id,
                    tool_id,
                    arguments_json,
                    result,
                    classification,
                }
            })
            .collect();
        Self {
            current_iteration_items,
            history_digest: Vec::new(),
            superseded: Vec::new(),
            suppressed_count: 0,
        }
    }

    pub fn current_legacy_batch(&self) -> Vec<(String, String, String, ToolExecutionResult)> {
        self.current_iteration_items
            .iter()
            .filter(|item| item.classification != EvidenceClass::Suppressed)
            .map(|item| {
                (
                    item.provider_tool_call_id.clone(),
                    item.tool_id.clone(),
                    item.arguments_json.clone(),
                    item.result.clone(),
                )
            })
            .collect()
    }

    pub fn current_len(&self) -> usize {
        self.current_iteration_items
            .iter()
            .filter(|item| item.classification != EvidenceClass::Suppressed)
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.current_len() == 0
    }

    pub fn history_digest_text(&self) -> String {
        self.history_digest
            .iter()
            .map(|entry| format!("- iter {}: {}", entry.iter_index, entry.summary))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Per-turn evidence accumulator.  Every iteration produces an `IterationEvidence`
/// snapshot; the ledger keeps the *last* iteration's snapshot as the canonical
/// "what to send back to the model" view, plus a compacted history for in-turn
/// cross-iteration reasoning.
#[derive(Debug, Clone)]
pub struct EvidenceLedger {
    iterations: Vec<IterationEvidence>,
    /// The current (open) iteration's items.
    current_items: Vec<EvidenceItem>,
    current_iter_index: u32,
    current_started_at: Instant,
    /// Running suppression count for the current iteration.
    current_suppressed_count: u32,
    /// Running error count for the current iteration.
    current_error_count: u32,
    /// Running recovery count for the current iteration.
    current_recovery_count: u32,
    /// Running new-evidence count for the current iteration.
    current_new_evidence_count: u32,
}

impl Default for EvidenceLedger {
    fn default() -> Self {
        Self {
            iterations: Vec::new(),
            current_items: Vec::new(),
            current_iter_index: 0,
            current_started_at: Instant::now(),
            current_suppressed_count: 0,
            current_error_count: 0,
            current_recovery_count: 0,
            current_new_evidence_count: 0,
        }
    }
}

impl EvidenceLedger {
    /// Begin a new iteration, sealing the previous one.
    pub fn begin_iteration(&mut self, iter_index: u32) {
        if !self.current_items.is_empty() || self.current_suppressed_count > 0 {
            let sealed = IterationEvidence {
                iter_index: self.current_iter_index,
                started_at: self.current_started_at,
                items: std::mem::take(&mut self.current_items),
                novelty: NoveltyScore {
                    new_evidence_count: self.current_new_evidence_count,
                    suppressed_count: self.current_suppressed_count,
                    error_count: self.current_error_count,
                    recovery_count: self.current_recovery_count,
                },
            };
            self.iterations.push(sealed);
        }
        self.current_iter_index = iter_index;
        self.current_started_at = Instant::now();
        self.current_suppressed_count = 0;
        self.current_error_count = 0;
        self.current_recovery_count = 0;
        self.current_new_evidence_count = 0;
        self.current_items.clear();
    }

    /// Backward-compatible push matching the old `last_tool_batch` 4-tuple signature.
    /// Automatically classifies: ok → NewEvidence, !ok → Error.
    pub fn push_legacy(&mut self, item: (String, String, String, ToolExecutionResult)) {
        let (provider_tool_call_id, tool_id, arguments_json, result) = item;
        let class = if !result.ok {
            EvidenceClass::Error
        } else {
            EvidenceClass::NewEvidence
        };
        self.push(
            provider_tool_call_id,
            tool_id,
            arguments_json,
            result,
            class,
        );
    }

    /// Push a result with explicit classification.
    pub fn push(
        &mut self,
        provider_tool_call_id: String,
        tool_id: String,
        arguments_json: String,
        result: ToolExecutionResult,
        classification: EvidenceClass,
    ) {
        match classification {
            EvidenceClass::NewEvidence => self.current_new_evidence_count += 1,
            EvidenceClass::Suppressed => self.current_suppressed_count += 1,
            EvidenceClass::Error => self.current_error_count += 1,
            EvidenceClass::Recovery => self.current_recovery_count += 1,
        }
        if classification == EvidenceClass::Suppressed {
            return;
        }
        self.current_items.push(EvidenceItem {
            provider_tool_call_id,
            tool_id,
            arguments_json,
            result,
            classification,
        });
    }

    /// Record a suppressed observation (not shown in continuation view).
    pub fn record_suppressed(&mut self) {
        self.current_suppressed_count += 1;
    }

    /// Clear current iteration items (matches old `last_tool_batch.clear()`).
    pub fn clear(&mut self) {
        self.current_items.clear();
        self.current_suppressed_count = 0;
        self.current_error_count = 0;
        self.current_recovery_count = 0;
        self.current_new_evidence_count = 0;
    }

    /// True if the current iteration has no items.
    pub fn is_empty(&self) -> bool {
        self.current_items.is_empty() && self.current_suppressed_count == 0
    }

    /// Number of model-visible items in the current iteration.
    pub fn len(&self) -> usize {
        self.current_items.len()
    }

    /// Number of suppressed results in the current iteration.
    pub fn suppressed_count(&self) -> u32 {
        self.current_suppressed_count
    }

    /// Number of new-evidence results in the current iteration.
    pub fn new_evidence_count(&self) -> u32 {
        self.current_new_evidence_count
    }

    /// Number of error results in the current iteration.
    pub fn error_count(&self) -> u32 {
        self.current_error_count
    }

    pub fn recovery_count(&self) -> u32 {
        self.current_recovery_count
    }

    pub fn current_total_count(&self) -> u32 {
        self.current_new_evidence_count
            + self.current_suppressed_count
            + self.current_error_count
            + self.current_recovery_count
    }

    /// Clone the current items into the legacy 4-tuple Vec format for continuation.
    pub fn to_legacy_batch(&self) -> Vec<(String, String, String, ToolExecutionResult)> {
        self.current_items
            .iter()
            .filter(|item| item.classification != EvidenceClass::Suppressed)
            .map(|item| {
                (
                    item.provider_tool_call_id.clone(),
                    item.tool_id.clone(),
                    item.arguments_json.clone(),
                    item.result.clone(),
                )
            })
            .collect()
    }

    /// Legacy iterator: yield only non-suppressed items in the 4-tuple format.
    pub fn iter_legacy(&self) -> impl Iterator<Item = (&str, &str, &str, &ToolExecutionResult)> {
        self.current_items
            .iter()
            .filter(|item| item.classification != EvidenceClass::Suppressed)
            .map(|item| {
                (
                    item.provider_tool_call_id.as_str(),
                    item.tool_id.as_str(),
                    item.arguments_json.as_str(),
                    &item.result,
                )
            })
    }

    /// Consecutive iterations with no new evidence.
    pub fn no_new_evidence_streak(&self) -> u32 {
        let mut streak = 0u32;
        if self.current_total_count() > 0 && self.current_new_evidence_count == 0 {
            streak += 1;
        } else if self.current_new_evidence_count > 0 {
            return 0;
        }
        for iter in self.iterations.iter().rev() {
            if iter.novelty.new_evidence_count == 0 {
                streak += 1;
            } else {
                break;
            }
        }
        streak
    }

    /// Consecutive iterations dominated by duplicate/suppressed results.
    pub fn duplicate_dominated_streak(&self) -> u32 {
        let mut streak = 0u32;
        let current_total = self.current_total_count();
        if current_total > 0 && self.current_suppressed_count as f32 / current_total as f32 >= 0.7 {
            streak += 1;
        } else if current_total > 0 {
            return 0;
        }
        for iter in self.iterations.iter().rev() {
            let total = iter.novelty.new_evidence_count
                + iter.novelty.suppressed_count
                + iter.novelty.error_count
                + iter.novelty.recovery_count;
            if total > 0 && iter.novelty.suppressed_count as f32 / total as f32 >= 0.7 {
                streak += 1;
            } else {
                break;
            }
        }
        streak
    }

    /// Replace all current items from a legacy Vec (used for `last_tool_batch = ...` assignments).
    pub fn replace_from_legacy(
        &mut self,
        items: Vec<(String, String, String, ToolExecutionResult)>,
    ) {
        self.current_items.clear();
        self.current_suppressed_count = 0;
        self.current_error_count = 0;
        self.current_recovery_count = 0;
        self.current_new_evidence_count = 0;
        for (pid, tid, args, result) in items {
            let class = if !result.ok {
                EvidenceClass::Error
            } else {
                EvidenceClass::NewEvidence
            };
            match class {
                EvidenceClass::NewEvidence => self.current_new_evidence_count += 1,
                EvidenceClass::Error => self.current_error_count += 1,
                _ => {}
            }
            self.current_items.push(EvidenceItem {
                provider_tool_call_id: pid,
                tool_id: tid,
                arguments_json: args,
                result,
                classification: class,
            });
        }
    }

    /// Build a continuation summary for the model.
    pub fn continuation_summary(&self) -> Option<String> {
        if self.current_suppressed_count == 0 {
            return None;
        }
        Some(format!(
            "{} duplicate observation(s) suppressed this iteration — avoid re-reading already-observed files/ranges; use collected evidence to produce the answer.",
            self.current_suppressed_count
        ))
    }

    pub fn view_for_continuation(&self) -> ContinuationView {
        let current_iteration_items = self.current_items.clone();
        let current_iter_index = Some(self.current_iter_index);
        let history_digest = self
            .iterations
            .iter()
            .filter(|iteration| Some(iteration.iter_index) != current_iter_index)
            .rev()
            .take(8)
            .map(|iteration| HistoryDigestEntry {
                iter_index: iteration.iter_index,
                summary: summarize_iteration(iteration),
            })
            .collect::<Vec<_>>();
        let suppressed_count = self.current_suppressed_count
            + self
                .iterations
                .iter()
                .map(|iteration| iteration.novelty.suppressed_count)
                .sum::<u32>();
        ContinuationView {
            current_iteration_items,
            history_digest,
            superseded: Vec::new(),
            suppressed_count,
        }
    }
}

fn summarize_iteration(iteration: &IterationEvidence) -> String {
    let mut parts = iteration
        .items
        .iter()
        .take(4)
        .map(|item| {
            format!(
                "{} ok={} args={} preview={}",
                item.tool_id,
                item.result.ok,
                compact_ledger_inline(&item.arguments_json, 120),
                compact_ledger_inline(&item.result.preview, 160)
            )
        })
        .collect::<Vec<_>>();
    if iteration.items.len() > parts.len() {
        parts.push(format!("+{} more", iteration.items.len() - parts.len()));
    }
    if iteration.novelty.suppressed_count > 0 {
        parts.push(format!(
            "{} duplicate suppressed",
            iteration.novelty.suppressed_count
        ));
    }
    if parts.is_empty() {
        "no model-visible tool evidence".to_string()
    } else {
        parts.join("; ")
    }
}

fn compact_ledger_inline(value: &str, limit: usize) -> String {
    let cleaned = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= limit {
        return cleaned;
    }
    let mut out = cleaned
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_result(tool_id: &str, preview: &str) -> ToolExecutionResult {
        ToolExecutionResult {
            tool_call_id: format!("{tool_id}_call"),
            tool_id: tool_id.to_string(),
            ok: true,
            preview: preview.to_string(),
            detail_json: "{\"ok\":true}".to_string(),
            exit_code: None,
        }
    }

    #[test]
    fn suppressed_evidence_does_not_enter_continuation_view() {
        let mut ledger = EvidenceLedger::default();
        ledger.begin_iteration(0);
        ledger.push(
            "toolu_0".to_string(),
            "file.read".to_string(),
            "{\"path\":\"README.md\"}".to_string(),
            ok_result("file.read", "read README"),
            EvidenceClass::NewEvidence,
        );
        ledger.begin_iteration(1);
        ledger.record_suppressed();

        let view = ledger.view_for_continuation();
        assert!(view.current_iteration_items.is_empty());
        assert_eq!(view.suppressed_count, 1);
        assert_eq!(view.history_digest.len(), 1);
        assert!(view.history_digest_text().contains("read README"));
    }

    #[test]
    fn history_digest_keeps_only_compact_recent_entries() {
        let mut ledger = EvidenceLedger::default();
        for iter in 0..12u32 {
            ledger.begin_iteration(iter);
            ledger.push(
                format!("toolu_{iter}"),
                "file.read".to_string(),
                format!("{{\"path\":\"file_{iter}.rs\",\"max_bytes\":8000}}"),
                ok_result("file.read", &format!("read file_{iter}")),
                EvidenceClass::NewEvidence,
            );
        }
        ledger.begin_iteration(12);
        ledger.record_suppressed();

        let view = ledger.view_for_continuation();
        let digest = view.history_digest_text();
        assert!(digest.len() < 4_000);
        assert_eq!(view.history_digest.len(), 8);
        assert!(digest.contains("file_11"));
        assert!(!digest.contains("file_0"));
    }
}
