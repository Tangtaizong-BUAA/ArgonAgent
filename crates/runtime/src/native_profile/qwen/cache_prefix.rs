use crate::model_adapter::{ModelRole, PlannedModelCall};
use crate::native_profile::deepseek::cache_prefix::{CachePrefixPolicy, ThreeZonePrompt};

pub fn qwen_cache_zones(
    role: ModelRole,
    plan: &PlannedModelCall,
    tool_catalog: &str,
    context_items: Vec<(&str, &str)>,
    turn_context: &str,
) -> ThreeZonePrompt {
    let base_prompt = "ResearchCode Qwen3.6-27B native mode.\n\
Stable prefix: keep Qwen chat-template and OpenAI-compatible tool schema rules unchanged.\n\
Use provider tool_calls when available. If a model emits visible tool markup, use <tool_call><name>...</name><arguments>{...}</arguments></tool_call> only.\n\
Keep reasoning_content/thinking/reasoning out of visible assistant content.\n\
Tool arguments must be JSON objects and tool names must come from <tool_catalog>.\n\
Do not use DeepSeek DSML markers in Qwen native mode.";
    let temperature_milli = plan.temperature_milli.unwrap_or_default().to_string();
    CachePrefixPolicy::build_zones(
        base_prompt,
        tool_catalog.lines().map(str::to_string).collect(),
        vec![
            ("role", role_label(&role)),
            ("parser_profile", "qwen-openai-tools-or-tool-call-xml"),
            (
                "role_model",
                plan.role_model_name
                    .as_deref()
                    .unwrap_or("Qwen/Qwen3.6-27B"),
            ),
            ("temperature_milli", temperature_milli.as_str()),
        ],
        context_items,
        turn_context,
    )
}

pub fn qwen_system_prompt(zones: &ThreeZonePrompt) -> String {
    let mut prompt = format!(
        "<qwen_cache_zone name=\"A\" purpose=\"stable_system_and_tools\" hash=\"{}\">\n{}\n</qwen_cache_zone>\n\n\
         <qwen_cache_zone name=\"B\" purpose=\"role_parser_and_sampling\" hash=\"{}\">\n{}\n</qwen_cache_zone>",
        zones.zone_a_hash(),
        zones.zone_a,
        zones.zone_b_hash(),
        zones.zone_b
    );
    if !zones.zone_c.trim().is_empty() {
        prompt.push_str(&format!(
            "\n\n<qwen_cache_zone name=\"C\" purpose=\"engineering_context\" hash=\"{}\">\n{}\n</qwen_cache_zone>",
            zones.zone_c_hash(),
            zones.zone_c
        ));
    }
    prompt
}

fn role_label(role: &ModelRole) -> &'static str {
    match role {
        ModelRole::Planner => "planner",
        ModelRole::Executor => "executor",
        ModelRole::Reviewer => "reviewer",
        ModelRole::Researcher => "researcher",
        ModelRole::Summarizer => "summarizer",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_kernel::AgentRole;
    use crate::model_adapter::{PlannedModelCall, ThinkingMode};
    use crate::native_profile::deepseek::role_split::RoleStage;
    use researchcode_kernel::model::OptimizationLevel;

    #[test]
    fn qwen_cache_prefix_keeps_context_in_zone_c() {
        let plan = PlannedModelCall {
            adapter_id: "qwen3-6-27b-native".to_string(),
            optimization_level: OptimizationLevel::Native,
            actual_model_name: "Qwen/Qwen3.6-27B".to_string(),
            display_model_name: "Qwen3.6-27B Native".to_string(),
            thinking_mode: ThinkingMode::NonThinking,
            native_tool_calls: true,
            stable_prompt_prefix: true,
            parser_profile: "qwen".to_string(),
            max_context_tokens: 262_000,
            notes: Vec::new(),
            agent_role: AgentRole::Executor,
            role_stage: RoleStage::Executing,
            role_model_name: Some("Qwen/Qwen3.6-27B".to_string()),
            temperature_milli: Some(200),
        };
        let zones = qwen_cache_zones(
            ModelRole::Executor,
            &plan,
            "- file.read",
            vec![("repo_map", "src/lib.rs")],
            "turn context",
        );

        assert!(zones.zone_a.contains("Qwen3.6-27B native mode"));
        assert!(zones.zone_c.contains("repo_map"));
        assert!(qwen_system_prompt(&zones).contains("qwen_cache_zone"));
    }
}
