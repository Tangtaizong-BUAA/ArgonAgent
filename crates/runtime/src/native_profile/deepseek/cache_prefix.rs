use crate::model_adapter::{ModelRole, PlannedModelCall, ThinkingMode};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreeZonePrompt {
    pub zone_a: String,
    pub zone_b: String,
    pub zone_c: String,
}

impl ThreeZonePrompt {
    pub fn zone_a_hash(&self) -> u64 {
        stable_hash(&self.zone_a)
    }

    pub fn zone_b_hash(&self) -> u64 {
        stable_hash(&self.zone_b)
    }

    pub fn zone_c_hash(&self) -> u64 {
        stable_hash(&self.zone_c)
    }
}

#[derive(Debug, Default, Clone)]
pub struct CachePrefixPolicy;

impl CachePrefixPolicy {
    pub fn build_zones(
        base_prompt: &str,
        mut tool_catalog: Vec<String>,
        session_metadata: Vec<(&str, &str)>,
        mut context_items: Vec<(&str, &str)>,
        turn_context: &str,
    ) -> ThreeZonePrompt {
        tool_catalog.sort();
        let mut metadata = session_metadata;
        metadata.sort_by_key(|(key, _)| *key);
        context_items.sort_by_key(|(source, _)| *source);
        let mut zone_c_parts = context_items
            .into_iter()
            .map(|(source, content)| {
                format!("<context_item source=\"{source}\">\n{content}\n</context_item>")
            })
            .collect::<Vec<_>>();
        if !turn_context.trim().is_empty() {
            zone_c_parts.push(turn_context.to_string());
        }
        ThreeZonePrompt {
            zone_a: format!("{base_prompt}\n{}", tool_catalog.join("\n")),
            zone_b: metadata
                .into_iter()
                .map(|(key, value)| format!("{key}: {value}"))
                .collect::<Vec<_>>()
                .join("\n"),
            zone_c: zone_c_parts.join("\n\n"),
        }
    }
}

/// Build DeepSeek-specific cache zones from role, plan, tool catalog, and turn context.
pub fn deepseek_cache_zones(
    role: ModelRole,
    plan: &PlannedModelCall,
    tool_catalog: &str,
    turn_context: &str,
) -> ThreeZonePrompt {
    let temperature_milli = plan.temperature_milli.unwrap_or_default().to_string();
    let base_prompt = "ResearchCode DeepSeek native mode.\n\
Stable prefix: keep these rules unchanged for prefix-cache reuse.\n\
Use DeepSeek native tool calls when available. If native tool calls are unavailable, use one DSML/XML tool call only.\n\
Do not invent tool names. Use only names present in <tool_catalog>.\n\
If transport already provides native tool-call schema, never emit DSML/XML tags.\n\
Keep reasoning_content in the provider reasoning channel. Never replay reasoning_content as a normal user/tool message.\n\
Tool arguments must be valid JSON. If uncertain, ask for context instead of inventing files.\n\
If user input is only greeting/chit-chat and does not require workspace facts, answer directly without any tool call.\n\
Prefer late compaction and preserve stable system/tool catalog ordering.";
    CachePrefixPolicy::build_zones(
        base_prompt,
        tool_catalog.lines().map(str::to_string).collect(),
        vec![
            ("role", role_label(&role)),
            ("thinking_mode", thinking_label(&plan.thinking_mode)),
            ("parser_profile", plan.parser_profile.as_str()),
            (
                "role_model",
                plan.role_model_name.as_deref().unwrap_or("deepseek-chat"),
            ),
            ("temperature_milli", temperature_milli.as_str()),
        ],
        Vec::new(),
        turn_context,
    )
}

