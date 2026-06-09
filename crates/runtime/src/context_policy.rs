//! Model-specific context policy for native DeepSeek/Qwen modes.

use crate::context_budget::{
    DEEPSEEK_COMPACTION_THRESHOLD_TOKENS, DEEPSEEK_EFFECTIVE_CONTEXT_LIMIT_TOKENS,
};
use researchcode_kernel::model::NativeModelFamily;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeContextPolicy {
    pub family: NativeModelFamily,
    pub max_context_tokens: u64,
    pub compaction_threshold_tokens: u64,
    pub preserve_stable_prompt_prefix: bool,
    pub preserve_reasoning_channel: bool,
    pub patch_sized_edits: bool,
    pub prefix_cache_aware_ordering: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextAction {
    KeepFullContext,
    CompactHistory,
    StopAndSummarize,
}

pub fn native_context_policy(family: NativeModelFamily) -> NativeContextPolicy {
    match family {
        NativeModelFamily::DeepSeek => NativeContextPolicy {
            family,
            max_context_tokens: DEEPSEEK_EFFECTIVE_CONTEXT_LIMIT_TOKENS,
            compaction_threshold_tokens: DEEPSEEK_COMPACTION_THRESHOLD_TOKENS,
            preserve_stable_prompt_prefix: true,
            preserve_reasoning_channel: true,
            patch_sized_edits: false,
            prefix_cache_aware_ordering: true,
        },
        NativeModelFamily::Qwen => NativeContextPolicy {
            family,
            max_context_tokens: 262_000,
            compaction_threshold_tokens: 210_000,
            preserve_stable_prompt_prefix: true,
            preserve_reasoning_channel: true,
            patch_sized_edits: true,
            prefix_cache_aware_ordering: false,
        },
    }
}

pub fn decide_context_action(policy: &NativeContextPolicy, current_tokens: u64) -> ContextAction {
    if current_tokens >= policy.max_context_tokens {
        ContextAction::StopAndSummarize
    } else if current_tokens >= policy.compaction_threshold_tokens {
        ContextAction::CompactHistory
    } else {
        ContextAction::KeepFullContext
    }
}

pub fn order_context_items_for_cache<'a>(
    policy: &NativeContextPolicy,
    stable_items: &'a [&'a str],
    volatile_items: &'a [&'a str],
) -> Vec<&'a str> {
    let mut output = Vec::new();
    if policy.prefix_cache_aware_ordering {
        output.extend_from_slice(stable_items);
        output.extend_from_slice(volatile_items);
    } else {
        output.extend_from_slice(volatile_items);
        output.extend_from_slice(stable_items);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_uses_256k_safe_cap_and_cache_ordering() {
        let policy = native_context_policy(NativeModelFamily::DeepSeek);
        assert_eq!(
            policy.max_context_tokens,
            DEEPSEEK_EFFECTIVE_CONTEXT_LIMIT_TOKENS
        );
        assert_eq!(
            decide_context_action(&policy, DEEPSEEK_COMPACTION_THRESHOLD_TOKENS - 1),
            ContextAction::KeepFullContext
        );
        assert_eq!(
            decide_context_action(&policy, DEEPSEEK_COMPACTION_THRESHOLD_TOKENS),
            ContextAction::CompactHistory
        );
        assert_eq!(
            decide_context_action(&policy, 256_000),
            ContextAction::StopAndSummarize
        );
        assert_eq!(
            order_context_items_for_cache(&policy, &["system", "tools"], &["task"]),
            vec!["system", "tools", "task"]
        );
    }

    #[test]
    fn qwen_uses_262k_budget_and_patch_sized_edits() {
        let policy = native_context_policy(NativeModelFamily::Qwen);
        assert_eq!(policy.max_context_tokens, 262_000);
        assert!(policy.patch_sized_edits);
        assert_eq!(
            decide_context_action(&policy, 220_000),
            ContextAction::CompactHistory
        );
        assert_eq!(
            decide_context_action(&policy, 262_000),
            ContextAction::StopAndSummarize
        );
    }
}
