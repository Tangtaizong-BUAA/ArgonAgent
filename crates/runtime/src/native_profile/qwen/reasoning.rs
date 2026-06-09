use std::collections::HashMap;

use crate::native_profile::deepseek::reasoning::sanitize_reasoning;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwenReasoningEntry {
    pub turn_index: u32,
    pub assistant_message_id: String,
    pub raw_reasoning: String,
    pub sanitized_preview: String,
    pub source_field: String,
}

#[derive(Debug, Default, Clone)]
pub struct QwenReasoningReplayManager {
    latest_by_session: HashMap<String, QwenReasoningEntry>,
}

impl QwenReasoningReplayManager {
    pub fn capture_delta(
        &mut self,
        session_id: &str,
        turn_index: u32,
        assistant_message_id: impl Into<String>,
        source_field: impl Into<String>,
        raw_delta: &str,
    ) {
        let source_field = source_field.into();
        let entry = self
            .latest_by_session
            .entry(session_id.to_string())
            .or_insert_with(|| QwenReasoningEntry {
                turn_index,
                assistant_message_id: assistant_message_id.into(),
                raw_reasoning: String::new(),
                sanitized_preview: String::new(),
                source_field: source_field.clone(),
            });
        entry.source_field = source_field;
        entry.raw_reasoning.push_str(raw_delta);
        entry
            .sanitized_preview
            .push_str(&sanitize_reasoning(raw_delta));
    }

    pub fn latest(&self, session_id: &str) -> Option<&QwenReasoningEntry> {
        self.latest_by_session.get(session_id)
    }

    pub fn replay_message(&self, session_id: &str) -> Option<String> {
        let entry = self.latest(session_id)?;
        if entry.sanitized_preview.trim().is_empty() {
            return None;
        }
        Some(format!(
            "[qwen reasoning replay: turn={}, source={}] {}",
            entry.turn_index, entry.source_field, entry.sanitized_preview
        ))
    }

    pub fn drop_session(&mut self, session_id: &str) {
        self.latest_by_session.remove(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen_reasoning_replay_sanitizes_thinking_tags() {
        let mut manager = QwenReasoningReplayManager::default();
        manager.capture_delta(
            "sess",
            2,
            "assistant-2",
            "reasoning_content",
            "Need sk-testsecret from .env",
        );

        let latest = manager.latest("sess").unwrap();
        assert_eq!(latest.raw_reasoning, "Need sk-testsecret from .env");
        assert_eq!(
            latest.sanitized_preview,
            "Need [REDACTED_SECRET] from [REDACTED_PATH]"
        );
        assert!(manager
            .replay_message("sess")
            .unwrap()
            .contains("qwen reasoning replay"));
    }
}
