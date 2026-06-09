//! Native non-stream response normalization.
//!
//! This module is deliberately transport-free. It converts recorded or live
//! provider JSON payloads into the normalized fields consumed by
//! `provider_response_adapter`, so live DeepSeek/Qwen calls cannot invent a
//! separate persistence path.

use crate::native_profile::deepseek::reasoning::sanitize_reasoning;
use crate::provider_response_adapter::NativeProviderStreamKind;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedNativeProviderResponse {
    pub provider: NativeProviderStreamKind,
    pub visible_content: String,
    /// DSML-formatted tool calls extracted from the raw provider payload,
    /// suitable for downstream `parse_tool_calls`. Empty string when no
    /// tool calls are present.
    pub tool_calls_dsml: String,
    pub hidden_reasoning_sanitized: Option<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    pub prompt_cache_hit_tokens: u64,
    pub prompt_cache_miss_tokens: u64,
}

/// Extract all text blocks from a DeepSeek Anthropic content array and
/// join them with newlines.
fn extract_deepseek_anthropic_text_blocks(payload: &str) -> String {
    let mut texts = Vec::new();
    let mut cursor = 0usize;
    let needle = "\"type\":\"text\"";
    while let Some(pos) = payload[cursor..].find(needle) {
        let start = cursor + pos;
        if let Some(text_val) = extract_json_string_at(&payload[start..], "text") {
            if !text_val.is_empty() {
                texts.push(text_val);
            }
        }
        cursor = start + needle.len();
    }
    texts.join("\n")
}

/// Extract tool_use blocks from a DeepSeek Anthropic content array and
/// convert them to DSML tool-call markup.
fn extract_deepseek_anthropic_tool_uses_dsml(payload: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(payload) {
        let mut parts = Vec::new();
        collect_anthropic_tool_uses_dsml(&value, &mut parts);
        if !parts.is_empty() {
            return format!(
                "<｜｜DSML｜｜tool_calls>{}</｜｜DSML｜｜tool_calls>",
                parts.join("")
            );
        }
    }

    let mut dsml_parts = Vec::new();
    let mut cursor = 0usize;
    let needle = "\"type\":\"tool_use\"";
    while let Some(pos) = payload[cursor..].find(needle) {
        let start = cursor + pos;
        let rest = &payload[start..];
        let tool_id = extract_json_string_at(rest, "id").unwrap_or_default();
        let tool_name = extract_json_string_at(rest, "name").unwrap_or_default();
        if tool_name.is_empty() {
            cursor = start + needle.len();
            continue;
        }
        // Extract the input object.
        let input_json = extract_json_object_at(rest, "input").unwrap_or_else(|| "{}".to_string());
        let params = tool_input_to_dsml_params(&tool_name, &input_json, &tool_id);
        dsml_parts.push(params);
        cursor = start + needle.len();
    }
    if dsml_parts.is_empty() {
        String::new()
    } else {
        format!(
            "<｜｜DSML｜｜tool_calls>{}</｜｜DSML｜｜tool_calls>",
            dsml_parts.join("")
        )
    }
}

/// Convert a tool-call input JSON into DSML parameter markup.
fn tool_input_to_dsml_params(tool_name: &str, input_json: &str, tool_id: &str) -> String {
    let flat_params = flatten_json_top_level_values(input_json);
    let params_str = flat_params
        .iter()
        .map(|(key, value, raw_json)| {
            format!(
                "<｜｜DSML｜｜parameter name=\"{}\" string=\"{}\">{}</｜｜DSML｜｜parameter>",
                dsml_attr_escape(key),
                if *raw_json { "false" } else { "true" },
                value
            )
        })
        .collect::<Vec<_>>()
        .join("");
    format!(
        "<｜｜DSML｜｜invoke name=\"{}\" id=\"{}\">{}</｜｜DSML｜｜invoke>",
        dsml_attr_escape(tool_name),
        dsml_attr_escape(tool_id),
        params_str
    )
}

