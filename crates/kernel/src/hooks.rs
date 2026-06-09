//! Permission-safe hook primitives.

use crate::message::StopReason;
use crate::model::NativeModelFamily;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart {
        session_id: String,
        model_family: NativeModelFamily,
    },
    UserPromptSubmit {
        text: String,
        attachments: Vec<String>,
    },
    PreToolUse {
        tool_id: String,
        args_json: String,
        provider_tool_use_id: Option<String>,
    },
    PostToolUse {
        tool_id: String,
        result_preview: String,
        ok: bool,
        duration_ms: u64,
    },
    PostToolUseFailure {
        tool_id: String,
        error: String,
        retryable: bool,
    },
    PreCompact {
        before_tokens: u64,
        will_keep_messages: usize,
    },
    PostCompact {
        after_tokens: u64,
        summary_preview: String,
    },
    Stop {
        reason: StopReason,
    },
    ReasoningChainCompleted {
        sanitized: String,
        tokens: u64,
        elapsed_ms: u64,
    },
    DsmlFallbackTriggered {
        raw_content_preview: String,
        parsed_tool_count: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookDecision {
    Allow,
    Modify { new_input_json: String },
    Deny { reason: String },
    Warn { warning: String },
}

impl HookDecision {
    /// Validate a HookDecision, ensuring `Modify` contains valid JSON.
    pub fn validate(&self) -> Result<(), String> {
        if let HookDecision::Modify { new_input_json } = self {
            serde_json::from_str::<serde_json::Value>(new_input_json)
                .map_err(|e| format!("HookDecision::Modify input is not valid JSON: {e}"))?;
        }
        Ok(())
    }
}

pub trait Hook: Send + Sync {
    fn matches(&self, event: &HookEvent) -> bool;
    fn handle(&self, event: &HookEvent) -> HookDecision;
    fn timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookTimeoutPolicy {
    AllowWithWarning,
    DenyWithError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookDispatchPolicy {
    pub default_timeout_ms: u64,
    pub timeout_policy: HookTimeoutPolicy,
    pub permission_gate_remains_authoritative: bool,
}

impl Default for HookDispatchPolicy {
    fn default() -> Self {
        Self {
            default_timeout_ms: 5_000,
            timeout_policy: HookTimeoutPolicy::DenyWithError,
            permission_gate_remains_authoritative: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DenyShellHook;

    impl Hook for DenyShellHook {
        fn matches(&self, event: &HookEvent) -> bool {
            matches!(event, HookEvent::PreToolUse { tool_id, .. } if tool_id == "shell.command")
        }

        fn handle(&self, _event: &HookEvent) -> HookDecision {
            HookDecision::Deny {
                reason: "shell disabled for test".to_string(),
            }
        }
    }

    #[test]
    fn hook_can_match_and_return_decision() {
        let hook = DenyShellHook;
        let event = HookEvent::PreToolUse {
            tool_id: "shell.command".to_string(),
            args_json: "{\"command\":\"pwd\"}".to_string(),
            provider_tool_use_id: Some("toolu_shell".to_string()),
        };

        assert!(hook.matches(&event));
        assert_eq!(
            hook.handle(&event),
            HookDecision::Deny {
                reason: "shell disabled for test".to_string()
            }
        );
    }

    #[test]
    fn default_policy_denies_on_timeout() {
        let policy = HookDispatchPolicy::default();
        assert_eq!(policy.default_timeout_ms, 5_000);
        assert_eq!(policy.timeout_policy, HookTimeoutPolicy::DenyWithError);
        assert!(policy.permission_gate_remains_authoritative);
    }
}
