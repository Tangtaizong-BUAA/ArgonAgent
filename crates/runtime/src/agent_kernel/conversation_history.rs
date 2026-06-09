use crate::event_log::EventLog;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationToolCall {
    pub id: String,
    pub tool_id: String,
    pub arguments_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMessage {
    pub role: String,
    pub content: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_calls: Vec<ConversationToolCall>,
    pub reasoning_preview: Option<String>,
}

impl ConversationMessage {
    fn new(role: impl Into<String>, content: Option<String>) -> Self {
        Self {
            role: role.into(),
            content,
            tool_call_id: None,
            tool_calls: Vec::new(),
            reasoning_preview: None,
        }
    }
}

pub fn conversation_messages_from_event_log(log: &EventLog) -> Vec<ConversationMessage> {
    let mut messages = Vec::<ConversationMessage>::new();
    let mut assembled_arguments = BTreeMap::<String, String>::new();
    for event in log.iter() {
        match event.event_type.as_str() {
            "model.stream_delta" => {
                let provider = extract_json_string(&event.payload_json, "provider");
                let delta_kind = extract_json_string(&event.payload_json, "delta_kind");
                let preview = extract_json_string(&event.payload_json, "preview");
                match (provider.as_deref(), delta_kind.as_deref(), preview) {
                    (Some("user"), Some("input"), Some(text)) => {
                        messages.push(ConversationMessage::new("user", Some(text)));
                    }
                    (_, Some("content"), Some(text)) if !text.is_empty() => {
                        append_assistant_content(&mut messages, &text);
                    }
                    (_, Some("reasoning_sanitized" | "thinking_sanitized"), Some(text)) => {
                        append_reasoning_preview(&mut messages, &text);
                    }
                    _ => {}
                }
            }
            "tool.call.assembled" => {
                let replayable =
                    extract_json_bool(&event.payload_json, "arguments_replayable").unwrap_or(false);
                if replayable {
                    if let (Some(tool_call_id), Some(arguments_json)) = (
                        extract_json_string(&event.payload_json, "tool_call_id"),
                        extract_json_object(&event.payload_json, "arguments"),
                    ) {
                        assembled_arguments.insert(tool_call_id, arguments_json);
                    }
                }
            }
            "tool.call_requested" => {
                if let (Some(internal_tool_call_id), Some(tool_id)) = (
                    extract_json_string(&event.payload_json, "tool_call_id"),
                    extract_json_string(&event.payload_json, "tool_id"),
                ) {
                    let reasoning_preview =
                        take_pending_assistant_reasoning_before_tool_call(&mut messages);
                    let provider_tool_call_id =
                        extract_json_string(&event.payload_json, "provider_tool_call_id");
                    let arguments_json = assembled_arguments
                        .remove(&internal_tool_call_id)
                        .unwrap_or_else(|| "{}".to_string());
                    messages.push(ConversationMessage {
                        role: "assistant".to_string(),
                        content: None,
                        tool_call_id: None,
                        tool_calls: vec![ConversationToolCall {
                            id: provider_tool_call_id.unwrap_or(internal_tool_call_id),
                            tool_id,
                            arguments_json,
                        }],
                        reasoning_preview,
                    });
                }
            }
            "tool.result_recorded" => {
                if let (Some(internal_tool_call_id), Some(preview)) = (
                    extract_json_string(&event.payload_json, "tool_call_id"),
                    extract_json_string(&event.payload_json, "preview"),
                ) {
                    let provider_tool_call_id =
                        extract_json_string(&event.payload_json, "provider_tool_call_id");
                    messages.push(ConversationMessage {
                        role: "tool".to_string(),
                        content: Some(preview),
                        tool_call_id: Some(provider_tool_call_id.unwrap_or(internal_tool_call_id)),
                        tool_calls: Vec::new(),
                        reasoning_preview: None,
                    });
                }
            }
            "tool.permission.denied" | "tool.permission.resolved" => {
                let decision = extract_json_string(&event.payload_json, "decision");
                if decision.as_deref() == Some("deny")
                    || event.event_type == "tool.permission.denied"
                {
                    let reason = extract_json_string(&event.payload_json, "reason")
                        .unwrap_or_else(|| "permission denied".to_string());
                    let content = format!("PermissionDenied: {reason}");
                    if let Some(tool_call_id) = event_tool_call_id(&event.payload_json) {
                        messages.push(ConversationMessage {
                            role: "tool".to_string(),
                            content: Some(content),
                            tool_call_id: Some(tool_call_id),
                            tool_calls: Vec::new(),
                            reasoning_preview: None,
                        });
                    } else {
                        messages.push(ConversationMessage::new("assistant", Some(content)));
                    }
                }
            }
            "tool.model_readable_error" | "tool.error" => {
                let error_code = extract_json_string(&event.payload_json, "error_code")
                    .or_else(|| {
                        extract_json_object(&event.payload_json, "error")
                            .and_then(|error| extract_json_string(&error, "error_code"))
                    })
                    .or_else(|| extract_json_string(&event.payload_json, "result_tool_id"))
                    .unwrap_or_else(|| "TOOL_ERROR".to_string());
                let short_message = extract_json_string(&event.payload_json, "short_message")
                    .or_else(|| {
                        extract_json_object(&event.payload_json, "error")
                            .and_then(|error| extract_json_string(&error, "short_message"))
                    })
                    .unwrap_or_default();
                let content = if short_message.is_empty() {
                    format!("ToolError: {error_code}")
                } else {
                    format!("ToolError: {error_code}: {short_message}")
                };
                if let Some(tool_call_id) = event_tool_call_id(&event.payload_json) {
                    messages.push(ConversationMessage {
                        role: "tool".to_string(),
                        content: Some(content),
                        tool_call_id: Some(tool_call_id),
                        tool_calls: Vec::new(),
                        reasoning_preview: None,
                    });
                } else {
                    messages.push(ConversationMessage::new("assistant", Some(content)));
                }
            }
            "subagent.completed" | "subagent.summary_recorded" => {
                if let Some(summary) = extract_json_string(&event.payload_json, "summary") {
                    messages.push(ConversationMessage::new(
                        "assistant",
                        Some(format!("task.dispatch summary: {summary}")),
                    ));
                }
            }
            "context.compaction.completed" | "agent.compaction.completed" => {
                messages.push(ConversationMessage::new(
                    "system",
                    Some("[runtime compaction completed; prior turns were folded]".to_string()),
                ));
            }
            _ => {}
        }
    }
    messages
}

pub fn conversation_messages_to_openai_json(messages: &[ConversationMessage]) -> String {
    let mut output = String::from("[");
    for (index, message) in messages.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(&conversation_message_to_openai_json(message));
    }
    output.push(']');
    output
}