fn collect_anthropic_tool_uses_dsml(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("tool_use") {
                if let Some(tool_name) = object.get("name").and_then(Value::as_str) {
                    let tool_id = object.get("id").and_then(Value::as_str).unwrap_or("");
                    let input_json = object
                        .get("input")
                        .map(Value::to_string)
                        .unwrap_or_else(|| "{}".to_string());
                    parts.push(tool_input_to_dsml_params(tool_name, &input_json, tool_id));
                    return;
                }
            }
            for value in object.values() {
                collect_anthropic_tool_uses_dsml(value, parts);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_anthropic_tool_uses_dsml(value, parts);
            }
        }
        _ => {}
    }
}

fn collect_openai_tool_calls_dsml(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            if let Some(Value::Array(calls)) = object.get("tool_calls") {
                for call in calls {
                    if let Some((name, input_json, call_id)) = openai_tool_call_parts(call) {
                        parts.push(tool_input_to_dsml_params(&name, &input_json, &call_id));
                    }
                }
                return;
            }
            for value in object.values() {
                collect_openai_tool_calls_dsml(value, parts);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_openai_tool_calls_dsml(value, parts);
            }
        }
        _ => {}
    }
}

fn openai_tool_call_parts(call: &Value) -> Option<(String, String, String)> {
    let object = call.as_object()?;
    let call_id = object
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(function) = object.get("function").and_then(Value::as_object) {
        let name = function.get("name").and_then(Value::as_str)?.to_string();
        let input_json = json_arguments_value_to_object_string(function.get("arguments"))?;
        return Some((name, input_json, call_id));
    }
    let name = object.get("name").and_then(Value::as_str)?.to_string();
    let input_json = json_arguments_value_to_object_string(
        object.get("arguments").or_else(|| object.get("input")),
    )?;
    Some((name, input_json, call_id))
}

fn json_arguments_value_to_object_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Object(_)) => value.map(Value::to_string),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.starts_with('{') {
                serde_json::from_str::<Value>(trimmed)
                    .ok()
                    .filter(Value::is_object)
                    .map(|value| value.to_string())
                    .or_else(|| Some(trimmed.to_string()))
            } else {
                Some("{}".to_string())
            }
        }
        Some(_) | None => Some("{}".to_string()),
    }
}

fn flatten_json_top_level_values(json: &str) -> Vec<(String, String, bool)> {
    if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(json) {
        return object
            .iter()
            .map(|(key, value)| match value {
                Value::String(text) => (key.clone(), text.clone(), false),
                other => (key.clone(), other.to_string(), true),
            })
            .collect();
    }
    flatten_json_top_level_strings(json)
        .into_iter()
        .map(|(key, value)| (key, value, false))
        .collect()
}

fn dsml_attr_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '&' => "&amp;".chars().collect::<Vec<_>>(),
            '"' => "&quot;".chars().collect(),
            '<' => "&lt;".chars().collect(),
            '>' => "&gt;".chars().collect(),
            other => vec![other],
        })
        .collect()
}

/// Flatten top-level string/number/bool values from a JSON object.
fn flatten_json_top_level_strings(json: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let trimmed = json.trim();
    let body = if trimmed.starts_with('{') && trimmed.ends_with('}') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        return pairs;
    };
    let mut cursor = 0usize;
    while cursor < body.len() {
        let rest = &body[cursor..];
        // Find a quoted key.
        let Some(key_start_rel) = rest.find('"') else {
            break;
        };
        let key_end = match rest[key_start_rel + 1..].find('"') {
            Some(rel_end) => key_start_rel + 1 + rel_end,
            None => break,
        };
        let key = &rest[key_start_rel + 1..key_end];
        let after_key = &rest[key_end + 1..];
        let colon_pos = match after_key.find(':') {
            Some(p) => p,
            None => break,
        };
        let after_colon = after_key[colon_pos + 1..].trim_start();
        let (value, consumed) = if after_colon.starts_with('"') {
            let val = decode_json_string_prefix(&after_colon[1..]).unwrap_or_default();
            let quote_end = after_colon[1..]
                .find('"')
                .map(|p| p + 2)
                .unwrap_or(after_colon.len());
            (val, quote_end)
        } else if after_colon.starts_with("true") || after_colon.starts_with("false") {
            let end = after_colon
                .find(|c: char| c == ',' || c == '}' || c.is_whitespace())
                .unwrap_or(after_colon.len());
            (after_colon[..end].to_string(), end)
        } else {
            let end = after_colon
                .find(|c: char| c == ',' || c == '}')
                .unwrap_or(after_colon.len());
            let num_str = after_colon[..end].trim().to_string();
            (num_str, end)
        };
        pairs.push((key.to_string(), value));
        cursor += key_end + 1 + colon_pos + 1 + consumed;
        // Skip commas.
        while cursor < body.len() && body.as_bytes()[cursor] == b',' {
            cursor += 1;
        }
    }
    pairs
}

