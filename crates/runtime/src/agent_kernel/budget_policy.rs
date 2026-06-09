use super::turn_state::{TurnBudget, TurnRoute};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetPolicy {
    pub default_budget: TurnBudget,
}

impl Default for BudgetPolicy {
    fn default() -> Self {
        Self {
            default_budget: TurnBudget::default(),
        }
    }
}

impl BudgetPolicy {
    pub fn for_route(&self, route: &TurnRoute) -> TurnBudget {
        let mut budget = self.default_budget.clone();
        match route {
            TurnRoute::DirectAnswer => {
                budget.max_iterations = 2;
                budget.max_tool_calls = 0;
                budget.max_input_tokens = 96_000;
                budget.max_reasoning_tokens = 16_000;
            }
            TurnRoute::ReadOnlyExplore | TurnRoute::ProjectStatus => {
                budget.max_iterations = 5;
                budget.max_tool_calls = 12;
                budget.max_input_tokens = 160_000;
            }
            TurnRoute::CodeEdit | TurnRoute::DebugFailure | TurnRoute::RunTests => {
                budget.max_iterations = 10;
                budget.max_tool_calls = 48;
                budget.max_input_tokens = 224_000;
                budget.max_reasoning_tokens = 96_000;
            }
            TurnRoute::LongHorizonTask => {
                budget.max_iterations = 16;
                budget.max_tool_calls = 96;
                budget.max_input_tokens = 320_000;
                budget.max_reasoning_tokens = 128_000;
            }
            TurnRoute::Review => {
                budget.max_iterations = 6;
                budget.max_tool_calls = 24;
                budget.max_input_tokens = 192_000;
                budget.max_reasoning_tokens = 48_000;
            }
        }
        budget
    }

    pub fn should_compact(&self, tokens_in: u64) -> bool {
        tokens_in > self.default_budget.max_input_tokens
    }

    pub fn should_compact_for_route(&self, route: &TurnRoute, tokens_in: u64) -> bool {
        tokens_in > self.for_route(route).max_input_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_threshold_is_deepseek_shaped() {
        let policy = BudgetPolicy::default();
        assert!(!policy.should_compact(192_000));
        assert!(policy.should_compact(192_001));
    }

    #[test]
    fn budgets_are_route_aware() {
        let policy = BudgetPolicy::default();
        assert!(
            policy
                .for_route(&TurnRoute::LongHorizonTask)
                .max_input_tokens
                > policy
                    .for_route(&TurnRoute::ReadOnlyExplore)
                    .max_input_tokens
        );
        assert!(
            policy.for_route(&TurnRoute::CodeEdit).max_tool_calls
                > policy.for_route(&TurnRoute::DirectAnswer).max_tool_calls
        );
    }
}
