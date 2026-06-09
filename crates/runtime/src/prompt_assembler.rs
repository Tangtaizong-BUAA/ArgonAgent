//! Native DeepSeek/Qwen prompt assembly.
//!
//! This is the layer that turns product/runtime state into model-facing
//! instructions. It follows the same principle as ClaudeCode-style scaffolding:
//! keep a stable system prefix, constrain tool use explicitly, and keep
//! volatile task/context content separate so model-specific parsers can remain
//! deterministic.

use crate::context_budget::{
    allocate_native_context_budget, validate_context_budget, ContextBudget, ContextBudgetValidation,
};
use crate::live_model_request::ModelRequestMessage;
use crate::model_adapter::{ModelRole, PlannedModelCall, ThinkingMode};
use crate::native_profile::deepseek::cache_prefix;
use crate::native_profile::{profile_for_family, NativeProfile};
use researchcode_kernel::context::{estimate_tokens, ContextBundle, ContextItem};
use researchcode_kernel::model::NativeModelFamily;
use researchcode_kernel::tool::{
    tool_capability_status_str, tool_catalog_hash, ToolRisk, ToolSpec,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePromptRequest<'a> {
    pub family: NativeModelFamily,
    pub role: ModelRole,
    pub plan: &'a PlannedModelCall,
    pub context: &'a ContextBundle,
    pub tools: &'a [ToolSpec],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePrompt {
    pub family: NativeModelFamily,
    pub context_budget: ContextBudget,
    pub budget_validation: ContextBudgetValidation,
    pub system_prompt: String,
    pub user_prompt: String,
    pub tool_catalog: String,
    pub context_text: String,
    pub estimated_input_tokens: u64,
    pub budget_warnings: Vec<String>,
}

pub fn assemble_native_prompt(request: NativePromptRequest<'_>) -> NativePrompt {
    let native_profile = profile_for_family(request.family.clone());
    let context_budget =
        allocate_native_context_budget(request.family.clone(), request.role.clone(), None);
    let budget_validation = validate_context_budget(&context_budget);
    let tool_catalog = stable_tool_catalog(request.tools, context_budget.max_active_tools);
    let (context_text, mut budget_warnings) =
        context_to_text_with_budget(request.context, &context_budget);
    let system_prompt = match request.family {
        NativeModelFamily::DeepSeek => {
            debug_assert!(native_profile.supports_reasoning_replay());
            deepseek_system_prompt(
                request.role.clone(),
                request.plan,
                &tool_catalog,
                &context_text,
            )
        }
        NativeModelFamily::Qwen => qwen_system_prompt(request.plan, &tool_catalog, &context_budget),
    };
    let user_prompt = format!(
        "Role: {}\nTask and context follow.\n\n<context>\n{}\n</context>\n\nReturn either a concise answer or one structured tool call.",
        role_label(&request.role),
        context_text
    );
    let estimated_input_tokens = estimate_tokens(&system_prompt) + estimate_tokens(&user_prompt);
    let prompt_scaffold_estimate = estimate_tokens(&system_prompt);
    if prompt_scaffold_estimate > context_budget.prompt_scaffold_tokens() {
        budget_warnings.push(format!(
            "system prompt estimate {} exceeds scaffold budget {}",
            prompt_scaffold_estimate,
            context_budget.prompt_scaffold_tokens()
        ));
    }
    NativePrompt {
        family: request.family,
        context_budget,
        budget_validation,
        system_prompt,
        user_prompt,
        tool_catalog,
        context_text,
        estimated_input_tokens,
        budget_warnings,
    }
}

pub fn native_prompt_messages(prompt: &NativePrompt) -> Vec<ModelRequestMessage> {
    vec![
        ModelRequestMessage {
            role: "system".to_string(),
            content: prompt.system_prompt.clone(),
            cache_control_ttl: None,
        },
        ModelRequestMessage {
            role: "user".to_string(),
            content: prompt.user_prompt.clone(),
            cache_control_ttl: None,
        },
    ]
}

fn deepseek_system_prompt(
    role: ModelRole,
    plan: &PlannedModelCall,
    tool_catalog: &str,
    turn_context: &str,
) -> String {
    let zones = cache_prefix::deepseek_cache_zones(role, plan, tool_catalog, turn_context);
    cache_prefix::deepseek_system_prompt(&zones)
}

fn qwen_system_prompt(
    plan: &PlannedModelCall,
    tool_catalog: &str,
    context_budget: &ContextBudget,
) -> String {
    format!(
        "ResearchCode Qwen3.6-27B native mode.\n\
         Use the Qwen-specific chat/template/parser contract; generic OpenAI-compatible transport alone is not native mode.\n\
         Use thinking for planning, diagnosis, and review; preserve thinking for tool-heavy execution when requested.\n\
         Make patch-sized edits, require fresh file context before patching, and never modify a stale or hallucinated path.\n\
         Tool arguments must be valid JSON. Return at most one structured tool call per step.\n\
         Native context budget is 262K tokens unless deployment capability explicitly proves otherwise.\n\
         Active prompt scaffold level: {:?}. Active tool limit: {}.\n\
         Thinking mode: {}.\n\
         Parser profile: {}.\n\n\
         <tool_catalog>\n{}\n</tool_catalog>",
        context_budget.scaffold_level,
        context_budget.max_active_tools,
        thinking_label(&plan.thinking_mode),
        plan.parser_profile,
        tool_catalog
    )
}