fn decode_json_string_prefix(s: &str) -> Option<String> {
    let mut output = String::new();
    let mut escaped = false;
    for ch in s.chars() {
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

/// Like `extract_json_string` but searches within the given slice.
fn extract_json_string_at(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let rest = input[start..].trim_start();
    decode_json_string_prefix(rest.strip_prefix('"')?)
}

/// Extract the JSON object value for a key, returning the raw braces-delimited text.
fn extract_json_object_at(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let rest = input[start..].trim_start();
    if !rest.starts_with('{') {
        return None;
    }
    let mut brace_count = 0i32;
    let mut end = 0usize;
    for (i, ch) in rest.char_indices() {
        if ch == '{' {
            brace_count += 1;
        } else if ch == '}' {
            brace_count -= 1;
            if brace_count == 0 {
                end = i + 1;
                break;
            }
        }
    }
    if end == 0 {
        None
    } else {
        Some(rest[..end].to_string())
    }
}

/// Extract OpenAI-style tool_calls from a payload and produce DSML markup.
fn extract_openai_tool_calls_dsml(payload: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(payload) {
        let mut parts = Vec::new();
        collect_openai_tool_calls_dsml(&value, &mut parts);
        if !parts.is_empty() {
            return format!(
                "<｜｜DSML｜｜tool_calls>{}</｜｜DSML｜｜tool_calls>",
                parts.join("")
            );
        }
    }

    let array = extract_json_array_for_key_at(payload, "tool_calls");
    let array = match array {
        Some(a) => a,
        None => return String::new(),
    };
    let objects = split_json_top_level_objects(&array);
    if objects.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    for obj in &objects {
        let name = extract_json_string_at(obj, "name").unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let call_id = extract_json_string_at(obj, "id").unwrap_or_default();
        let args = extract_json_object_at(obj, "arguments")
            .or_else(|| extract_json_object_at(obj, "input"))
            .unwrap_or_else(|| "{}".to_string());
        parts.push(tool_input_to_dsml_params(&name, &args, &call_id));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(
            "<｜｜DSML｜｜tool_calls>{}</｜｜DSML｜｜tool_calls>",
            parts.join("")
        )
    }
}

fn extract_json_array_for_key_at(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let rest = input[start..].trim_start();
    if !rest.starts_with('[') {
        return None;
    }
    let mut bracket_count = 0i32;
    let mut end = 0usize;
    for (i, ch) in rest.char_indices() {
        if ch == '[' {
            bracket_count += 1;
        } else if ch == ']' {
            bracket_count -= 1;
            if bracket_count == 0 {
                end = i + 1;
                break;
            }
        }
    }
    if end == 0 {
        None
    } else {
        Some(rest[..end].to_string())
    }
}

fn split_json_top_level_objects(input: &str) -> Vec<String> {
    let trimmed = input.trim();
    let inner = if trimmed.starts_with('[') && trimmed.ends_with(']') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        return vec![trimmed.to_string()];
    };
    let mut objects = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, ch) in inner.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
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
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            if depth == 0 {
                objects.push(inner[start..=i].to_string());
                start = i + 1;
            }
        }
    }
    objects
}