/// Build the DeepSeek system prompt from cache zones.
pub fn deepseek_system_prompt(zones: &ThreeZonePrompt) -> String {
    let mut prompt = format!(
        "<cache_zone name=\"A\" purpose=\"stable_system_and_tools\" hash=\"{}\">\n{}\n</cache_zone>\n\n\
         <cache_zone name=\"B\" purpose=\"role_parser_and_sampling\" hash=\"{}\">\n{}\n</cache_zone>\n\n\
         <tool_catalog>\n{}\n</tool_catalog>",
        zones.zone_a_hash(),
        zones.zone_a,
        zones.zone_b_hash(),
        zones.zone_b,
        zones
            .zone_a
            .lines()
            .filter(|line| line.starts_with("- "))
            .collect::<Vec<_>>()
            .join("\n")
    );
    if !zones.zone_c.trim().is_empty() {
        prompt.push_str(&format!(
            "\n\n<cache_zone name=\"C\" purpose=\"turn_context\" hash=\"{}\">\n{}\n</cache_zone>",
            zones.zone_c_hash(),
            zones.zone_c
        ));
    }
    prompt
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

fn stable_hash(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    #[test]
    fn zone_a_hash_is_stable_for_tool_ordering() {
        let one = CachePrefixPolicy::build_zones(
            "base",
            vec!["tool.b".to_string(), "tool.a".to_string()],
            vec![("root", "/tmp")],
            Vec::new(),
            "turn",
        );
        let two = CachePrefixPolicy::build_zones(
            "base",
            vec!["tool.a".to_string(), "tool.b".to_string()],
            vec![("root", "/tmp")],
            Vec::new(),
            "turn",
        );
        assert_eq!(one.zone_a_hash(), two.zone_a_hash());
    }

    #[test]
    fn system_prompt_renders_zone_c_when_present() {
        let zones = CachePrefixPolicy::build_zones(
            "base",
            vec!["- file.read".to_string()],
            vec![("role", "executor")],
            vec![("AGENTS.md", "rules")],
            "per-turn evidence summary",
        );
        let prompt = deepseek_system_prompt(&zones);
        assert!(prompt.contains("<cache_zone name=\"C\" purpose=\"turn_context\" hash=\""));
        assert!(prompt.contains("per-turn evidence summary"));
    }

    #[test]
    fn system_prompt_omits_empty_zone_c() {
        let zones = CachePrefixPolicy::build_zones(
            "base",
            Vec::new(),
            vec![("role", "executor")],
            Vec::new(),
            "",
        );
        let prompt = deepseek_system_prompt(&zones);
        assert!(!prompt.contains("<cache_zone name=\"C\""));
    }

    #[test]
    fn zone_c_context_item_hash_is_stable_for_ordering() {
        let one = CachePrefixPolicy::build_zones(
            "base",
            Vec::new(),
            Vec::new(),
            vec![("git_status", "clean"), ("AGENTS.md", "rules")],
            "",
        );
        let two = CachePrefixPolicy::build_zones(
            "base",
            Vec::new(),
            Vec::new(),
            vec![("AGENTS.md", "rules"), ("git_status", "clean")],
            "",
        );
        assert_eq!(one.zone_c_hash(), two.zone_c_hash());
    }
}

// DeepSeek live request cache-control breakpoint planning.
use researchcode_kernel::message::{ContentBlock, Message, MessageRole};
use researchcode_kernel::model::DeepSeekCapabilities;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachePlan {
    pub breakpoints: Vec<CacheBreakpoint>,
    pub estimated_hit_tokens: u64,
    pub skipped_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheBreakpoint {
    pub after_message_index: usize,
    pub ttl_seconds: u32,
}

pub fn plan_cache_breakpoints(
    messages: &[Message],
    capability: &DeepSeekCapabilities,
) -> CachePlan {
    if !capability.supports_context_caching {
        return CachePlan {
            breakpoints: Vec::new(),
            estimated_hit_tokens: 0,
            skipped_reason: Some("context caching unsupported by variant".to_string()),
        };
    }
    if messages.len() < 3 {
        return CachePlan {
            breakpoints: Vec::new(),
            estimated_hit_tokens: 0,
            skipped_reason: Some("not enough stable prefix messages".to_string()),
        };
    }

    let stable_prefix_end = messages.len().saturating_sub(4).max(1);
    let mut breakpoints = Vec::new();
    // Zone A: system + tools
    if has_system_prefix(messages) {
        breakpoints.push(CacheBreakpoint {
            after_message_index: 0,
            ttl_seconds: 3_600,
        });
    }
    // Zone B: metadata (midpoint between system and stable prefix end)
    let metadata_boundary = ((1 + stable_prefix_end) / 2).max(2);
    if metadata_boundary > 0
        && metadata_boundary < stable_prefix_end
        && breakpoints
            .iter()
            .all(|bp| bp.after_message_index != metadata_boundary)
    {
        breakpoints.push(CacheBreakpoint {
            after_message_index: metadata_boundary,
            ttl_seconds: 300,
        });
    }
    // Zone C: conversation (stable prefix end)
    if stable_prefix_end > 0
        && breakpoints
            .iter()
            .all(|bp| bp.after_message_index != stable_prefix_end)
    {
        breakpoints.push(CacheBreakpoint {
            after_message_index: stable_prefix_end,
            ttl_seconds: 300,
        });
    }

    CachePlan {
        estimated_hit_tokens: estimate_tokens(
            &messages[..=stable_prefix_end.min(messages.len() - 1)],
        ),
        breakpoints,
        skipped_reason: None,
    }
}

pub fn apply_cache_control_blocks(messages: &mut [Message], plan: &CachePlan) {
    for breakpoint in &plan.breakpoints {
        if let Some(message) = messages.get_mut(breakpoint.after_message_index) {
            message.cache_control_ttl = Some(breakpoint.ttl_seconds);
        }
    }
}

fn has_system_prefix(messages: &[Message]) -> bool {
    messages
        .first()
        .map(|message| message.role == MessageRole::System)
        .unwrap_or(false)
}

fn estimate_tokens(messages: &[Message]) -> u64 {
    let chars = messages
        .iter()
        .flat_map(|message| message.content.iter())
        .map(|block| match block {
            ContentBlock::Text { text } => text.len(),
            ContentBlock::Reasoning { sanitized, .. } => sanitized.len(),
            ContentBlock::ToolUse {
                name, input_json, ..
            } => name.len() + input_json.len(),
            ContentBlock::ToolResult { content, .. } => content.len(),
            ContentBlock::Image { .. } => 256,
            ContentBlock::CacheControl { .. } => 0,
        })
        .sum::<usize>();
    (chars as u64 / 4).max(1)
}

/// Apply cache breakpoints to live model request messages.
///
/// Zones:
///   A (system): 1-hour TTL on the first system message
///   B (metadata): 5-minute TTL at roughly mid-point of stable prefix
///   C (conversation): 5-minute TTL at the stable prefix end
pub fn apply_cache_breakpoints_to_model_messages(
    messages: &mut [crate::live_model_request::ModelRequestMessage],
    capability: &DeepSeekCapabilities,
) {
    if !capability.supports_context_caching || messages.len() < 3 {
        return;
    }
    let stable_prefix_end = messages.len().saturating_sub(4).max(1);
    // Zone A: system prompt (first message if it's a "system" role)
    if let Some(first) = messages.first_mut() {
        if first.role == "system" {
            first.cache_control_ttl = Some(3_600);
        }
    }
    // Zone B: metadata boundary (midpoint between system and stable end)
    let metadata_boundary = ((1 + stable_prefix_end) / 2).max(2);
    if metadata_boundary > 0 && metadata_boundary < stable_prefix_end {
        if let Some(msg) = messages.get_mut(metadata_boundary) {
            if msg.cache_control_ttl.is_none() {
                msg.cache_control_ttl = Some(300);
            }
        }
    }
    // Zone C: conversation (stable prefix end)
    if stable_prefix_end > 0 && stable_prefix_end < messages.len() {
        if let Some(msg) = messages.get_mut(stable_prefix_end) {
            if msg.cache_control_ttl.is_none() {
                msg.cache_control_ttl = Some(300);
            }
        }
    }
}

#[cfg(test)]
mod planner_tests {
    use super::*;
    use researchcode_kernel::message::Message;
    use researchcode_kernel::model::{DeepSeekVariant, NativeProtocol, ToolCallingReliability};

    #[test]
    fn long_stable_prefix_creates_cache_breakpoints() {
        let capability = DeepSeekVariant::V31.capabilities();
        let messages = vec![
            Message::text(MessageRole::System, "system prompt with tools"),
            Message::text(MessageRole::User, "turn 1"),
            Message::text(MessageRole::Assistant, "answer 1"),
            Message::text(MessageRole::User, "turn 2"),
            Message::text(MessageRole::Assistant, "answer 2"),
            Message::text(MessageRole::User, "turn 3"),
        ];

        let plan = plan_cache_breakpoints(&messages, &capability);
        assert_eq!(plan.skipped_reason, None);
        assert!(plan
            .breakpoints
            .iter()
            .any(|breakpoint| breakpoint.ttl_seconds == 3_600));
        assert!(plan.estimated_hit_tokens > 0);
    }

    #[test]
    fn unsupported_variant_skips_with_reason() {
        let capability = DeepSeekCapabilities {
            native_tool_calling: ToolCallingReliability::Stable,
            reasoning: false,
            max_context_tokens: 64_000,
            preferred_protocol: NativeProtocol::OpenAiCompatible,
            supports_context_caching: false,
            supports_fim: false,
        };
        let messages = vec![
            Message::text(MessageRole::System, "system"),
            Message::text(MessageRole::User, "user"),
            Message::text(MessageRole::Assistant, "assistant"),
        ];

        let plan = plan_cache_breakpoints(&messages, &capability);
        assert_eq!(
            plan.skipped_reason.as_deref(),
            Some("context caching unsupported by variant")
        );
        assert!(plan.breakpoints.is_empty());
    }

    #[test]
    fn applying_cache_plan_adds_cache_control_blocks() {
        let capability = DeepSeekVariant::V31.capabilities();
        let mut messages = vec![
            Message::text(MessageRole::System, "system"),
            Message::text(MessageRole::User, "user 1"),
            Message::text(MessageRole::Assistant, "assistant 1"),
            Message::text(MessageRole::User, "user 2"),
            Message::text(MessageRole::Assistant, "assistant 2"),
        ];
        let plan = plan_cache_breakpoints(&messages, &capability);
        apply_cache_control_blocks(&mut messages, &plan);

        assert!(messages
            .iter()
            .any(|message| { message.cache_control_ttl.is_some() }));
    }
}