pub fn conversation_messages_from_tool_result_continuation(
    prompt: &str,
    tool_calls: Vec<ConversationToolCall>,
    tool_results: Vec<(String, String)>,
    reasoning_preview: Option<String>,
) -> Vec<ConversationMessage> {
    let mut messages = vec![ConversationMessage::new("user", Some(prompt.to_string()))];
    messages.push(ConversationMessage {
        role: "assistant".to_string(),
        content: None,
        tool_call_id: None,
        tool_calls,
        reasoning_preview,
    });
    messages.extend(
        tool_results
            .into_iter()
            .map(|(tool_call_id, content)| ConversationMessage {
                role: "tool".to_string(),
                content: Some(content),
                tool_call_id: Some(tool_call_id),
                tool_calls: Vec::new(),
                reasoning_preview: None,
            }),
    );
    messages
}

fn conversation_message_to_openai_json(message: &ConversationMessage) -> String {
    match message.role.as_str() {
        "tool" => format!(
            "{{\"role\":\"tool\",\"tool_call_id\":\"{}\",\"content\":\"{}\"}}",
            json_escape(message.tool_call_id.as_deref().unwrap_or_default()),
            json_escape(message.content.as_deref().unwrap_or_default())
        ),
        "assistant" if !message.tool_calls.is_empty() => {
            let tool_calls = message
                .tool_calls
                .iter()
                .map(|call| {
                    format!(
                        "{{\"id\":\"{}\",\"type\":\"function\",\"function\":{{\"name\":\"{}\",\"arguments\":\"{}\"}}}}",
                        json_escape(&call.id),
                        json_escape(&provider_tool_name(&call.tool_id)),
                        json_escape(&call.arguments_json)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            let content = message
                .content
                .as_ref()
                .map(|content| format!("\"{}\"", json_escape(content)))
                .unwrap_or_else(|| "null".to_string());
            let reasoning = message
                .reasoning_preview
                .as_ref()
                .map(|reasoning| format!(",\"reasoning_content\":\"{}\"", json_escape(reasoning)))
                .unwrap_or_default();
            format!(
                "{{\"role\":\"assistant\",\"content\":{},\"tool_calls\":[{}]{}}}",
                content, tool_calls, reasoning
            )
        }
        "assistant" => {
            let reasoning = message
                .reasoning_preview
                .as_ref()
                .map(|reasoning| format!(",\"reasoning_content\":\"{}\"", json_escape(reasoning)))
                .unwrap_or_default();
            format!(
                "{{\"role\":\"assistant\",\"content\":\"{}\"{}}}",
                json_escape(message.content.as_deref().unwrap_or_default()),
                reasoning
            )
        }
        _ => format!(
            "{{\"role\":\"{}\",\"content\":\"{}\"}}",
            json_escape(&message.role),
            json_escape(message.content.as_deref().unwrap_or_default())
        ),
    }
}

fn take_pending_assistant_reasoning_before_tool_call(
    messages: &mut Vec<ConversationMessage>,
) -> Option<String> {
    let should_drop = messages.last().is_some_and(|message| {
        message.role == "assistant"
            && message.tool_calls.is_empty()
            && message.tool_call_id.is_none()
            && message
                .content
                .as_deref()
                .map(str::trim)
                .is_some_and(looks_like_tool_preamble)
    });
    if should_drop {
        return messages.pop().and_then(|message| message.reasoning_preview);
    }
    let should_move_reasoning = messages.last().is_some_and(|message| {
        message.role == "assistant"
            && message.tool_calls.is_empty()
            && message.tool_call_id.is_none()
            && message
                .content
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
    });
    if should_move_reasoning {
        return messages.pop().and_then(|message| message.reasoning_preview);
    }
    None
}

fn looks_like_tool_preamble(content: &str) -> bool {
    let text = content.trim();
    if text.chars().count() > 180 {
        return false;
    }
    let lowered = text.to_lowercase();
    let has_probe_verb = [
        "检查", "查看", "读取", "搜索", "浏览", "分析", "列出", "打开", "inspect", "check", "read",
        "search", "look", "list",
    ]
    .iter()
    .any(|needle| lowered.contains(needle));
    if !has_probe_verb {
        return false;
    }
    [
        "let我",
        "让我",
        "我先",
        "先",
        "let me",
        "i’m going to",
        "i'm going to",
    ]
    .iter()
    .any(|prefix| lowered.starts_with(prefix))
}

fn append_assistant_content(messages: &mut Vec<ConversationMessage>, text: &str) {
    if let Some(last) = messages
        .last_mut()
        .filter(|message| message.role == "assistant" && message.tool_calls.is_empty())
    {
        let content = last.content.get_or_insert_with(String::new);
        content.push_str(text);
        return;
    }
    messages.push(ConversationMessage::new(
        "assistant",
        Some(text.to_string()),
    ));
}

fn append_reasoning_preview(messages: &mut Vec<ConversationMessage>, text: &str) {
    if let Some(last) = messages
        .last_mut()
        .filter(|message| message.role == "assistant")
    {
        last.reasoning_preview = Some(text.to_string());
        return;
    }
    let mut message = ConversationMessage::new("assistant", None);
    message.reasoning_preview = Some(text.to_string());
    messages.push(message);
}

fn provider_tool_name(tool_id: &str) -> String {
    tool_id.replace('.', "_")
}

fn event_tool_call_id(payload: &str) -> Option<String> {
    extract_json_string(payload, "provider_tool_call_id")
        .or_else(|| extract_json_string(payload, "tool_call_id"))
}

fn extract_json_string(payload: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let start = payload.find(&marker)? + marker.len() - 1;
    parse_json_string_at(payload, start)
}

fn extract_json_bool(payload: &str, key: &str) -> Option<bool> {
    let marker = format!("\"{key}\":");
    let start = payload.find(&marker)? + marker.len();
    let rest = payload[start..].trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn extract_json_object(payload: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let start = payload.find(&marker)? + marker.len();
    let rest = payload[start..].trim_start();
    if rest.starts_with("null") {
        return None;
    }
    if !rest.starts_with('{') {
        return None;
    }
    let mut depth = 0i64;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && in_string {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[..=index].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_json_string_at(text: &str, start: usize) -> Option<String> {
    let bytes = text.as_bytes();
    if *bytes.get(start)? != b'"' {
        return None;
    }
    let mut result = String::new();
    let mut index = start + 1;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b'"' => return Some(result),
            b'\\' => {
                index += 1;
                let escaped = *bytes.get(index)?;
                match escaped {
                    b'"' => result.push('"'),
                    b'\\' => result.push('\\'),
                    b'/' => result.push('/'),
                    b'n' => result.push('\n'),
                    b'r' => result.push('\r'),
                    b't' => result.push('\t'),
                    b'b' => result.push('\u{0008}'),
                    b'f' => result.push('\u{000c}'),
                    b'u' => {
                        let hex = text.get(index + 1..index + 5)?;
                        let code = u32::from_str_radix(hex, 16).ok()?;
                        result.push(char::from_u32(code)?);
                        index += 4;
                    }
                    _ => return None,
                }
            }
            _ => {
                let ch = text[index..].chars().next()?;
                result.push(ch);
                index += ch.len_utf8() - 1;
            }
        }
        index += 1;
    }
    None
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            other if other.is_control() => format!("\\u{:04x}", other as u32).chars().collect(),
            other => vec![other],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::AgentSession;
    use crate::state::AgentState;

    #[test]
    fn event_log_projects_tool_turn_into_openai_messages() {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session
            .record_model_stream_delta("user_stream", "user", "input", "Read README")
            .unwrap();
        session
            .record_model_stream_delta(
                "assistant_stream",
                "deepseek",
                "content",
                "I will inspect it.",
            )
            .unwrap();
        session
            .record_tool_call_assembled("call_1", "file.read", r#"{"path":"README.md"}"#, true)
            .unwrap();
        session
            .record_tool_call_requested("call_1", "file.read")
            .unwrap();
        session
            .record_tool_call_completed("call_1", "file.read", true)
            .unwrap();
        session
            .record_tool_result_artifact(
                "call_1",
                "file.read",
                "artifact_call_1",
                "fnv64_demo",
                "20 bytes from README.md",
            )
            .unwrap();

        let messages = conversation_messages_from_event_log(session.event_log());
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[2].tool_calls[0].id, "call_1");
        assert_eq!(
            messages[2].tool_calls[0].arguments_json,
            r#"{"path":"README.md"}"#
        );
        assert_eq!(messages[3].role, "tool");
        assert_eq!(messages[3].tool_call_id.as_deref(), Some("call_1"));

        let json = conversation_messages_to_openai_json(&messages);
        assert!(json.contains("\"role\":\"tool\""));
        assert!(json.contains("\"tool_call_id\":\"call_1\""));
        assert!(json.contains("\"name\":\"file_read\""));
        assert!(json.contains("\\\"path\\\":\\\"README.md\\\""));
    }

    #[test]
    fn event_log_projects_provider_tool_call_id_when_available() {
        let mut session = AgentSession::new("proj", "sess_provider_id", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session
            .record_tool_call_assembled_with_provider_id(
                "native_loop_v2_tool_0",
                Some("call_provider_readme"),
                "file.read",
                r#"{"path":"README.md"}"#,
                true,
            )
            .unwrap();
        session
            .record_tool_call_requested_with_provider_id(
                "native_loop_v2_tool_0",
                Some("call_provider_readme"),
                "file.read",
            )
            .unwrap();
        session
            .record_tool_call_completed_with_provider_id(
                "native_loop_v2_tool_0",
                Some("call_provider_readme"),
                "file.read",
                true,
            )
            .unwrap();
        session
            .record_tool_result_artifact_with_provider_id(
                "native_loop_v2_tool_0",
                Some("call_provider_readme"),
                "file.read",
                "artifact_call_1",
                "fnv64_demo",
                "20 bytes from README.md",
            )
            .unwrap();

        let messages = conversation_messages_from_event_log(session.event_log());
        assert_eq!(messages[0].tool_calls[0].id, "call_provider_readme");
        assert_eq!(
            messages[1].tool_call_id.as_deref(),
            Some("call_provider_readme")
        );

        let json = conversation_messages_to_openai_json(&messages);
        assert!(json.contains("\"id\":\"call_provider_readme\""));
        assert!(json.contains("\"tool_call_id\":\"call_provider_readme\""));
        assert!(!json.contains("native_loop_v2_tool_0"));
    }

    #[test]
    fn event_log_projects_reasoning_content_onto_tool_call_message() {
        let mut session = AgentSession::new("proj", "sess_reasoning", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session.transition_to(AgentState::Executing).unwrap();
        session
            .record_model_stream_delta(
                "assistant_stream",
                "deepseek",
                "reasoning_sanitized",
                "plan",
            )
            .unwrap();
        session
            .record_model_stream_delta(
                "assistant_stream",
                "deepseek",
                "content",
                "Let me inspect README.",
            )
            .unwrap();
        session
            .record_tool_call_assembled_with_provider_id(
                "internal_read",
                Some("provider_read"),
                "file.read",
                r#"{"path":"README.md"}"#,
                true,
            )
            .unwrap();
        session
            .record_tool_call_requested_with_provider_id(
                "internal_read",
                Some("provider_read"),
                "file.read",
            )
            .unwrap();

        let messages = conversation_messages_from_event_log(session.event_log());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "assistant");
        assert_eq!(messages[0].tool_calls[0].id, "provider_read");
        assert_eq!(messages[0].reasoning_preview.as_deref(), Some("plan"));

        let json = conversation_messages_to_openai_json(&messages);
        assert!(json.contains("\"reasoning_content\":\"plan\""));
        assert!(json.contains("\"id\":\"provider_read\""));
        assert!(!json.contains("Let me inspect README."));
    }

    #[test]
    fn context_projection_drops_streamed_tool_preamble_before_tool_call() {
        let mut session = AgentSession::new("proj", "sess_preamble_history", "task").unwrap();
        session
            .transition_to(AgentState::Planning)
            .and_then(|_| session.transition_to(AgentState::RetrievingContext))
            .and_then(|_| session.transition_to(AgentState::Executing))
            .unwrap();
        session
            .record_model_stream_delta(
                "stream_1",
                "deepseek",
                "content",
                "Let我先检查项目中的现有计划和相关设计文档。",
            )
            .unwrap();
        session
            .record_tool_call_requested("tool_call_1", "file.list_directory")
            .unwrap();
        session
            .record_tool_result_artifact(
                "tool_call_1",
                "file.list_directory",
                "artifact_1",
                "fnv64_hash",
                "file.list_directory · listed 3 entries",
            )
            .unwrap();

        let messages = conversation_messages_from_event_log(session.event_log());
        let json = conversation_messages_to_openai_json(&messages);

        assert!(!json.contains("Let我先检查"));
        assert!(!messages.iter().any(|message| message
            .content
            .as_deref()
            .is_some_and(|content| content.contains("Let我先检查"))));
        assert!(json.contains("\"tool_calls\""));
        assert!(json.contains("\"role\":\"tool\""));
        assert!(json.contains("file.list_directory"));
    }

    #[test]
    fn tool_result_continuation_projection_preserves_openai_shape() {
        let messages = conversation_messages_from_tool_result_continuation(
            "continue",
            vec![ConversationToolCall {
                id: "call_read".to_string(),
                tool_id: "file.read".to_string(),
                arguments_json: r#"{"path":"README.md"}"#.to_string(),
            }],
            vec![(
                "call_read".to_string(),
                r#"{"ok":true,"preview":"README"}"#.to_string(),
            )],
            Some("reason".to_string()),
        );

        let json = conversation_messages_to_openai_json(&messages);
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"tool_calls\""));
        assert!(json.contains("\"id\":\"call_read\""));
        assert!(json.contains("\"name\":\"file_read\""));
        assert!(json.contains("\"reasoning_content\":\"reason\""));
        assert!(json.contains("\"role\":\"tool\""));
        assert!(json.contains("\"tool_call_id\":\"call_read\""));
    }

    #[test]
    fn event_log_projects_subagent_and_compaction_events() {
        let mut session = AgentSession::new("proj", "sess_subagent_history", "task").unwrap();
        session
            .record_runtime_event(
                "subagent.completed",
                researchcode_kernel::Actor::Runtime,
                r#"{"subagent_id":"subagent_1","summary":"child inspected README"}"#.to_string(),
            )
            .unwrap();
        session
            .record_runtime_event(
                "context.compaction.completed",
                researchcode_kernel::Actor::Runtime,
                "{}".to_string(),
            )
            .unwrap();

        let messages = conversation_messages_from_event_log(session.event_log());
        let json = conversation_messages_to_openai_json(&messages);
        assert!(json.contains("task.dispatch summary: child inspected README"));
        assert!(json.contains("[runtime compaction completed"));
    }

    #[test]
    fn event_log_projects_permission_and_tool_errors_as_tool_messages() {
        let mut session = AgentSession::new("proj", "sess_error_history", "task").unwrap();
        session
            .record_runtime_event(
                "tool.permission.denied",
                researchcode_kernel::Actor::Runtime,
                r#"{"tool_call_id":"call_write","reason":"write denied"}"#.to_string(),
            )
            .unwrap();
        session
            .record_runtime_event(
                "tool.model_readable_error",
                researchcode_kernel::Actor::Runtime,
                r#"{"tool_call_id":"call_edit","error":{"error_code":"SCHEMA_VALIDATION_FAILED","short_message":"missing old_string"}}"#.to_string(),
            )
            .unwrap();

        let messages = conversation_messages_from_event_log(session.event_log());
        let json = conversation_messages_to_openai_json(&messages);
        assert!(json.contains("\"role\":\"tool\""));
        assert!(json.contains("\"tool_call_id\":\"call_write\""));
        assert!(json.contains("PermissionDenied: write denied"));
        assert!(json.contains("\"tool_call_id\":\"call_edit\""));
        assert!(json.contains("ToolError: SCHEMA_VALIDATION_FAILED: missing old_string"));
    }
}
