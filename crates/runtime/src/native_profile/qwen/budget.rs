use crate::context_budget::{CacheStabilityPolicy, ContextBudget, ScaffoldLevel};
use crate::model_adapter::ModelRole;
use researchcode_kernel::model::NativeModelFamily;

pub fn qwen_fast_budget(deployment_context_tokens: Option<u64>) -> ContextBudget {
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

pub fn qwen_guarded_budget(deployment_context_tokens: Option<u64>) -> ContextBudget {
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

pub fn qwen_scaffold_level_for_role(role: &ModelRole) -> ScaffoldLevel {
    match role {
        ModelRole::Executor | ModelRole::Summarizer => ScaffoldLevel::QwenFast,
        ModelRole::Planner | ModelRole::Reviewer | ModelRole::Researcher => {
            ScaffoldLevel::QwenGuarded
        }
    }
}
