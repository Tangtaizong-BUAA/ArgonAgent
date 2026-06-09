//! Durable transcript primitives.

use crate::message::Message;
use crate::KernelEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptEntry {
    pub entry_id: String,
    pub sequence: u64,
    pub timestamp: String,
    pub kind: TranscriptKind,
    pub message: Option<Message>,
    pub event: Option<KernelEvent>,
    pub cache_breakpoint: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptKind {
    UserMessage,
    AssistantMessage,
    ToolUse,
    ToolResult,
    ReasoningChain,
    CompactionMarker,
    SubagentBoundary,
    SystemNote,
}

impl TranscriptEntry {
    pub fn raw_volatile_reasoning_would_persist(&self) -> bool {
        self.message
            .as_ref()
            .map(|message| message.contains_raw_volatile_reasoning())
            .unwrap_or(false)
    }

    pub fn validate_persistable(&self) -> Result<(), String> {
        if self.raw_volatile_reasoning_would_persist() {
            return Err("raw volatile reasoning cannot be persisted in transcript".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ContentBlock, Message, MessageRole};

    #[test]
    fn transcript_rejects_raw_volatile_reasoning_persistence() {
        let entry = TranscriptEntry {
            entry_id: "entry_1".to_string(),
            sequence: 1,
            timestamp: "2026-05-10T00:00:00Z".to_string(),
            kind: TranscriptKind::ReasoningChain,
            message: Some(Message::new(
                MessageRole::Assistant,
                vec![ContentBlock::Reasoning {
                    sanitized: "safe summary".to_string(),
                    raw_volatile: Some("raw provider reasoning".to_string()),
                    tokens: Some(12),
                    signature: None,
                }],
            )),
            event: None,
            cache_breakpoint: None,
        };

        assert!(entry.validate_persistable().is_err());
    }

    #[test]
    fn transcript_accepts_sanitized_reasoning_summary() {
        let entry = TranscriptEntry {
            entry_id: "entry_1".to_string(),
            sequence: 1,
            timestamp: "2026-05-10T00:00:00Z".to_string(),
            kind: TranscriptKind::ReasoningChain,
            message: Some(Message::new(
                MessageRole::Assistant,
                vec![ContentBlock::Reasoning {
                    sanitized: "safe summary".to_string(),
                    raw_volatile: None,
                    tokens: Some(12),
                    signature: None,
                }],
            )),
            event: None,
            cache_breakpoint: Some(1),
        };

        assert!(entry.validate_persistable().is_ok());
    }
}