pub fn normalize_deepseek_anthropic_response(
    payload: &str,
) -> Result<NormalizedNativeProviderResponse, String> {
    let visible_content = {
        let text_blocks = extract_deepseek_anthropic_text_blocks(payload);
        if text_blocks.is_empty() {
            extract_json_string(payload, "content").unwrap_or_default()
        } else {
            text_blocks
        }
    };
    let hidden_reasoning_sanitized = extract_json_string(payload, "reasoning_content")
        .or_else(|| extract_json_string(payload, "reasoning"))
        .map(|value| sanitize_reasoning(&value))
        .filter(|value| !value.is_empty());
    Ok(NormalizedNativeProviderResponse {
        provider: NativeProviderStreamKind::DeepSeek,
        visible_content,
        tool_calls_dsml: extract_deepseek_anthropic_tool_uses_dsml(payload),
        hidden_reasoning_sanitized,
        prompt_tokens: extract_json_u64(payload, "input_tokens")
            .or_else(|| extract_json_u64(payload, "prompt_tokens"))
            .unwrap_or(0),
        completion_tokens: extract_json_u64(payload, "output_tokens")
            .or_else(|| extract_json_u64(payload, "completion_tokens"))
            .unwrap_or(0),
        reasoning_tokens: extract_json_u64(payload, "reasoning_tokens").unwrap_or(0),
        prompt_cache_hit_tokens: extract_json_u64(payload, "cache_read_input_tokens")
            .or_else(|| extract_json_u64(payload, "prompt_cache_hit_tokens"))
            .unwrap_or(0),
        prompt_cache_miss_tokens: extract_json_u64(payload, "cache_creation_input_tokens")
            .or_else(|| extract_json_u64(payload, "prompt_cache_miss_tokens"))
            .unwrap_or(0),
    })
}

pub fn normalize_deepseek_openai_response(
    payload: &str,
) -> Result<NormalizedNativeProviderResponse, String> {
    let visible_content = extract_json_string(payload, "content").unwrap_or_default();
    let hidden_reasoning_sanitized = extract_json_string(payload, "reasoning_content")
        .or_else(|| extract_json_string(payload, "reasoning"))
        .map(|value| sanitize_reasoning(&value))
        .filter(|value| !value.is_empty());
    Ok(NormalizedNativeProviderResponse {
        provider: NativeProviderStreamKind::DeepSeek,
        visible_content,
        tool_calls_dsml: extract_openai_tool_calls_dsml(payload),
        hidden_reasoning_sanitized,
        prompt_tokens: extract_json_u64(payload, "prompt_tokens")
            .or_else(|| extract_json_u64(payload, "input_tokens"))
            .unwrap_or(0),
        completion_tokens: extract_json_u64(payload, "completion_tokens")
            .or_else(|| extract_json_u64(payload, "output_tokens"))
            .unwrap_or(0),
        reasoning_tokens: extract_json_u64(payload, "reasoning_tokens").unwrap_or(0),
        prompt_cache_hit_tokens: extract_json_u64(payload, "prompt_cache_hit_tokens")
            .or_else(|| extract_json_u64(payload, "cache_read_input_tokens"))
            .unwrap_or(0),
        prompt_cache_miss_tokens: extract_json_u64(payload, "prompt_cache_miss_tokens")
            .or_else(|| extract_json_u64(payload, "cache_creation_input_tokens"))
            .unwrap_or(0),
    })
}

