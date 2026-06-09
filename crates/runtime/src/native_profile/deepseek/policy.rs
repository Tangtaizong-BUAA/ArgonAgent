//! DeepSeek native runtime policies for reasoning and tool-call protocol choice.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningBudget {
    pub max_reasoning_tokens_per_turn: u64,
    pub max_total_reasoning_tokens: u64,
    pub auto_fold_threshold: u64,
}

impl Default for ReasoningBudget {
    fn default() -> Self {
        Self {
            max_reasoning_tokens_per_turn: 8_000,
            max_total_reasoning_tokens: 16_000,
            auto_fold_threshold: 12_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasoningFoldPolicy {
    KeepRawForAdjacentReplay { max_turns: u32 },
    SummarizeAfter { turns: u32, target_tokens: u64 },
    DropOlderThanSeconds { seconds: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasoningBudgetAction {
    Continue,
    FoldNow { reason: String },
    BlockReplay { reason: String },
}

pub fn evaluate_reasoning_budget(
    budget: &ReasoningBudget,
    current_turn_tokens: u64,
    total_reasoning_tokens: u64,
    required_for_adjacent_replay: bool,
) -> ReasoningBudgetAction {
    if required_for_adjacent_replay && total_reasoning_tokens > budget.max_total_reasoning_tokens {
        return ReasoningBudgetAction::BlockReplay {
            reason: "required reasoning replay exceeds total budget".to_string(),
        };
    }
    if current_turn_tokens > budget.max_reasoning_tokens_per_turn {
        return ReasoningBudgetAction::FoldNow {
            reason: "turn reasoning budget exceeded".to_string(),
        };
    }
    if total_reasoning_tokens > budget.auto_fold_threshold {
        return ReasoningBudgetAction::FoldNow {
            reason: "total reasoning fold threshold exceeded".to_string(),
        };
    }
    ReasoningBudgetAction::Continue
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallProtocol {
    NativeJson,
    Dsml,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallProtocolPolicy {
    pub prefer: ToolCallProtocol,
    pub allow_fallback: bool,
    pub dsml_fallback_rate_warn_threshold: f32,
    pub auto_disable_dsml_when_native_works: bool,
}

impl Default for ToolCallProtocolPolicy {
    fn default() -> Self {
        Self {
            prefer: ToolCallProtocol::NativeJson,
            allow_fallback: true,
            dsml_fallback_rate_warn_threshold: 0.2,
            auto_disable_dsml_when_native_works: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCallProtocolMetrics {
    pub native_successes: u32,
    pub dsml_fallbacks: u32,
    pub parser_repairs: u32,
    pub wrong_tool_preventions: u32,
}

impl ToolCallProtocolMetrics {
    pub fn total_tool_turns(&self) -> u32 {
        self.native_successes + self.dsml_fallbacks
    }

    pub fn dsml_fallback_rate(&self) -> f32 {
        let total = self.total_tool_turns();
        if total == 0 {
            0.0
        } else {
            self.dsml_fallbacks as f32 / total as f32
        }
    }

    pub fn should_warn_dsml_fallback(&self, policy: &ToolCallProtocolPolicy) -> bool {
        self.total_tool_turns() >= 3
            && self.dsml_fallback_rate() > policy.dsml_fallback_rate_warn_threshold
    }

    pub fn native_is_stable_enough_to_hide_dsml_guidance(&self) -> bool {
        self.native_successes >= 5 && self.dsml_fallbacks == 0 && self.parser_repairs == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_budget_folds_before_context_blowup() {
        let action = evaluate_reasoning_budget(&ReasoningBudget::default(), 2_000, 13_000, false);
        assert_eq!(
            action,
            ReasoningBudgetAction::FoldNow {
                reason: "total reasoning fold threshold exceeded".to_string()
            }
        );
    }

    #[test]
    fn required_reasoning_replay_blocks_when_budget_is_exhausted() {
        let action = evaluate_reasoning_budget(&ReasoningBudget::default(), 2_000, 20_000, true);
        assert_eq!(
            action,
            ReasoningBudgetAction::BlockReplay {
                reason: "required reasoning replay exceeds total budget".to_string()
            }
        );
    }

    #[test]
    fn dsml_fallback_rate_warns_after_threshold() {
        let metrics = ToolCallProtocolMetrics {
            native_successes: 2,
            dsml_fallbacks: 1,
            parser_repairs: 0,
            wrong_tool_preventions: 0,
        };
        assert!(metrics.should_warn_dsml_fallback(&ToolCallProtocolPolicy::default()));
    }

    #[test]
    fn stable_native_success_can_hide_dsml_guidance() {
        let metrics = ToolCallProtocolMetrics {
            native_successes: 5,
            dsml_fallbacks: 0,
            parser_repairs: 0,
            wrong_tool_preventions: 0,
        };
        assert!(metrics.native_is_stable_enough_to_hide_dsml_guidance());
    }
}
