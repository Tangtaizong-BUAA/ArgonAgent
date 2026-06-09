// DeepSeek reasoning replay policy and state.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasoningReplayMode {
    NativeField,
    SummarizedOnly,
    Drop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasoningReplayTarget {
    DeepSeekNativeRequest,
    GenericChatMessage,
    ToolResultMessage,
    Artifact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasoningReplayDecision {
    AllowNativeReplay { sanitized: String },
    PersistSanitizedSummary { sanitized: String },
    Drop,
    BlockIncompatibleReplay,
}

pub fn decide_reasoning_replay(
    reasoning_content: &str,
    mode: ReasoningReplayMode,
    target: ReasoningReplayTarget,
) -> ReasoningReplayDecision {
    let sanitized = sanitize_reasoning(reasoning_content);
    match target {
        ReasoningReplayTarget::GenericChatMessage | ReasoningReplayTarget::ToolResultMessage => {
            ReasoningReplayDecision::BlockIncompatibleReplay
        }
        ReasoningReplayTarget::DeepSeekNativeRequest => match mode {
            ReasoningReplayMode::NativeField => {
                ReasoningReplayDecision::AllowNativeReplay { sanitized }
            }
            ReasoningReplayMode::SummarizedOnly => {
                ReasoningReplayDecision::PersistSanitizedSummary { sanitized }
            }
            ReasoningReplayMode::Drop => ReasoningReplayDecision::Drop,
        },
        ReasoningReplayTarget::Artifact => match mode {
            ReasoningReplayMode::NativeField | ReasoningReplayMode::SummarizedOnly => {
                ReasoningReplayDecision::PersistSanitizedSummary { sanitized }
            }
            ReasoningReplayMode::Drop => ReasoningReplayDecision::Drop,
        },
    }
}

pub fn sanitize_reasoning(value: &str) -> String {
    let mut sanitized = value.to_string();
    sanitized = redact_after_prefix(&sanitized, "sk-");
    sanitized = redact_after_prefix(&sanitized, "AKIA");
    sanitized = sanitized.replace(".env", "[REDACTED_PATH]");
    sanitized = sanitized.replace("id_rsa", "[REDACTED_KEY_PATH]");
    sanitized
}

fn redact_after_prefix(value: &str, prefix: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    while let Some(index) = rest.find(prefix) {
        output.push_str(&rest[..index]);
        output.push_str("[REDACTED_SECRET]");
        let after_prefix = &rest[index + prefix.len()..];
        let end = after_prefix
            .find(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\'' || ch == ',')
            .unwrap_or(after_prefix.len());
        rest = &after_prefix[end..];
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[test]
    fn blocks_claude_style_reasoning_replay_as_chat_message() {
        assert_eq!(
            decide_reasoning_replay(
                "Need to inspect parser",
                ReasoningReplayMode::NativeField,
                ReasoningReplayTarget::GenericChatMessage,
            ),
            ReasoningReplayDecision::BlockIncompatibleReplay
        );
        assert_eq!(
            decide_reasoning_replay(
                "Need to inspect parser",
                ReasoningReplayMode::NativeField,
                ReasoningReplayTarget::ToolResultMessage,
            ),
            ReasoningReplayDecision::BlockIncompatibleReplay
        );
    }

    #[test]
    fn allows_native_field_replay_only_for_deepseek_request() {
        assert_eq!(
            decide_reasoning_replay(
                "Need to inspect parser",
                ReasoningReplayMode::NativeField,
                ReasoningReplayTarget::DeepSeekNativeRequest,
            ),
            ReasoningReplayDecision::AllowNativeReplay {
                sanitized: "Need to inspect parser".to_string()
            }
        );
    }

    #[test]
    fn sanitizes_secrets_before_persisting_or_replaying() {
        let decision = decide_reasoning_replay(
            "Secret sk-testsecret in .env",
            ReasoningReplayMode::NativeField,
            ReasoningReplayTarget::Artifact,
        );
        assert_eq!(
            decision,
            ReasoningReplayDecision::PersistSanitizedSummary {
                sanitized: "Secret [REDACTED_SECRET] in [REDACTED_PATH]".to_string()
            }
        );
    }
}

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningEntry {
    pub turn_index: u32,
    pub assistant_message_id: String,
    pub raw_reasoning: String,
    pub sanitized_preview: String,
    pub tokens: u64,
}

#[derive(Debug, Default, Clone)]
pub struct ReasoningReplayManager {
    last_reasoning: HashMap<String, ReasoningEntry>,
}

impl ReasoningReplayManager {
    pub fn capture(
        &mut self,
        session_id: &str,
        turn_index: u32,
        assistant_message_id: impl Into<String>,
        raw_delta: &str,
        sanitized_delta: &str,
    ) {
        let entry = self
            .last_reasoning
            .entry(session_id.to_string())
            .or_insert_with(|| ReasoningEntry {
                turn_index,
                assistant_message_id: assistant_message_id.into(),
                raw_reasoning: String::new(),
                sanitized_preview: String::new(),
                tokens: 0,
            });
        entry.raw_reasoning.push_str(raw_delta);
        entry.sanitized_preview.push_str(sanitized_delta);
    }

    pub fn capture_raw_response(
        &mut self,
        session_id: &str,
        turn_index: u32,
        assistant_message_id: impl Into<String>,
        raw_reasoning: &str,
    ) {
        self.capture(
            session_id,
            turn_index,
            assistant_message_id,
            raw_reasoning,
            &sanitize_reasoning(raw_reasoning),
        );
    }

    pub fn latest(&self, session_id: &str) -> Option<&ReasoningEntry> {
        self.last_reasoning.get(session_id)
    }

    pub fn inject(
        &mut self,
        session_id: &str,
        turn_index: u32,
        assistant_message_id: impl Into<String>,
        raw_reasoning: &str,
    ) {
        let sanitized = sanitize_reasoning(raw_reasoning);
        self.last_reasoning.insert(
            session_id.to_string(),
            ReasoningEntry {
                turn_index,
                assistant_message_id: assistant_message_id.into(),
                raw_reasoning: raw_reasoning.to_string(),
                sanitized_preview: sanitized,
                tokens: 0,
            },
        );
    }

    pub fn drop_session(&mut self, session_id: &str) {
        self.last_reasoning.remove(session_id);
    }

    /// During reasoning budget folding, clear raw reasoning to prevent large replay.
    /// Keeps sanitized preview for observability.
    pub fn compact_old_reasoning(&mut self, current_turn: u32) -> usize {
        let mut compacted = 0;
        for entry in self.last_reasoning.values_mut() {
            if entry
                .raw_reasoning
                .starts_with("[reasoning folded at turn ")
            {
                continue;
            }
            entry.raw_reasoning = format!(
                "[reasoning folded at turn {current_turn} — {} raw tokens cleared; sanitized preview retained]",
                entry.tokens
            );
            compacted += 1;
        }
        compacted
    }
}

#[cfg(test)]
mod manager_tests {
    use super::*;

    #[test]
    fn raw_reasoning_is_separate_from_sanitized_preview() {
        let mut manager = ReasoningReplayManager::default();
        manager.capture("sess", 1, "assistant-1", "sk-secret", "[redacted]");
        let latest = manager.latest("sess").unwrap();
        assert_eq!(latest.raw_reasoning, "sk-secret");
        assert_eq!(latest.sanitized_preview, "[redacted]");
    }

    #[test]
    fn capture_raw_response_sanitizes_preview_only() {
        let mut manager = ReasoningReplayManager::default();
        manager.capture_raw_response("sess", 1, "assistant-1", "Need sk-secret from .env");
        let latest = manager.latest("sess").unwrap();
        assert_eq!(latest.raw_reasoning, "Need sk-secret from .env");
        assert_eq!(
            latest.sanitized_preview,
            "Need [REDACTED_SECRET] from [REDACTED_PATH]"
        );
    }

    #[test]
    fn compact_old_reasoning_folds_raw_replay_once() {
        let mut manager = ReasoningReplayManager::default();
        manager.capture_raw_response("sess", 1, "assistant-1", "private chain");

        assert_eq!(manager.compact_old_reasoning(3), 1);
        let latest = manager.latest("sess").unwrap();
        assert!(latest.raw_reasoning.contains("reasoning folded at turn 3"));
        assert_eq!(latest.sanitized_preview, "private chain");

        assert_eq!(manager.compact_old_reasoning(4), 0);
        assert!(manager
            .latest("sess")
            .unwrap()
            .raw_reasoning
            .contains("turn 3"));
    }
}