pub fn normalize_qwen_openai_response(
    payload: &str,
) -> Result<NormalizedNativeProviderResponse, String> {
    if let Some(model) = extract_json_string(payload, "model") {
        if !model.contains("Qwen3.6-27B") {
            return Err("Qwen native response requires Qwen3.6-27B deployment".to_string());
        }
    }
    let visible_content = extract_json_string(payload, "content").unwrap_or_default();
    let hidden_reasoning_sanitized = extract_json_string(payload, "reasoning_content")
        .or_else(|| extract_json_string(payload, "thinking"))
        .map(|value| sanitize_reasoning(&value))
        .filter(|value| !value.is_empty());
    Ok(NormalizedNativeProviderResponse {
        provider: NativeProviderStreamKind::Qwen,
        visible_content,
        tool_calls_dsml: extract_openai_tool_calls_dsml(payload),
        hidden_reasoning_sanitized,
        prompt_tokens: extract_json_u64(payload, "prompt_tokens").unwrap_or(0),
        completion_tokens: extract_json_u64(payload, "completion_tokens").unwrap_or(0),
        reasoning_tokens: extract_json_u64(payload, "reasoning_tokens").unwrap_or(0),
        prompt_cache_hit_tokens: 0,
        prompt_cache_miss_tokens: 0,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_deepseek_anthropic_message_response() {
        let response = r#"{
          "id":"msg_1",
          "model":"deepseek-v4-flash",
          "content":[{"type":"text","text":"Visible OK"}],
          "reasoning_content":"Need sk-testsecret from .env",
          "usage":{
            "input_tokens":100,
            "output_tokens":20,
            "reasoning_tokens":15,
            "cache_read_input_tokens":80,
            "cache_creation_input_tokens":20
          }
        }"#;
        let normalized = normalize_deepseek_anthropic_response(response).unwrap();
        assert_eq!(normalized.provider, NativeProviderStreamKind::DeepSeek);
        assert_eq!(normalized.visible_content, "Visible OK");
        assert_eq!(
            normalized.hidden_reasoning_sanitized.as_deref(),
            Some("Need [REDACTED_SECRET] from [REDACTED_PATH]")
        );
        assert_eq!(normalized.prompt_tokens, 100);
        assert_eq!(normalized.completion_tokens, 20);
        assert_eq!(normalized.reasoning_tokens, 15);
        assert_eq!(normalized.prompt_cache_hit_tokens, 80);
        assert_eq!(normalized.prompt_cache_miss_tokens, 20);
    }

    #[test]
    fn normalizes_deepseek_anthropic_tool_use_to_canonical_dsml() {
        let response = r#"{
          "content":[
            {"type":"text","text":"Reading."},
            {"type":"tool_use","id":"toolu_1","name":"file_read","input":{"path":"README.md","max_bytes":2000}}
          ]
        }"#;
        let normalized = normalize_deepseek_anthropic_response(response).unwrap();
        assert!(normalized
            .tool_calls_dsml
            .contains("<｜｜DSML｜｜invoke name=\"file_read\" id=\"toolu_1\">"));
        assert!(normalized
            .tool_calls_dsml
            .contains("<｜｜DSML｜｜parameter name=\"path\" string=\"true\">README.md"));
        assert!(normalized
            .tool_calls_dsml
            .contains("<｜｜DSML｜｜parameter name=\"max_bytes\" string=\"false\">2000"));
        let parsed = crate::tcml::parse_tool_calls(&normalized.tool_calls_dsml);
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            crate::tcml::normalize_tool_id(&parsed[0].tool_id),
            "file.read"
        );
        assert_eq!(
            crate::tcml::parse_tool_arguments(&parsed[0].arguments_json)
                .path
                .as_deref(),
            Some("README.md")
        );
    }

    #[test]
    fn normalizes_qwen_openai_message_response_and_rejects_wrong_model() {
        let response = r#"{
          "model":"Qwen/Qwen3.6-27B",
          "choices":[{"message":{"reasoning_content":"Need sk-testsecret from .env","content":"Patch ready"}}],
          "usage":{"prompt_tokens":90,"completion_tokens":18,"reasoning_tokens":7}
        }"#;
        let normalized = normalize_qwen_openai_response(response).unwrap();
        assert_eq!(normalized.provider, NativeProviderStreamKind::Qwen);
        assert_eq!(normalized.visible_content, "Patch ready");
        assert_eq!(
            normalized.hidden_reasoning_sanitized.as_deref(),
            Some("Need [REDACTED_SECRET] from [REDACTED_PATH]")
        );
        assert_eq!(normalized.prompt_tokens, 90);
        assert_eq!(normalized.completion_tokens, 18);
        assert_eq!(normalized.reasoning_tokens, 7);

        let wrong = normalize_qwen_openai_response(r#"{"model":"Qwen/Qwen2-7B"}"#);
        assert!(wrong.is_err());
    }

    #[test]
    fn normalizes_deepseek_openai_tool_calls_string_arguments_to_canonical_dsml() {
        let response = r#"{
          "model":"deepseek-v4-flash",
          "choices":[{"message":{"content":"","reasoning_content":"Need sk-testsecret from .env","tool_calls":[
            {"id":"call_1","type":"function","function":{"name":"write_file","arguments":"{\"path\":\"generated.html\",\"content\":\"<html>ok</html>\"}"}}
          ]}}],
          "usage":{"prompt_tokens":9,"completion_tokens":3,"reasoning_tokens":2,"prompt_cache_hit_tokens":4,"prompt_cache_miss_tokens":5}
        }"#;
        let normalized = normalize_deepseek_openai_response(response).unwrap();
        assert_eq!(normalized.provider, NativeProviderStreamKind::DeepSeek);
        assert_eq!(
            normalized.hidden_reasoning_sanitized.as_deref(),
            Some("Need [REDACTED_SECRET] from [REDACTED_PATH]")
        );
        assert_eq!(normalized.prompt_tokens, 9);
        assert_eq!(normalized.completion_tokens, 3);
        assert_eq!(normalized.reasoning_tokens, 2);
        assert_eq!(normalized.prompt_cache_hit_tokens, 4);
        assert_eq!(normalized.prompt_cache_miss_tokens, 5);
        assert!(normalized
            .tool_calls_dsml
            .contains("<｜｜DSML｜｜invoke name=\"write_file\" id=\"call_1\">"));
        let parsed = crate::tcml::parse_tool_calls(&normalized.tool_calls_dsml);
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            crate::tcml::normalize_tool_id(&parsed[0].tool_id),
            "file.write"
        );
        let args = crate::tcml::parse_tool_arguments(&parsed[0].arguments_json);
        assert_eq!(args.path.as_deref(), Some("generated.html"));
        assert_eq!(args.content.as_deref(), Some("<html>ok</html>"));
    }

    #[test]
    fn normalizes_qwen_openai_tool_calls_string_arguments_to_canonical_dsml() {
        let response = r#"{
          "model":"Qwen/Qwen3.6-27B",
          "choices":[{"message":{"content":"","tool_calls":[
            {"id":"call_1","type":"function","function":{"name":"write_file","arguments":"{\"path\":\"generated.html\",\"content\":\"<html>ok</html>\"}"}}
          ]}}],
          "usage":{"prompt_tokens":9,"completion_tokens":3}
        }"#;
        let normalized = normalize_qwen_openai_response(response).unwrap();
        assert!(normalized
            .tool_calls_dsml
            .contains("<｜｜DSML｜｜invoke name=\"write_file\" id=\"call_1\">"));
        assert!(normalized
            .tool_calls_dsml
            .contains("<｜｜DSML｜｜parameter name=\"path\" string=\"true\">generated.html"));
        assert!(normalized
            .tool_calls_dsml
            .contains("<｜｜DSML｜｜parameter name=\"content\" string=\"true\"><html>ok</html>"));
        let parsed = crate::tcml::parse_tool_calls(&normalized.tool_calls_dsml);
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            crate::tcml::normalize_tool_id(&parsed[0].tool_id),
            "file.write"
        );
        let args = crate::tcml::parse_tool_arguments(&parsed[0].arguments_json);
        assert_eq!(args.path.as_deref(), Some("generated.html"));
        assert_eq!(args.content.as_deref(), Some("<html>ok</html>"));
    }
}
