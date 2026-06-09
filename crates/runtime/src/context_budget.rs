//! Context budget and scaffold policy for native DeepSeek/Qwen modes.
//!
//! This module encodes the architecture decision that runtime scaffold stays
//! strong, while prompt/context scaffold is model- and mode-specific. It does
//! not assemble prompts; it gives prompt/context builders a testable budget.

use crate::agent_kernel::TurnBudget;
use crate::model_adapter::ModelRole;
use researchcode_kernel::model::NativeModelFamily;

pub const DEEPSEEK_EFFECTIVE_CONTEXT_LIMIT_TOKENS: u64 = 256_000;
pub const DEEPSEEK_TARGET_MODEL_CALL_TOKENS: u64 = 240_000;
pub const DEEPSEEK_COMPACTION_THRESHOLD_TOKENS: u64 = 192_000;
pub const DEEPSEEK_COMPACTION_FLOOR_TOKENS: u64 = 24_000;
pub const DEEPSEEK_REASONING_REPLAY_BUDGET_TOKENS: u64 = 12_000;

pub fn deepseek_compaction_threshold_tokens(max_context_tokens: u64) -> u64 {
    DEEPSEEK_COMPACTION_THRESHOLD_TOKENS.min(max_context_tokens * 3 / 4)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScaffoldLevel {
    CompatibleMinimal,
    QwenFast,
    QwenGuarded,
    DeepSeekFull,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheStabilityPolicy {
    StablePrefix,
    DynamicPrefix,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextBudget {
    pub model_id: String,
    pub model_family: NativeModelFamily,
    pub scaffold_level: ScaffoldLevel,
    pub max_context_tokens: u64,
    pub output_reserve_tokens: u64,
    pub emergency_reserve_tokens: u64,
    pub static_prompt_budget: u64,
    pub model_profile_budget: u64,
    pub tool_schema_budget: u64,
    pub task_contract_budget: u64,
    pub repo_map_budget: u64,
    pub file_snippet_budget: u64,
    pub tool_output_budget: u64,
    pub memory_budget: u64,
    pub reasoning_replay_budget: u64,
    pub research_data_budget: u64,
    pub compaction_threshold: u64,
    pub compaction_floor: u64,
    pub min_retrieval_budget: u64,
    pub max_active_tools: usize,
    pub max_files_per_turn: usize,
    pub max_tool_output_per_turn: u64,
    pub cache_stability_policy: CacheStabilityPolicy,
}

impl ContextBudget {
    pub fn prompt_scaffold_tokens(&self) -> u64 {
        self.static_prompt_budget + self.model_profile_budget + self.tool_schema_budget
    }

    pub fn protected_reserve_tokens(&self) -> u64 {
        self.output_reserve_tokens + self.emergency_reserve_tokens
    }

    pub fn dynamic_context_tokens(&self) -> u64 {
        self.repo_map_budget
            + self.file_snippet_budget
            + self.tool_output_budget
            + self.memory_budget
            + self.reasoning_replay_budget
            + self.research_data_budget
    }

    pub fn to_summary_line(&self) -> String {
        format!(
            "context budget model={} level={:?} max={} scaffold={} dynamic={} reserve={} tools={} files={}",
            self.model_id,
            self.scaffold_level,
            self.max_context_tokens,
            self.prompt_scaffold_tokens(),
            self.dynamic_context_tokens(),
            self.protected_reserve_tokens(),
            self.max_active_tools,
            self.max_files_per_turn
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextBudgetValidation {
    pub ok: bool,
    pub errors: Vec<String>,
}

impl ContextBudgetValidation {
    pub fn to_summary_line(&self) -> String {
        format!(
            "context budget validation ok={} errors={}",
            self.ok,
            self.errors.len()
        )
    }
}

pub fn allocate_native_context_budget(
    family: NativeModelFamily,
    role: ModelRole,
    deployment_context_tokens: Option<u64>,
) -> ContextBudget {
    match family {
        NativeModelFamily::DeepSeek => deepseek_full_budget(deployment_context_tokens),
        NativeModelFamily::Qwen => match qwen_scaffold_level_for_role(&role) {
            ScaffoldLevel::QwenFast => qwen_fast_budget(deployment_context_tokens),
            ScaffoldLevel::QwenGuarded => qwen_guarded_budget(deployment_context_tokens),
            _ => unreachable!("Qwen role routing must use Qwen scaffold levels"),
        },
    }
}

pub fn allocate_native_context_budget_for_turn(
    family: NativeModelFamily,
    role: ModelRole,
    deployment_context_tokens: Option<u64>,
    turn_budget: &TurnBudget,
) -> ContextBudget {
    let mut budget = allocate_native_context_budget(family, role, deployment_context_tokens);
    if turn_budget.max_input_tokens > 0 {
        budget.compaction_threshold = turn_budget
            .max_input_tokens
            .min(budget.max_context_tokens.saturating_sub(1));
        budget.compaction_floor = budget
            .compaction_floor
            .min(budget.compaction_threshold.saturating_sub(1));
    }
    if turn_budget.max_output_tokens > 0 {
        budget.output_reserve_tokens = budget
            .output_reserve_tokens
            .min(turn_budget.max_output_tokens);
    }
    if turn_budget.max_reasoning_tokens > 0 {
        budget.reasoning_replay_budget = budget
            .reasoning_replay_budget
            .min(turn_budget.max_reasoning_tokens);
    }
    budget
}

pub fn validate_context_budget(budget: &ContextBudget) -> ContextBudgetValidation {
    let mut errors = Vec::new();
    let total_named = budget.prompt_scaffold_tokens()
        + budget.task_contract_budget
        + budget.dynamic_context_tokens()
        + budget.protected_reserve_tokens();
    if total_named > budget.max_context_tokens {
        errors.push(format!(
            "named budget exceeds max context: {} > {}",
            total_named, budget.max_context_tokens
        ));
    }
    if budget.compaction_floor >= budget.compaction_threshold {
        errors.push("compaction_floor must be below compaction_threshold".to_string());
    }
    if budget.compaction_threshold >= budget.max_context_tokens {
        errors.push("compaction_threshold must be below max_context_tokens".to_string());
    }
    if budget.output_reserve_tokens == 0 || budget.emergency_reserve_tokens == 0 {
        errors.push("output and emergency reserves are mandatory".to_string());
    }
    if matches!(
        budget.scaffold_level,
        ScaffoldLevel::QwenFast | ScaffoldLevel::QwenGuarded
    ) && budget.prompt_scaffold_tokens() > budget.max_context_tokens / 10
    {
        errors.push("Qwen prompt scaffold must stay below 10% of context".to_string());
    }
    if matches!(budget.scaffold_level, ScaffoldLevel::QwenFast) && budget.max_active_tools > 5 {
        errors.push("Qwen fast mode must keep active tools narrow".to_string());
    }
    if matches!(budget.scaffold_level, ScaffoldLevel::DeepSeekFull)
        && budget.reasoning_replay_budget == 0
    {
        errors.push("DeepSeek full mode must reserve reasoning replay budget".to_string());
    }
    ContextBudgetValidation {
        ok: errors.is_empty(),
        errors,
    }
}

fn deepseek_full_budget(deployment_context_tokens: Option<u64>) -> ContextBudget {
    let max = deployment_context_tokens
        .unwrap_or(DEEPSEEK_EFFECTIVE_CONTEXT_LIMIT_TOKENS)
        .clamp(64_000, DEEPSEEK_EFFECTIVE_CONTEXT_LIMIT_TOKENS);
    ContextBudget {
        model_id: "deepseek-v4-native".to_string(),
        model_family: NativeModelFamily::DeepSeek,
        scaffold_level: ScaffoldLevel::DeepSeekFull,
        max_context_tokens: max,
        output_reserve_tokens: deepseek_scale(max, 16_000),
        emergency_reserve_tokens: deepseek_scale(max, 16_000),
        static_prompt_budget: deepseek_scale(max, 8_000),
        model_profile_budget: deepseek_scale(max, 3_000),
        tool_schema_budget: deepseek_scale(max, 6_000),
        task_contract_budget: deepseek_scale(max, 4_000),
        repo_map_budget: deepseek_scale(max, 20_000),
        file_snippet_budget: deepseek_scale(max, 70_000),
        tool_output_budget: deepseek_scale(max, 24_000),
        memory_budget: deepseek_scale(max, 12_000),
        reasoning_replay_budget: deepseek_scale(max, DEEPSEEK_REASONING_REPLAY_BUDGET_TOKENS),
        research_data_budget: deepseek_scale(max, 8_000),
        compaction_threshold: deepseek_compaction_threshold_tokens(max),
        compaction_floor: deepseek_scale(max, DEEPSEEK_COMPACTION_FLOOR_TOKENS),
        min_retrieval_budget: max * 31 / 100,
        max_active_tools: 8,
        max_files_per_turn: 24,
        max_tool_output_per_turn: deepseek_scale(max, 24_000),
        cache_stability_policy: CacheStabilityPolicy::StablePrefix,
    }
}

fn qwen_fast_budget(deployment_context_tokens: Option<u64>) -> ContextBudget {
    let max = deployment_context_tokens.unwrap_or(262_000).max(64_000);
    let degraded = max <= 128_000;
    ContextBudget {
        model_id: "qwen3-6-27b-native".to_string(),
        model_family: NativeModelFamily::Qwen,
        scaffold_level: ScaffoldLevel::QwenFast,
        max_context_tokens: max,
        output_reserve_tokens: if degraded { 12_000 } else { 18_000 },
        emergency_reserve_tokens: if degraded { 18_000 } else { 30_000 },
        static_prompt_budget: if degraded { 4_500 } else { 7_000 },
        model_profile_budget: if degraded { 1_500 } else { 2_500 },
        tool_schema_budget: if degraded { 3_500 } else { 4_500 },
        task_contract_budget: if degraded { 2_000 } else { 3_000 },
        repo_map_budget: if degraded { 9_000 } else { 16_000 },
        file_snippet_budget: if degraded { 46_000 } else { 95_000 },
        tool_output_budget: if degraded { 10_000 } else { 24_000 },
        memory_budget: if degraded { 5_000 } else { 12_000 },
        reasoning_replay_budget: 0,
        research_data_budget: if degraded { 4_000 } else { 12_000 },
        compaction_threshold: max * 80 / 100,
        compaction_floor: max * 62 / 100,
        min_retrieval_budget: max * 45 / 100,
        max_active_tools: 5,
        max_files_per_turn: if degraded { 8 } else { 14 },
        max_tool_output_per_turn: if degraded { 12_000 } else { 28_000 },
        cache_stability_policy: CacheStabilityPolicy::StablePrefix,
    }
}

fn qwen_guarded_budget(deployment_context_tokens: Option<u64>) -> ContextBudget {
    let max = deployment_context_tokens.unwrap_or(262_000).max(64_000);
    let degraded = max <= 128_000;
    ContextBudget {
        model_id: "qwen3-6-27b-native".to_string(),
        model_family: NativeModelFamily::Qwen,
        scaffold_level: ScaffoldLevel::QwenGuarded,
        max_context_tokens: max,
        output_reserve_tokens: if degraded { 10_000 } else { 20_000 },
        emergency_reserve_tokens: if degraded { 15_000 } else { 28_000 },
        static_prompt_budget: if degraded { 4_000 } else { 8_000 },
        model_profile_budget: if degraded { 1_500 } else { 3_000 },
        tool_schema_budget: if degraded { 3_000 } else { 6_000 },
        task_contract_budget: if degraded { 2_000 } else { 4_000 },
        repo_map_budget: if degraded { 8_000 } else { 20_000 },
        file_snippet_budget: if degraded { 40_000 } else { 90_000 },
        tool_output_budget: if degraded { 8_000 } else { 24_000 },
        memory_budget: if degraded { 5_000 } else { 14_000 },
        reasoning_replay_budget: 0,
        research_data_budget: if degraded { 4_000 } else { 8_000 },
        compaction_threshold: max * 80 / 100,
        compaction_floor: max * 60 / 100,
        min_retrieval_budget: max * 42 / 100,
        max_active_tools: 7,
        max_files_per_turn: if degraded { 12 } else { 22 },
        max_tool_output_per_turn: if degraded { 18_000 } else { 40_000 },
        cache_stability_policy: CacheStabilityPolicy::StablePrefix,
    }
}

fn qwen_scaffold_level_for_role(role: &ModelRole) -> ScaffoldLevel {
    match role {
        ModelRole::Executor | ModelRole::Summarizer => ScaffoldLevel::QwenFast,
        ModelRole::Planner | ModelRole::Reviewer | ModelRole::Researcher => {
            ScaffoldLevel::QwenGuarded
        }
    }
}

fn deepseek_scale(max: u64, value_at_256k: u64) -> u64 {
    (max * value_at_256k / DEEPSEEK_EFFECTIVE_CONTEXT_LIMIT_TOKENS).max(1_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_full_budget_clamps_to_safe_256k_context() {
        let budget =
            allocate_native_context_budget(NativeModelFamily::DeepSeek, ModelRole::Planner, None);
        assert_eq!(budget.scaffold_level, ScaffoldLevel::DeepSeekFull);
        assert_eq!(
            budget.max_context_tokens,
            DEEPSEEK_EFFECTIVE_CONTEXT_LIMIT_TOKENS
        );
        assert_eq!(
            budget.compaction_threshold,
            DEEPSEEK_COMPACTION_THRESHOLD_TOKENS
        );
        assert_eq!(
            budget.reasoning_replay_budget,
            DEEPSEEK_REASONING_REPLAY_BUDGET_TOKENS
        );
        assert!(budget.prompt_scaffold_tokens() <= 20_000);
        assert!(budget.max_files_per_turn <= 24);
        assert!(matches!(
            budget.cache_stability_policy,
            CacheStabilityPolicy::StablePrefix
        ));
        assert!(validate_context_budget(&budget).ok);
    }

    #[test]
    fn deepseek_declared_1m_deployment_still_clamps_to_256k() {
        let budget = allocate_native_context_budget(
            NativeModelFamily::DeepSeek,
            ModelRole::Executor,
            Some(1_000_000),
        );
        assert_eq!(
            budget.max_context_tokens,
            DEEPSEEK_EFFECTIVE_CONTEXT_LIMIT_TOKENS
        );
        assert!(validate_context_budget(&budget).ok);
    }

    #[test]
    fn turn_budget_shapes_native_context_budget() {
        let turn_budget = TurnBudget {
            max_iterations: 8,
            max_tool_calls: 32,
            max_input_tokens: 192_000,
            max_output_tokens: 8_000,
            max_reasoning_tokens: 4_000,
        };
        let budget = allocate_native_context_budget_for_turn(
            NativeModelFamily::DeepSeek,
            ModelRole::Executor,
            None,
            &turn_budget,
        );
        assert_eq!(
            budget.max_context_tokens,
            DEEPSEEK_EFFECTIVE_CONTEXT_LIMIT_TOKENS
        );
        assert_eq!(budget.compaction_threshold, 192_000);
        assert_eq!(budget.output_reserve_tokens, 8_000);
        assert_eq!(budget.reasoning_replay_budget, 4_000);
        assert!(budget.compaction_threshold < budget.max_context_tokens);
        assert!(validate_context_budget(&budget).ok);
    }

    #[test]
    fn qwen_executor_uses_fast_thin_scaffold() {
        let budget =
            allocate_native_context_budget(NativeModelFamily::Qwen, ModelRole::Executor, None);
        assert_eq!(budget.scaffold_level, ScaffoldLevel::QwenFast);
        assert_eq!(budget.max_context_tokens, 262_000);
        assert!(budget.prompt_scaffold_tokens() < budget.max_context_tokens / 10);
        assert!(budget.max_active_tools <= 5);
        assert_eq!(budget.reasoning_replay_budget, 0);
        assert!(validate_context_budget(&budget).ok);
    }

    #[test]
    fn qwen_planner_uses_guarded_mode_and_degrades_to_128k() {
        let budget = allocate_native_context_budget(
            NativeModelFamily::Qwen,
            ModelRole::Planner,
            Some(128_000),
        );
        assert_eq!(budget.scaffold_level, ScaffoldLevel::QwenGuarded);
        assert_eq!(budget.max_context_tokens, 128_000);
        assert!(budget.file_snippet_budget <= 60_000);
        assert!(budget.output_reserve_tokens >= 10_000);
        assert!(validate_context_budget(&budget).ok);
    }

    #[test]
    fn validation_rejects_qwen_prompt_scaffold_bloat() {
        let mut budget =
            allocate_native_context_budget(NativeModelFamily::Qwen, ModelRole::Executor, None);
        budget.static_prompt_budget = 40_000;
        assert!(!validate_context_budget(&budget).ok);
    }
}
