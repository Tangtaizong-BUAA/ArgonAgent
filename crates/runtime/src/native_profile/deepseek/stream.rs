//! DeepSeek streaming response parser and state helpers.
//!
//! This module owns DeepSeek-native stream parsing under `native_profile::deepseek`;
//! the legacy top-level `deepseek_stream` module is now a deprecated re-export.

use crate::native_profile::deepseek::reasoning::sanitize_reasoning;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepSeekStreamDelta {
    Reasoning {
        sanitized_delta: String,
        raw_delta: String,
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
    Telemetry(DeepSeekStreamTelemetry),
    Done,
    Ignored,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeepSeekStreamTelemetry {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub prompt_cache_hit_tokens: Option<u64>,
    pub prompt_cache_miss_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeepSeekStreamToolCall {
    pub index: usize,
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeepSeekStreamAssembly {
    pub reasoning_sanitized: String,
    /// Raw DeepSeek thinking/reasoning text for provider continuation only.
    ///
    /// This must stay volatile. It is needed because DeepSeek thinking-mode
    /// tool continuations require the assistant thinking block from the same
    /// turn to be replayed to the provider, but raw reasoning must not be
    /// written to event logs, transcripts, artifacts, or visible UI output.
    pub reasoning_raw_volatile: String,
    pub content: String,
    pub tool_name: Option<String>,
    pub tool_arguments: String,
    pub tool_calls: Vec<DeepSeekStreamToolCall>,
    pub telemetry: DeepSeekStreamTelemetry,
    pub stop_reason: Option<String>,
    pub done: bool,
}

impl DeepSeekStreamAssembly {
    pub fn apply(&mut self, delta: DeepSeekStreamDelta) {
        match delta {
            DeepSeekStreamDelta::Reasoning {
                sanitized_delta,
                raw_delta,
            } => {
                self.reasoning_sanitized.push_str(&sanitized_delta);
                self.reasoning_raw_volatile.push_str(&raw_delta);
            }
            DeepSeekStreamDelta::Content { delta } => self.content.push_str(&delta),
            DeepSeekStreamDelta::ToolCall {
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
                    self.tool_calls.push(DeepSeekStreamToolCall {
                        index,
                        ..DeepSeekStreamToolCall::default()
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
            DeepSeekStreamDelta::Telemetry(telemetry) => {
                merge_telemetry(&mut self.telemetry, telemetry)
            }
            DeepSeekStreamDelta::StopReason(reason) => {
                if !reason.trim().is_empty() {
                    self.stop_reason = Some(reason);
                }
            }
            DeepSeekStreamDelta::Done => self.done = true,
            DeepSeekStreamDelta::Ignored => {}
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
        // Fallback: produce multiple pairs from tool_calls entries that have
        // arguments, using the shared tool_name when individual names are absent.
        let fallback = self
            .tool_calls
            .iter()
            .filter_map(|call| {
                let name = call.name.clone().or_else(|| self.tool_name.clone())?;
                let arguments = if call.arguments.trim().starts_with('{') {
                    call.arguments.clone()
                } else {
                    "{}".to_string()
                };
                if arguments == "{}" && call.arguments.trim().is_empty() {
                    return None;
                }
                Some((name, arguments))
            })
            .collect::<Vec<_>>();
        if !fallback.is_empty() {
            return fallback;
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

pub fn parse_deepseek_sse_line(line: &str) -> Result<DeepSeekStreamDelta, String> {
    Ok(parse_deepseek_sse_line_all(line)?
        .into_iter()
        .next()
        .unwrap_or(DeepSeekStreamDelta::Ignored))
}

pub fn parse_deepseek_sse_line_all(line: &str) -> Result<Vec<DeepSeekStreamDelta>, String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(':') {
        return Ok(vec![DeepSeekStreamDelta::Ignored]);
    }
    let payload = trimmed
        .strip_prefix("data:")
        .map(str::trim)
        .unwrap_or(trimmed);
    if payload == "[DONE]" {
        return Ok(vec![DeepSeekStreamDelta::Done]);
    }
    if payload.contains("\"message_stop\"") {
        return Ok(vec![DeepSeekStreamDelta::Done]);
    }
    if payload.contains("\"thinking_delta\"") || payload.contains("\"type\":\"thinking\"") {
        let raw_delta = extract_json_string(payload, "thinking").unwrap_or_default();
        return Ok(vec![DeepSeekStreamDelta::Reasoning {
            sanitized_delta: sanitize_reasoning(&raw_delta),
            raw_delta,
        }]);
    }
    if payload.contains("\"text_delta\"") {
        return Ok(vec![DeepSeekStreamDelta::Content {
            delta: extract_json_string(payload, "text").unwrap_or_default(),
        }]);
    }
    if payload.contains("\"type\":\"tool_use\"") {
        return Ok(vec![DeepSeekStreamDelta::ToolCall {
            index: extract_json_u64(payload, "index").map(|value| value as usize),
            id: extract_json_string(payload, "id"),
            name: extract_json_string(payload, "name").unwrap_or_default(),
            arguments_fragment: extract_json_object(payload, "input")
                .filter(|value| value.trim() != "{}")
                .unwrap_or_default(),
        }]);
    }
    if payload.contains("\"input_json_delta\"") {
        return Ok(vec![DeepSeekStreamDelta::ToolCall {
            index: extract_json_u64(payload, "index").map(|value| value as usize),
            id: None,
            name: String::new(),
            arguments_fragment: extract_json_string(payload, "partial_json").unwrap_or_default(),
        }]);
    }
    let mut deltas = Vec::new();
    if let Some(reason) = extract_json_string(payload, "finish_reason")
        .or_else(|| extract_json_string(payload, "stop_reason"))
        .filter(|value| !value.trim().is_empty())
    {
        deltas.push(DeepSeekStreamDelta::StopReason(reason));
    }
    if payload.contains("\"reasoning_content\"") {
        if let Some(raw_delta) =
            extract_json_string(payload, "reasoning_content").filter(|value| !value.is_empty())
        {
            deltas.push(DeepSeekStreamDelta::Reasoning {
                sanitized_delta: sanitize_reasoning(&raw_delta),
                raw_delta,
            });
        }
    }
    if payload.contains("\"content\"") {
        let content = extract_json_string(payload, "content")
            .or_else(|| extract_json_string(payload, "text"))
            .unwrap_or_default();
        if !content.is_empty() {
            deltas.push(DeepSeekStreamDelta::Content { delta: content });
        }
    }
    if payload.contains("\"tool_calls\"") {
        deltas.extend(parse_openai_tool_call_deltas(payload));
    }
    if payload.contains("\"usage\"") {
        deltas.push(DeepSeekStreamDelta::Telemetry(DeepSeekStreamTelemetry {
            prompt_tokens: extract_json_u64(payload, "prompt_tokens")
                .or_else(|| extract_json_u64(payload, "input_tokens")),
            completion_tokens: extract_json_u64(payload, "completion_tokens")
                .or_else(|| extract_json_u64(payload, "output_tokens")),
            reasoning_tokens: extract_json_u64(payload, "reasoning_tokens"),
            prompt_cache_hit_tokens: extract_json_u64(payload, "prompt_cache_hit_tokens")
                .or_else(|| extract_json_u64(payload, "cache_read_input_tokens")),
            prompt_cache_miss_tokens: extract_json_u64(payload, "prompt_cache_miss_tokens")
                .or_else(|| extract_json_u64(payload, "cache_creation_input_tokens")),
        }));
    }
    if deltas.is_empty() {
        Ok(vec![DeepSeekStreamDelta::Ignored])
    } else {
        Ok(deltas)
    }
}

fn parse_openai_tool_call_deltas(payload: &str) -> Vec<DeepSeekStreamDelta> {
    let Some(array) = extract_json_array(payload, "tool_calls") else {
        return Vec::new();
    };
    let objects = match split_top_level_json_objects(&array) {
        Ok(objects) => objects,
        Err(_e) => {
            // Report malformed tool_calls array instead of silently ignoring it.
            // The error is deliberately surfaced as a telemetry-style log so that
            // the runtime observability layer can surface it without panicking.
            eprintln!("deepseek_stream: {_e}");
            return Vec::new();
        }
    };
    objects
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
            Some(DeepSeekStreamDelta::ToolCall {
                index,
                id,
                name,
                arguments_fragment,
            })
        })
        .collect()
}

pub fn assemble_deepseek_sse_lines(lines: &[&str]) -> Result<DeepSeekStreamAssembly, String> {
    let mut assembly = DeepSeekStreamAssembly::default();
    for line in lines {
        for delta in parse_deepseek_sse_line_all(line)? {
            assembly.apply(delta);
        }
    }
    Ok(assembly)
}

fn merge_telemetry(target: &mut DeepSeekStreamTelemetry, next: DeepSeekStreamTelemetry) {
    if next.prompt_tokens.is_some() {
        target.prompt_tokens = next.prompt_tokens;
    }
    if next.completion_tokens.is_some() {
        target.completion_tokens = next.completion_tokens;
    }
    if next.reasoning_tokens.is_some() {
        target.reasoning_tokens = next.reasoning_tokens;
    }
    if next.prompt_cache_hit_tokens.is_some() {
        target.prompt_cache_hit_tokens = next.prompt_cache_hit_tokens;
    }
    if next.prompt_cache_miss_tokens.is_some() {
        target.prompt_cache_miss_tokens = next.prompt_cache_miss_tokens;
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
    let mut uni_value = 0u32;
    let mut uni_digits = 0u8;
    for ch in rest.chars() {
        if uni_digits > 0 {
            if let Some(digit) = ch.to_digit(16) {
                uni_value = (uni_value << 4) | digit;
                uni_digits += 1;
                if uni_digits == 5 {
                    // consumed 4 hex digits
                    if let Some(c) = char::from_u32(uni_value) {
                        output.push(c);
                    }
                    uni_digits = 0;
                    uni_value = 0;
                }
            } else {
                // non-hex char after \\u: treat as bad escape, stop unicode mode
                uni_digits = 0;
                uni_value = 0;
                if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    return Some(output);
                } else {
                    output.push(ch);
                }
            }
        } else if escaped {
            match ch {
                'n' => output.push('\n'),
                't' => output.push('\t'),
                '"' => output.push('"'),
                '\\' => output.push('\\'),
                'u' => {
                    uni_value = 0;
                    uni_digits = 1;
                }
                other => output.push(other),
            }
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

fn extract_json_object(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let rest = input[start..].trim_start();
    if !rest.starts_with('{') {
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
            '{' => depth += 1,
            '}' => {
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

fn split_top_level_json_objects(array_json: &str) -> Result<Vec<String>, String> {
    let trimmed = array_json.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return Err(format!(
            "malformed JSON array: input {:.80} does not start/end with brackets",
            trimmed
        ));
    }
    let inner = &trimmed[1..trimmed.len() - 1];
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
    Ok(objects)
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn separates_reasoning_content_from_visible_content() {
        let assembly = assemble_deepseek_sse_lines(&[
            r#"data: {"choices":[{"delta":{"reasoning_content":"Need sk-testsecret from .env"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"Visible answer"}}]}"#,
            "data: [DONE]",
        ])
        .unwrap();
        assert_eq!(
            assembly.reasoning_sanitized,
            "Need [REDACTED_SECRET] from [REDACTED_PATH]"
        );
        assert_eq!(
            assembly.reasoning_raw_volatile,
            "Need sk-testsecret from .env"
        );
        assert_eq!(assembly.content, "Visible answer");
        assert!(assembly.done);
    }

    #[test]
    fn openai_chunk_with_null_reasoning_still_records_visible_content() {
        let assembly = assemble_deepseek_sse_lines(&[
            r#"data: {"choices":[{"delta":{"content":"Visible","reasoning_content":null}}],"usage":null}"#,
            r#"data: {"choices":[{"delta":{"content":" answer","reasoning_content":null}}],"usage":null}"#,
            r#"data: {"choices":[],"usage":{"prompt_tokens":19,"completion_tokens":69,"reasoning_tokens":56,"prompt_cache_hit_tokens":0,"prompt_cache_miss_tokens":19}}"#,
            "data: [DONE]",
        ])
        .unwrap();
        assert_eq!(assembly.reasoning_sanitized, "");
        assert_eq!(assembly.reasoning_raw_volatile, "");
        assert_eq!(assembly.content, "Visible answer");
        assert_eq!(assembly.telemetry.prompt_tokens, Some(19));
        assert_eq!(assembly.telemetry.completion_tokens, Some(69));
        assert_eq!(assembly.telemetry.reasoning_tokens, Some(56));
        assert!(assembly.done);
    }

    #[test]
    fn assembles_tool_call_delta_independently() {
        let assembly = assemble_deepseek_sse_lines(&[
            r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"name":"file.read","arguments":"{\"path\":"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"\"src/parser.ts\"}"}}]}}]}"#,
        ])
        .unwrap();
        assert_eq!(assembly.tool_name.as_deref(), Some("file.read"));
        assert_eq!(assembly.tool_arguments, "{\"path\":\"src/parser.ts\"}");
        assert_eq!(assembly.content, "");
    }

    #[test]
    fn assembles_multiple_openai_tool_calls_independently() {
        let assembly = assemble_deepseek_sse_lines(&[
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"file_read","arguments":"{\"path\":\"README.md\"}"}},{"index":1,"id":"call_2","function":{"name":"file_read","arguments":"{\"path\":\"Cargo.toml\"}"}}]}}]}"#,
        ])
        .unwrap();
        let pairs = assembly.tool_call_pairs();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].0, "file_read");
        assert_eq!(pairs[0].1, "{\"path\":\"README.md\"}");
        assert_eq!(pairs[1].0, "file_read");
        assert_eq!(pairs[1].1, "{\"path\":\"Cargo.toml\"}");
    }

    #[test]
    fn captures_usage_and_prefix_cache_telemetry() {
        let assembly = assemble_deepseek_sse_lines(&[
            r#"data: {"usage":{"prompt_tokens":100,"completion_tokens":20,"reasoning_tokens":15,"prompt_cache_hit_tokens":80,"prompt_cache_miss_tokens":20}}"#,
        ])
        .unwrap();
        assert_eq!(assembly.telemetry.prompt_tokens, Some(100));
        assert_eq!(assembly.telemetry.reasoning_tokens, Some(15));
        assert_eq!(assembly.telemetry.prompt_cache_hit_tokens, Some(80));
        assert_eq!(assembly.telemetry.prompt_cache_miss_tokens, Some(20));
    }

    #[test]
    fn parses_deepseek_anthropic_sse_thinking_text_usage_and_stop() {
        let assembly = assemble_deepseek_sse_lines(&[
            r#"event: message_start"#,
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":100,"output_tokens":1,"cache_creation_input_tokens":20,"cache_read_input_tokens":80}}}"#,
            r#"event: content_block_start"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"Need sk-testsecret from .env","signature":"sig"}}"#,
            r#"event: content_block_delta"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":" then inspect"}}"#,
            r#"event: content_block_delta"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Visible answer"}}"#,
            r#"event: message_delta"#,
            r#"data: {"type":"message_delta","usage":{"output_tokens":12}}"#,
            r#"event: message_stop"#,
            r#"data: {"type":"message_stop"}"#,
        ])
        .unwrap();
        assert!(assembly.reasoning_sanitized.contains("[REDACTED_SECRET]"));
        assert!(assembly.reasoning_sanitized.contains("[REDACTED_PATH]"));
        assert_eq!(
            assembly.reasoning_raw_volatile,
            "Need sk-testsecret from .env then inspect"
        );
        assert_eq!(assembly.content, "Visible answer");
        assert_eq!(assembly.telemetry.prompt_tokens, Some(100));
        assert_eq!(assembly.telemetry.completion_tokens, Some(12));
        assert_eq!(assembly.telemetry.prompt_cache_hit_tokens, Some(80));
        assert_eq!(assembly.telemetry.prompt_cache_miss_tokens, Some(20));
        assert!(assembly.done);
    }

    #[test]
    fn parses_deepseek_anthropic_tool_use_input_delta() {
        let assembly = assemble_deepseek_sse_lines(&[
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"file.read","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"src/parser.ts\"}"}}"#,
        ])
        .unwrap();
        assert_eq!(assembly.tool_name.as_deref(), Some("file.read"));
        assert_eq!(assembly.tool_arguments, "{\"path\":\"src/parser.ts\"}");
    }

    #[test]
    fn parses_deepseek_anthropic_tool_use_full_input_on_start() {
        let assembly = assemble_deepseek_sse_lines(&[
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"file_write","input":{"path":"demo.html","content":"<html>OK</html>"}}}"#,
        ])
        .unwrap();
        assert_eq!(assembly.tool_name.as_deref(), Some("file_write"));
        assert_eq!(
            assembly.tool_arguments,
            "{\"path\":\"demo.html\",\"content\":\"<html>OK</html>\"}"
        );
    }

    #[test]
    fn parses_multiple_deepseek_anthropic_tool_use_blocks() {
        let assembly = assemble_deepseek_sse_lines(&[
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"file_read","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"README.md\"}"}}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_2","name":"file_read","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"Cargo.toml\"}"}}"#,
        ])
        .unwrap();
        let pairs = assembly.tool_call_pairs();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].1, "{\"path\":\"README.md\"}");
        assert_eq!(pairs[1].1, "{\"path\":\"Cargo.toml\"}");
    }

    #[test]
    fn parses_deepseek_anthropic_final_json_content_and_usage() {
        let assembly = assemble_deepseek_sse_lines(&[
            r#"{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"text","text":"你好！我可以帮你处理代码。"}],"usage":{"input_tokens":105,"output_tokens":252}}"#,
        ])
        .unwrap();
        assert_eq!(assembly.content, "你好！我可以帮你处理代码。");
        assert_eq!(assembly.telemetry.prompt_tokens, Some(105));
        assert_eq!(assembly.telemetry.completion_tokens, Some(252));
    }
}

