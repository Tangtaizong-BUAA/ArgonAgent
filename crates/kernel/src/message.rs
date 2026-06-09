//! Provider-agnostic message primitives for the agent kernel.
//!
//! Runtime/provider adapters may serialize these blocks differently, but the
//! kernel contract keeps text, reasoning, tool use, tool result, and cache
//! control as first-class content instead of lossy string concatenation.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Reasoning {
        sanitized: String,
        raw_volatile: Option<String>,
        tokens: Option<u64>,
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    Image {
        source: ImageSource,
    },
    CacheControl {
        ttl_seconds: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    Path(String),
    DataUrl(String),
    Url(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: MessageRole,
    pub content: Vec<ContentBlock>,
    /// Anthropic API `cache_control` at the message level (not inside `content`).
    pub cache_control_ttl: Option<u32>,
}

impl Message {
    pub fn new(role: MessageRole, content: Vec<ContentBlock>) -> Self {
        Self {
            role,
            content,
            cache_control_ttl: None,
        }
    }

    pub fn text(role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![ContentBlock::Text { text: text.into() }],
            cache_control_ttl: None,
        }
    }

    pub fn contains_raw_volatile_reasoning(&self) -> bool {
        self.content.iter().any(|block| {
            matches!(
                block,
                ContentBlock::Reasoning {
                    raw_volatile: Some(value),
                    ..
                } if !value.is_empty()
            )
        })
    }

    pub fn provider_tool_use_ids(&self) -> Vec<&str> {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
    Refusal,
    ReasoningExhausted,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_keeps_reasoning_and_tool_ids_as_structured_blocks() {
        let message = Message::new(
            MessageRole::Assistant,
            vec![
                ContentBlock::Text {
                    text: "I will inspect the file.".to_string(),
                },
                ContentBlock::Reasoning {
                    sanitized: "Need file context".to_string(),
                    raw_volatile: Some("Need file context with provider trace".to_string()),
                    tokens: Some(4),
                    signature: Some("sig_1".to_string()),
                },
                ContentBlock::ToolUse {
                    id: "toolu_1".to_string(),
                    name: "file.read".to_string(),
                    input_json: "{\"path\":\"README.md\"}".to_string(),
                },
            ],
        );

        assert!(message.contains_raw_volatile_reasoning());
        assert_eq!(message.provider_tool_use_ids(), vec!["toolu_1"]);
    }

    #[test]
    fn tool_result_keeps_provider_tool_use_id() {
        let message = Message::new(
            MessageRole::Tool,
            vec![ContentBlock::ToolResult {
                tool_use_id: "call_abc".to_string(),
                content: "{\"ok\":true}".to_string(),
                is_error: false,
            }],
        );

        assert_eq!(
            message.content,
            vec![ContentBlock::ToolResult {
                tool_use_id: "call_abc".to_string(),
                content: "{\"ok\":true}".to_string(),
                is_error: false,
            }]
        );
    }
}