fn stable_tool_catalog(tools: &[ToolSpec], max_active_tools: usize) -> String {
    let mut tools = tools
        .iter()
        .filter(|tool| tool.enabled_by_default)
        .collect::<Vec<_>>();
    tools.sort_by(|left, right| left.tool_id.cmp(&right.tool_id));
    let rows = tools
        .into_iter()
        .take(max_active_tools)
        .map(|tool| {
            format!(
                "- id={} aliases=[{}] status={} risk={} permission_kind={:?} renderer={:?} concurrency_safe={} result_policy={:?} max_result_chars={} input_schema={}",
                tool.tool_id,
                tool.provider_aliases.join(","),
                tool_capability_status_str(&tool.capability_status),
                risk_label(&tool.risk),
                tool.permission_kind,
                tool.renderer,
                tool.concurrency_safe,
                tool.result_policy,
                tool.max_result_size_chars,
                tool.input_schema_json
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "catalog_hash={} active_tool_count={}\n{}",
        tool_catalog_hash(),
        rows.lines().count(),
        rows
    )
}

fn context_to_text_with_budget(
    context: &ContextBundle,
    budget: &ContextBudget,
) -> (String, Vec<String>) {
    let mut used_tokens = 0;
    let mut included = Vec::new();
    let mut omitted = Vec::new();
    for item in &context.items {
        if used_tokens + item.token_estimate <= budget.dynamic_context_tokens() {
            used_tokens += item.token_estimate;
            included.push(format_context_item(item));
        } else {
            omitted.push(format!("{}:{}", item_kind_label(&item.kind), item.source));
        }
    }
    let mut warnings = Vec::new();
    if !omitted.is_empty() {
        warnings.push(format!(
            "context omitted {} items over dynamic budget {}",
            omitted.len(),
            budget.dynamic_context_tokens()
        ));
        included.push(format!(
            "[context_budget_omitted tokens_used={} dynamic_budget={}]\n{}",
            used_tokens,
            budget.dynamic_context_tokens(),
            omitted.join(", ")
        ));
    }
    (included.join("\n\n"), warnings)
}

fn format_context_item(item: &ContextItem) -> String {
    format!(
        "[{} source={} privacy={} tokens={}]\n{}",
        item_kind_label(&item.kind),
        item.source,
        item.privacy_class,
        item.token_estimate,
        item.content
    )
}

fn thinking_label(mode: &ThinkingMode) -> &'static str {
    match mode {
        ThinkingMode::Thinking => "thinking",
        ThinkingMode::NonThinking => "non-thinking",
        ThinkingMode::PreserveThinking => "preserve-thinking",
    }
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

fn risk_label(risk: &ToolRisk) -> &'static str {
    match risk {
        ToolRisk::ReadOnly => "read_only",
        ToolRisk::WritesFiles => "writes_files",
        ToolRisk::ExecutesCommand => "executes_command",
        ToolRisk::UsesNetwork => "uses_network",
        ToolRisk::ExportsArtifact => "exports_artifact",
        ToolRisk::Interactive => "interactive",
    }
}

fn item_kind_label(kind: &researchcode_kernel::context::ContextItemKind) -> &'static str {
    match kind {
        researchcode_kernel::context::ContextItemKind::UserTask => "user_task",
        researchcode_kernel::context::ContextItemKind::ProjectInstructions => {
            "project_instructions"
        }
        researchcode_kernel::context::ContextItemKind::Plan => "plan",
        researchcode_kernel::context::ContextItemKind::RepoMap => "repo_map",
        researchcode_kernel::context::ContextItemKind::FileSnippet => "file_snippet",
        researchcode_kernel::context::ContextItemKind::SearchResult => "search_result",
        researchcode_kernel::context::ContextItemKind::GitStatus => "git_status",
        researchcode_kernel::context::ContextItemKind::ToolResultPreview => "tool_result_preview",
        researchcode_kernel::context::ContextItemKind::ResearchProfile => "research_profile",
        researchcode_kernel::context::ContextItemKind::PrivacyReport => "privacy_report",
        researchcode_kernel::context::ContextItemKind::Memory => "memory",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_builder::ContextBundleBuilder;
    use crate::model_adapter::{
        DeepSeekNativeAdapter, ModelAdapter, ModelAdapterRequest, QwenNativeAdapter,
    };
    use researchcode_kernel::model::{NativeModelProfile, OptimizationLevel};
    use researchcode_kernel::tool::core_tool_specs;

    #[test]
    fn deepseek_prompt_preserves_reasoning_and_prefix_cache_rules() {
        let plan = deepseek_plan(ModelRole::Planner);
        let context = sample_context("deepseek");
        let prompt = assemble_native_prompt(NativePromptRequest {
            family: NativeModelFamily::DeepSeek,
            role: ModelRole::Planner,
            plan: &plan,
            context: &context,
            tools: &core_tool_specs(),
        });
        assert!(prompt.system_prompt.contains("prefix-cache"));
        assert!(prompt.system_prompt.contains("<cache_zone name=\"A\""));
        assert!(prompt
            .system_prompt
            .contains("<cache_zone name=\"B\" purpose=\"role_parser_and_sampling\""));
        assert_eq!(
            prompt.context_budget.scaffold_level,
            crate::context_budget::ScaffoldLevel::DeepSeekFull
        );
        assert!(prompt.budget_validation.ok);
        assert!(prompt.system_prompt.contains("reasoning_content"));
        assert!(prompt.system_prompt.contains("DSML/XML"));
        assert!(prompt.system_prompt.contains("status=production"));
        assert!(prompt.system_prompt.contains("file.read"));
        assert!(!prompt.system_prompt.contains("artifact.export"));
        let messages = native_prompt_messages(&prompt);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
    }

    #[test]
    fn qwen_prompt_preserves_qwen_native_rules() {
        let plan = qwen_plan(ModelRole::Executor);
        let context = sample_context("qwen");
        let prompt = assemble_native_prompt(NativePromptRequest {
            family: NativeModelFamily::Qwen,
            role: ModelRole::Executor,
            plan: &plan,
            context: &context,
            tools: &core_tool_specs(),
        });
        assert!(prompt.system_prompt.contains("Qwen3.6-27B native mode"));
        assert_eq!(
            prompt.context_budget.scaffold_level,
            crate::context_budget::ScaffoldLevel::QwenFast
        );
        assert!(prompt.budget_validation.ok);
        assert!(prompt.system_prompt.contains("patch-sized edits"));
        assert!(prompt.system_prompt.contains("262K"));
        assert!(prompt.tool_catalog.lines().count() <= prompt.context_budget.max_active_tools + 1);
        assert!(!prompt.tool_catalog.contains("shell.command"));
        assert!(prompt.user_prompt.contains("<context>"));
    }

    #[test]
    fn qwen_prompt_omits_context_over_dynamic_budget() {
        let plan = qwen_plan(ModelRole::Executor);
        let mut context = sample_context("qwen");
        context
            .items
            .push(researchcode_kernel::context::ContextItem {
                item_id: "huge".to_string(),
                kind: researchcode_kernel::context::ContextItemKind::FileSnippet,
                source: "src/huge.rs".to_string(),
                content: "huge file".to_string(),
                token_estimate: 500_000,
                privacy_class: "internal".to_string(),
            });
        let prompt = assemble_native_prompt(NativePromptRequest {
            family: NativeModelFamily::Qwen,
            role: ModelRole::Executor,
            plan: &plan,
            context: &context,
            tools: &core_tool_specs(),
        });
        assert!(prompt
            .budget_warnings
            .iter()
            .any(|warning| warning.contains("context omitted")));
        assert!(prompt.context_text.contains("context_budget_omitted"));
        assert!(!prompt.context_text.contains("huge file"));
    }

    fn sample_context(model_family: &str) -> researchcode_kernel::context::ContextBundle {
        let mut builder = ContextBundleBuilder::new("bundle", model_family, 16_000);
        builder.add_user_task("Fix parser without touching secrets");
        builder.build()
    }

    fn deepseek_plan(role: ModelRole) -> PlannedModelCall {
        let adapter = DeepSeekNativeAdapter::new(
            NativeModelProfile {
                profile_id: "deepseek-v4-native".to_string(),
                family: NativeModelFamily::DeepSeek,
                optimization_level: OptimizationLevel::Native,
            },
            "deepseek-v4-flash",
        )
        .unwrap();
        adapter
            .plan_call(&ModelAdapterRequest {
                role,
                task_summary: "prompt".to_string(),
                requires_tools: true,
                context_tokens_estimate: 2_000,
            })
            .unwrap()
    }

    fn qwen_plan(role: ModelRole) -> PlannedModelCall {
        let adapter = QwenNativeAdapter::new(
            NativeModelProfile {
                profile_id: "qwen3-6-27b-native".to_string(),
                family: NativeModelFamily::Qwen,
                optimization_level: OptimizationLevel::Native,
            },
            "Qwen/Qwen3.6-27B",
        )
        .unwrap();
        adapter
            .plan_call(&ModelAdapterRequest {
                role,
                task_summary: "prompt".to_string(),
                requires_tools: true,
                context_tokens_estimate: 2_000,
            })
            .unwrap()
    }
}