use std::collections::{BTreeMap, BTreeSet};

use crate::live_http_transport::LiveHttpStreamEvent;
use crate::tcml::parser::{ParsedToolCall, ToolCallParseStatus, ToolCallSyntax};

#[derive(Debug, Default, Clone)]
pub struct DsmlChunkFilter {
    inside: bool,
    pending: String,
}

impl DsmlChunkFilter {
    pub fn filter(&mut self, chunk: &str) -> String {
        let mut output = String::new();
        let combined;
        let mut rest = if self.pending.is_empty() {
            chunk
        } else {
            self.pending.push_str(chunk);
            combined = self.pending.clone();
            self.pending.clear();
            combined.as_str()
        };
        loop {
            if self.inside {
                if let Some(end) = find_any(
                    rest,
                    &[
                        "</｜｜DSML｜｜tool_calls>",
                        "</tool_call>",
                        "<|tool_calls_section_end|>",
                    ],
                ) {
                    rest = &rest[end.1..];
                    self.inside = false;
                    continue;
                }
                let (_, pending) = split_pending_marker_prefix(
                    rest,
                    &[
                        "</｜｜DSML｜｜tool_calls>",
                        "</tool_call>",
                        "<|tool_calls_section_end|>",
                    ],
                );
                self.pending.push_str(pending);
                return output;
            }
            if let Some(start) = find_any(
                rest,
                &[
                    "<｜｜DSML｜｜tool_calls>",
                    "<tool_call>",
                    "<|tool_calls_section_begin|>",
                ],
            ) {
                output.push_str(&rest[..start.0]);
                rest = &rest[start.1..];
                self.inside = true;
                continue;
            }
            let (visible, pending) = split_pending_marker_prefix(
                rest,
                &[
                    "<｜｜DSML｜｜tool_calls>",
                    "<tool_call>",
                    "<|tool_calls_section_begin|>",
                ],
            );
            output.push_str(visible);
            self.pending.push_str(pending);
            return output;
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct StreamingToolCallAssembler {
    calls: BTreeMap<usize, StreamingToolCallState>,
    current_index: Option<usize>,
    next_index: usize,
    completed: BTreeSet<usize>,
}

#[derive(Debug, Clone, Default)]
struct StreamingToolCallState {
    provider_tool_use_id: Option<String>,
    name: Option<String>,
    arguments_json: String,
    finished: bool,
    requires_finished: bool,
}

#[derive(Debug, Clone)]
pub struct CompletedStreamingToolCall {
    pub provider_tool_use_id: String,
    pub parsed: ParsedToolCall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncompleteStreamingToolCall {
    pub index: usize,
    pub provider_tool_use_id: String,
    pub tool_id: String,
    pub arguments_json: String,
    pub reason: String,
}

impl StreamingToolCallAssembler {
    pub fn apply(&mut self, event: &LiveHttpStreamEvent) -> Vec<CompletedStreamingToolCall> {
        match event {
            LiveHttpStreamEvent::ToolCallStarted {
                index,
                id,
                name,
                input_json,
                requires_finished,
            } => {
                let index = index.unwrap_or_else(|| {
                    let next = self.next_index;
                    self.next_index += 1;
                    next
                });
                self.current_index = Some(index);
                let state = self.calls.entry(index).or_default();
                state.requires_finished = state.requires_finished || *requires_finished;
                if let Some(id) = id.as_ref().filter(|value| !value.trim().is_empty()) {
                    state.provider_tool_use_id = Some(id.clone());
                }
                if !name.trim().is_empty() {
                    state.name = Some(name.clone());
                }
                if let Some(input_json) = input_json.as_ref().filter(|value| !value.is_empty()) {
                    state.arguments_json.push_str(input_json);
                }
                self.completed_call_if_ready(index).into_iter().collect()
            }
            LiveHttpStreamEvent::ToolCallArgumentsDelta { index, delta } => {
                let index = index.or(self.current_index).unwrap_or(0);
                self.current_index = Some(index);
                let state = self.calls.entry(index).or_default();
                state.arguments_json.push_str(delta);
                self.completed_call_if_ready(index).into_iter().collect()
            }
            LiveHttpStreamEvent::ToolCallFinished { index } => {
                let index = index.or(self.current_index).unwrap_or(0);
                let state = self.calls.entry(index).or_default();
                state.finished = true;
                self.completed_call_if_ready(index).into_iter().collect()
            }
            LiveHttpStreamEvent::HttpStatus { .. }
            | LiveHttpStreamEvent::VisibleTextDelta(_)
            | LiveHttpStreamEvent::ThinkingDelta { .. }
            | LiveHttpStreamEvent::ContentBlockStarted { .. }
            | LiveHttpStreamEvent::ContentBlockFinished { .. } => Vec::new(),
        }
    }

    fn completed_call_if_ready(&mut self, index: usize) -> Option<CompletedStreamingToolCall> {
        if self.completed.contains(&index) {
            return None;
        }
        let state = self.calls.get(&index)?;
        let name = state.name.as_ref()?.trim();
        if name.is_empty() {
            return None;
        }
        if state.requires_finished && !state.finished {
            return None;
        }
        let arguments_json = if state.arguments_json.trim().is_empty() && state.finished {
            "{}".to_string()
        } else if json_object_complete(&state.arguments_json) {
            state.arguments_json.clone()
        } else {
            return None;
        };
        self.completed.insert(index);
        let provider_tool_use_id = state
            .provider_tool_use_id
            .clone()
            .unwrap_or_else(|| format!("toolu_stream_{index}"));
        Some(CompletedStreamingToolCall {
            provider_tool_use_id: provider_tool_use_id.clone(),
            parsed: ParsedToolCall {
                provider_tool_call_id: Some(provider_tool_use_id),
                tool_id: name.to_string(),
                arguments_json,
                syntax: ToolCallSyntax::NativeJson,
                status: ToolCallParseStatus::Parsed,
                repair_applied: false,
            },
        })
    }

    pub fn drain_incomplete_as_completed(
        &mut self,
        reason: &str,
    ) -> Vec<CompletedStreamingToolCall> {
        let mut drained = Vec::new();
        let indexes = self
            .calls
            .iter()
            .filter_map(|(index, state)| {
                if self.completed.contains(index) {
                    return None;
                }
                let has_partial = state
                    .name
                    .as_ref()
                    .is_some_and(|name| !name.trim().is_empty())
                    || state
                        .provider_tool_use_id
                        .as_ref()
                        .is_some_and(|id| !id.trim().is_empty())
                    || !state.arguments_json.trim().is_empty()
                    || state.finished;
                has_partial.then_some(*index)
            })
            .collect::<Vec<_>>();
        for index in indexes {
            let Some(state) = self.calls.remove(&index) else {
                continue;
            };
            self.completed.insert(index);
            let provider_tool_use_id = state
                .provider_tool_use_id
                .clone()
                .unwrap_or_else(|| format!("toolu_stream_{index}"));
            let tool_id = state
                .name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or("unknown_streaming_tool")
                .to_string();
            let arguments_json = if state.arguments_json.trim().is_empty() {
                format!(
                    "{{\"__streaming_incomplete\":true,\"reason\":{}}}",
                    json_string_local(reason)
                )
            } else {
                state.arguments_json
            };
            drained.push(CompletedStreamingToolCall {
                provider_tool_use_id: provider_tool_use_id.clone(),
                parsed: ParsedToolCall {
                    provider_tool_call_id: Some(provider_tool_use_id),
                    tool_id,
                    arguments_json,
                    syntax: ToolCallSyntax::NativeJson,
                    status: ToolCallParseStatus::Parsed,
                    repair_applied: false,
                },
            });
        }
        drained
    }
}

fn json_string_local(value: &str) -> String {
    format!("\"{}\"", escape_json_local(value))
}

fn escape_json_local(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn json_object_complete(value: &str) -> bool {
    let trimmed = value.trim();
    if !(trimmed.starts_with('{') && trimmed.ends_with('}')) {
        return false;
    }
    let mut depth = 0i64;
    let mut in_string = false;
    let mut escaped = false;
    for ch in trimmed.chars() {
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
            '}' => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return false;
        }
    }
    depth == 0 && !in_string
}

fn find_any(input: &str, needles: &[&str]) -> Option<(usize, usize)> {
    needles
        .iter()
        .filter_map(|needle| {
            input
                .find(needle)
                .map(|start| (start, start + needle.len()))
        })
        .min_by_key(|(start, _)| *start)
}

fn split_pending_marker_prefix<'a>(input: &'a str, markers: &[&str]) -> (&'a str, &'a str) {
    let mut pending_start = input.len();
    for (index, _) in input.char_indices() {
        let suffix = &input[index..];
        if markers
            .iter()
            .any(|marker| marker.starts_with(suffix) && suffix.len() < marker.len())
        {
            pending_start = pending_start.min(index);
        }
    }
    input.split_at(pending_start)
}

#[cfg(test)]
mod processor_tests {
    use super::*;

    #[test]
    fn dsml_filter_hides_cross_chunk_content() {
        let mut filter = DsmlChunkFilter::default();
        assert_eq!(filter.filter("visible <tool_call>secret"), "visible ");
        assert_eq!(filter.filter(" still secret"), "");
        assert_eq!(filter.filter("</tool_call> done"), " done");
    }

    #[test]
    fn dsml_filter_buffers_split_markers() {
        let mut filter = DsmlChunkFilter::default();
        assert_eq!(filter.filter("visible <too"), "visible ");
        assert_eq!(filter.filter("l_call>secret</too"), "");
        assert_eq!(filter.filter("l_call> done"), " done");
    }

    #[test]
    fn streaming_tool_assembler_completes_split_json_arguments() {
        let mut assembler = StreamingToolCallAssembler::default();
        assert!(assembler
            .apply(&LiveHttpStreamEvent::ToolCallStarted {
                index: Some(0),
                id: Some("call_1".to_string()),
                name: "file.read".to_string(),
                input_json: None,
                requires_finished: true,
            })
            .is_empty());
        assert!(assembler
            .apply(&LiveHttpStreamEvent::ToolCallArgumentsDelta {
                index: Some(0),
                delta: "{\"path\":\"README".to_string(),
            })
            .is_empty());
        let completed_before_finish =
            assembler.apply(&LiveHttpStreamEvent::ToolCallArgumentsDelta {
                index: Some(0),
                delta: ".md\"}".to_string(),
            });
        assert!(completed_before_finish.is_empty());
        let completed = assembler.apply(&LiveHttpStreamEvent::ToolCallFinished { index: Some(0) });
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].provider_tool_use_id, "call_1");
        assert_eq!(completed[0].parsed.tool_id, "file.read");
        assert_eq!(
            completed[0].parsed.arguments_json,
            "{\"path\":\"README.md\"}"
        );
    }

    #[test]
    fn streaming_tool_assembler_emits_empty_object_on_finish() {
        let mut assembler = StreamingToolCallAssembler::default();
        assert!(assembler
            .apply(&LiveHttpStreamEvent::ToolCallStarted {
                index: None,
                id: None,
                name: "git.status".to_string(),
                input_json: None,
                requires_finished: true,
            })
            .is_empty());
        let completed = assembler.apply(&LiveHttpStreamEvent::ToolCallFinished { index: None });
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].provider_tool_use_id, "toolu_stream_0");
        assert_eq!(completed[0].parsed.arguments_json, "{}");
    }
}
