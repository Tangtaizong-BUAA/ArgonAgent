//! Qwen3.6-27B streaming response parser.
//!
//! This is a no-network parser for recorded OpenAI-compatible/Qwen-style SSE
//! chunks. Qwen native mode keeps thinking content separate from visible
//! assistant content and rejects non-canonical Qwen deployments.

use crate::native_profile::deepseek::reasoning::sanitize_reasoning;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QwenStreamDelta {
    Deployment {
        model: String,
    },
    Thinking {
        sanitized_delta: String,
    },
    Content {
        delta: String,
    },
    ToolCall {
        index: Option<usize>,
        id: Option<String>,
        name: String,
        arguments_fragment: String,
    },
    StopReason(String),
    Telemetry(QwenStreamTelemetry),
    Done,
    Ignored,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QwenStreamTelemetry {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QwenStreamToolCall {
    pub index: usize,
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QwenStreamAssembly {
    pub deployment_model: Option<String>,
    pub thinking_sanitized: String,
    pub content: String,
    pub tool_name: Option<String>,
    pub tool_arguments: String,
    pub tool_calls: Vec<QwenStreamToolCall>,
    pub telemetry: QwenStreamTelemetry,
    pub stop_reason: Option<String>,
    pub done: bool,
}

impl QwenStreamAssembly {
    pub fn apply(&mut self, delta: QwenStreamDelta) {
        match delta {
            QwenStreamDelta::Deployment { model } => self.deployment_model = Some(model),
            QwenStreamDelta::Thinking { sanitized_delta } => {
                self.thinking_sanitized.push_str(&sanitized_delta);
            }
            QwenStreamDelta::Content { delta } => self.content.push_str(&delta),
            QwenStreamDelta::ToolCall {
                index,
                id,
                name,
                arguments_fragment,
            } => {
                let index = index
                    .or_else(|| self.tool_calls.last().map(|call| call.index))
                    .unwrap_or(0);
                let call_position = if let Some(position) =
                    self.tool_calls.iter().position(|call| call.index == index)
                {
                    position
                } else {
                    self.tool_calls.push(QwenStreamToolCall {
                        index,
                        ..QwenStreamToolCall::default()
                    });
                    self.tool_calls.len() - 1
                };
                let call = &mut self.tool_calls[call_position];
                if let Some(id) = id.filter(|value| !value.is_empty()) {
                    call.id = Some(id);
                }
                if !name.is_empty() {
                    call.name = Some(name);
                }
                call.arguments.push_str(&arguments_fragment);
                self.refresh_legacy_tool_fields();
            }
            QwenStreamDelta::Telemetry(telemetry) => {
                merge_telemetry(&mut self.telemetry, telemetry)
            }
            QwenStreamDelta::StopReason(reason) => {
                if !reason.trim().is_empty() {
                    self.stop_reason = Some(reason);
                }
            }
            QwenStreamDelta::Done => self.done = true,
            QwenStreamDelta::Ignored => {}
        }
    }

    fn refresh_legacy_tool_fields(&mut self) {
        if let Some(call) = self.tool_calls.iter().find(|call| call.name.is_some()) {
            self.tool_name = call.name.clone();
            self.tool_arguments = call.arguments.clone();
        }
    }

    pub fn tool_call_pairs(&self) -> Vec<(String, String)> {
        let pairs = self
            .tool_calls
            .iter()
            .filter_map(|call| {
                let name = call.name.as_ref()?.clone();
                let arguments = if call.arguments.trim().starts_with('{') {
                    call.arguments.clone()
                } else {
                    "{}".to_string()
                };
                Some((name, arguments))
            })
            .collect::<Vec<_>>();
        if !pairs.is_empty() {
            return pairs;
        }
        self.tool_name
            .as_ref()
            .map(|name| {
                let arguments = if self.tool_arguments.trim().starts_with('{') {
                    self.tool_arguments.clone()
                } else {
                    "{}".to_string()
                };
                vec![(name.clone(), arguments)]
            })
            .unwrap_or_default()
    }
}

pub fn parse_qwen_sse_line(line: &str) -> Result<QwenStreamDelta, String> {
    Ok(parse_qwen_sse_line_all(line)?
        .into_iter()
        .next()
        .unwrap_or(QwenStreamDelta::Ignored))
}

pub fn parse_qwen_sse_line_all(line: &str) -> Result<Vec<QwenStreamDelta>, String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(':') {
        return Ok(vec![QwenStreamDelta::Ignored]);
    }
    let payload = trimmed
        .strip_prefix("data:")
        .map(str::trim)
        .unwrap_or(trimmed);
    if payload == "[DONE]" {
        return Ok(vec![QwenStreamDelta::Done]);
    }
    let mut deltas = Vec::new();
    if let Some(reason) = extract_json_string(payload, "finish_reason")
        .or_else(|| extract_json_string(payload, "stop_reason"))
        .filter(|value| !value.trim().is_empty())
    {
        deltas.push(QwenStreamDelta::StopReason(reason));
    }
    if payload.contains("\"reasoning_content\"")
        || payload.contains("\"thinking\"")
        || payload.contains("\"reasoning\"")
    {
        if let Some(value) = extract_json_string(payload, "reasoning_content")
            .or_else(|| extract_json_string(payload, "thinking"))
            .or_else(|| extract_json_string(payload, "reasoning"))
            .filter(|value| !value.is_empty())
        {
            deltas.push(QwenStreamDelta::Thinking {
                sanitized_delta: sanitize_reasoning(&value),
            });
        }
    }
    if payload.contains("\"tool_calls\"") {
        deltas.extend(parse_openai_tool_call_deltas(payload));
    }
    if payload.contains("\"content\"") {
        if let Some(delta) =
            extract_json_string(payload, "content").filter(|value| !value.is_empty())
        {
            deltas.push(QwenStreamDelta::Content { delta });
        }
    }
    if payload.contains("\"usage\"") {
        deltas.push(QwenStreamDelta::Telemetry(QwenStreamTelemetry {
            prompt_tokens: extract_json_u64(payload, "prompt_tokens"),
            completion_tokens: extract_json_u64(payload, "completion_tokens"),
            total_tokens: extract_json_u64(payload, "total_tokens"),
        }));
    }
    if payload.contains("\"model\"") {
        if let Some(model) = extract_json_string(payload, "model") {
            deltas.push(QwenStreamDelta::Deployment { model });
        }
    }
    if deltas.is_empty() {
        Ok(vec![QwenStreamDelta::Ignored])
    } else {
        Ok(deltas)
    }
}

pub fn assemble_qwen_sse_lines(lines: &[&str]) -> Result<QwenStreamAssembly, String> {
    let mut assembly = QwenStreamAssembly::default();
    for line in lines {
        if let Some(model) = extract_json_string(line, "model") {
            assembly.apply(QwenStreamDelta::Deployment { model });
        }
        for delta in parse_qwen_sse_line_all(line)? {
            assembly.apply(delta);
        }
    }
    validate_qwen_stream_assembly(&assembly)?;
    Ok(assembly)
}

fn parse_openai_tool_call_deltas(payload: &str) -> Vec<QwenStreamDelta> {
    let Some(array) = extract_json_array(payload, "tool_calls") else {
        return Vec::new();
    };
    split_top_level_json_objects(&array)
        .into_iter()
        .enumerate()
        .filter_map(|(fallback_index, object)| {
            let name = extract_json_string(&object, "name").unwrap_or_default();
            let arguments_fragment = extract_json_string(&object, "arguments").unwrap_or_default();
            let id = extract_json_string(&object, "id");
            let index = extract_json_u64(&object, "index")
                .map(|value| value as usize)
                .or(Some(fallback_index));
            if name.is_empty() && arguments_fragment.is_empty() && id.is_none() {
                return None;
            }
            Some(QwenStreamDelta::ToolCall {
                index,
                id,
                name,
                arguments_fragment,
            })
        })
        .collect()
}

pub fn validate_qwen_stream_assembly(assembly: &QwenStreamAssembly) -> Result<(), String> {
    if let Some(model) = &assembly.deployment_model {
        if !model.contains("Qwen3.6-27B") {
            return Err("Qwen native stream requires Qwen3.6-27B deployment".to_string());
        }
    }
    Ok(())
}

fn merge_telemetry(target: &mut QwenStreamTelemetry, next: QwenStreamTelemetry) {
    if next.prompt_tokens.is_some() {
        target.prompt_tokens = next.prompt_tokens;
    }
    if next.completion_tokens.is_some() {
        target.completion_tokens = next.completion_tokens;
    }
    if next.total_tokens.is_some() {
        target.total_tokens = next.total_tokens;
    }
}

fn extract_json_string(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let rest = input[start..].trim_start();
    if rest.starts_with("null") {
        return None;
    }
    let rest = rest.strip_prefix('"')?;
    let mut output = String::new();
    let mut escaped = false;
    for ch in rest.chars() {
        if escaped {
            output.push(match ch {
                'n' => '\n',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some(output);
        } else {
            output.push(ch);
        }
    }
    None
}

fn extract_json_u64(input: &str, key: &str) -> Option<u64> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let rest = input[start..].trim_start();
    let number = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if number.is_empty() {
        None
    } else {
        number.parse::<u64>().ok()
    }
}

fn extract_json_array(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let rest = input[start..].trim_start();
    if !rest.starts_with('[') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_string {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '[' => depth += 1,
            ']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(rest[..index + ch.len_utf8()].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_json_objects(array_json: &str) -> Vec<String> {
    let trimmed = array_json.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed);
    let mut objects = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in inner.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_string {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(index);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = start.take() {
                        objects.push(inner[start..index + ch.len_utf8()].to_string());
                    }
                }
            }
            _ => {}
        }
    }
    objects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_thinking_from_visible_content() {
        let assembly = assemble_qwen_sse_lines(&[
            r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"reasoning_content":"Need sk-testsecret from .env"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"Visible answer"}}]}"#,
            "data: [DONE]",
        ])
        .unwrap();
        assert_eq!(
            assembly.thinking_sanitized,
            "Need [REDACTED_SECRET] from [REDACTED_PATH]"
        );
        assert_eq!(assembly.content, "Visible answer");
        assert!(assembly.done);
    }

    #[test]
    fn ollama_reasoning_field_is_treated_as_hidden_thinking() {
        let assembly = assemble_qwen_sse_lines(&[
            r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"role":"assistant","content":"","reasoning":"Need sk-testsecret from .env"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"Visible Ollama answer"}}]}"#,
            "data: [DONE]",
        ])
        .unwrap();
        assert_eq!(
            assembly.thinking_sanitized,
            "Need [REDACTED_SECRET] from [REDACTED_PATH]"
        );
        assert_eq!(assembly.content, "Visible Ollama answer");
    }

    #[test]
    fn null_reasoning_does_not_hide_visible_content() {
        let assembly = assemble_qwen_sse_lines(&[
            r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"content":"Visible","reasoning_content":null}}],"usage":null}"#,
            r#"data: {"choices":[{"delta":{"content":" answer","reasoning_content":null}}],"usage":null}"#,
            "data: [DONE]",
        ])
        .unwrap();
        assert_eq!(assembly.thinking_sanitized, "");
        assert_eq!(assembly.content, "Visible answer");
        assert!(assembly.done);
    }

    #[test]
    fn rejects_noncanonical_qwen_deployment() {
        let result = assemble_qwen_sse_lines(&[
            r#"data: {"model":"Qwen/Qwen2-7B","choices":[{"delta":{"content":"No"}}]}"#,
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn assembles_tool_call_fragments() {
        let assembly = assemble_qwen_sse_lines(&[
            r#"data: {"model":"Qwen/Qwen3.6-27B"}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"name":"patch.apply","arguments":"{\"path\":"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"\"src/lib.rs\"}"}}]}}]}"#,
        ])
        .unwrap();
        assert_eq!(assembly.tool_name.as_deref(), Some("patch.apply"));
        assert_eq!(assembly.tool_arguments, "{\"path\":\"src/lib.rs\"}");
    }

    #[test]
    fn assembles_multiple_tool_calls_independently() {
        let assembly = assemble_qwen_sse_lines(&[
            r#"data: {"model":"Qwen/Qwen3.6-27B"}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"file_read","arguments":"{\"path\":\"README.md\"}"}},{"index":1,"id":"call_2","function":{"name":"file_read","arguments":"{\"path\":\"Cargo.toml\"}"}}]}}]}"#,
        ])
        .unwrap();
        let pairs = assembly.tool_call_pairs();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].1, "{\"path\":\"README.md\"}");
        assert_eq!(pairs[1].1, "{\"path\":\"Cargo.toml\"}");
    }

    #[test]
    fn captures_usage_telemetry() {
        let assembly = assemble_qwen_sse_lines(&[
            r#"data: {"model":"Qwen/Qwen3.6-27B"}"#,
            r#"data: {"usage":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}}"#,
        ])
        .unwrap();
        assert_eq!(assembly.telemetry.prompt_tokens, Some(100));
        assert_eq!(assembly.telemetry.completion_tokens, Some(20));
        assert_eq!(assembly.telemetry.total_tokens, Some(120));
    }
}
