//! Compatible-provider request transform boundary.
//!
//! DeepSeek and Qwen keep native adapters. This module is deliberately generic:
//! it converts a validated compatible provider config into a prepared HTTP
//! request shape, but it does not provide native prompts, native parsers, native
//! eval promotion, or model-specific compensation.

use crate::live_model_request::{ModelRequestMessage, PreparedModelHttpRequest};
use researchcode_kernel::model::{CompatibleProviderConfig, OptimizationLevel};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibleProviderRequest {
    pub provider: CompatibleProviderConfig,
    pub messages: Vec<ModelRequestMessage>,
    pub max_tokens: u64,
    pub stream: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibleProviderResponse {
    pub visible_text: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub parser_profile: String,
}

pub fn build_compatible_provider_request(
    request: &CompatibleProviderRequest,
) -> Result<PreparedModelHttpRequest, String> {
    request.provider.validate()?;
    if request.provider.optimization_level == OptimizationLevel::Native {
        return Err("compatible provider cannot use native optimization".to_string());
    }
    if request.messages.is_empty() {
        return Err("at least one message is required".to_string());
    }
    let authorization_env = request
        .provider
        .api_key_env
        .clone()
        .ok_or_else(|| "compatible provider v0 requires api_key_env".to_string())?;
    let body_json = match request.provider.protocol.as_str() {
        "openai_compatible" => openai_body_json(
            &request.provider.actual_model_name,
            &request.messages,
            request.max_tokens,
            request.stream,
        ),
        "anthropic_compatible" => anthropic_body_json(
            &request.provider.actual_model_name,
            &request.messages,
            request.max_tokens,
            request.stream,
        )?,
        "custom" => {
            if request.provider.request_transform_id.as_deref() != Some("custom_passthrough_v0") {
                return Err(
                    "custom provider requires request_transform_id=custom_passthrough_v0"
                        .to_string(),
                );
            }
            openai_body_json(
                &request.provider.actual_model_name,
                &request.messages,
                request.max_tokens,
                request.stream,
            )
        }
        _ => return Err("unsupported compatible provider protocol".to_string()),
    };
    Ok(PreparedModelHttpRequest {
        method: "POST".to_string(),
        url: compatible_target_url(&request.provider),
        authorization_env,
        body_json,
        stream: request.stream,
    })
}

pub fn normalize_compatible_provider_response(
    protocol: &str,
    response_body: &str,
) -> Result<CompatibleProviderResponse, String> {
    let visible_text = match protocol {
        "openai_compatible" | "custom" => extract_json_string(response_body, "content")
            .ok_or_else(|| "compatible response missing content".to_string())?,
        "anthropic_compatible" => extract_json_string(response_body, "text")
            .ok_or_else(|| "compatible response missing text".to_string())?,
        _ => return Err("unsupported compatible provider protocol".to_string()),
    };
    Ok(CompatibleProviderResponse {
        visible_text,
        prompt_tokens: extract_json_number(response_body, "prompt_tokens")
            .or_else(|| extract_json_number(response_body, "input_tokens"))
            .unwrap_or(0),
        completion_tokens: extract_json_number(response_body, "completion_tokens")
            .or_else(|| extract_json_number(response_body, "output_tokens"))
            .unwrap_or(0),
        parser_profile: "compatible_generic_parser".to_string(),
    })
}

fn compatible_target_url(provider: &CompatibleProviderConfig) -> String {
    let trimmed = provider.base_url.trim_end_matches('/');
    match provider.protocol.as_str() {
        "openai_compatible" if trimmed.ends_with("/v1") => {
            format!("{trimmed}/chat/completions")
        }
        "anthropic_compatible" if trimmed.ends_with("/anthropic") => {
            format!("{trimmed}/v1/messages")
        }
        "anthropic_compatible" if trimmed.ends_with("/v1") => format!("{trimmed}/messages"),
        _ => trimmed.to_string(),
    }
}

fn anthropic_body_json(
    model: &str,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
) -> Result<String, String> {
    let system = messages
        .iter()
        .filter(|message| message.role == "system")
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut body = Map::new();
    body.insert("model".to_string(), Value::String(model.to_string()));
    body.insert(
        "max_tokens".to_string(),
        Value::Number(serde_json::Number::from(max_tokens)),
    );
    body.insert("stream".to_string(), Value::Bool(stream));
    if !system.is_empty() {
        body.insert("system".to_string(), Value::String(system));
    }
    let mut projected_messages = Vec::<Value>::new();
    for message in messages.iter().filter(|message| message.role != "system") {
        projected_messages.push(anthropic_message_json(message)?);
    }
    body.insert("messages".to_string(), Value::Array(projected_messages));
    serde_json::to_string(&Value::Object(body))
        .map_err(|error| format!("serialize anthropic request: {error}"))
}

fn anthropic_message_json(message: &ModelRequestMessage) -> Result<Value, String> {
    match message.role.as_str() {
        "assistant" => Ok(json!({
            "role": "assistant",
            "content": anthropic_assistant_content_blocks(&message.content)?,
        })),
        "tool" => Ok(json!({
            "role": "user",
            "content": [anthropic_tool_result_block(&message.content)?],
        })),
        "user" => Ok(json!({
            "role": "user",
            "content": [{"type": "text", "text": message.content}],
        })),
        other => Err(format!("unsupported anthropic-compatible role: {other}")),
    }
}

fn anthropic_assistant_content_blocks(content: &str) -> Result<Vec<Value>, String> {
    let Ok(value) = serde_json::from_str::<Value>(content) else {
        return Ok(vec![json!({"type": "text", "text": content})]);
    };
    let Some(object) = value.as_object() else {
        return Ok(vec![json!({"type": "text", "text": content})]);
    };
    let mut blocks = Vec::new();
    if let Some(text) = object
        .get("content")
        .and_then(|value| value.as_str())
        .filter(|text| !text.is_empty())
    {
        blocks.push(json!({"type": "text", "text": text}));
    }
    if let Some(tool_calls) = object.get("tool_calls").and_then(|value| value.as_array()) {
        for tool_call in tool_calls {
            let id = tool_call
                .get("id")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    "anthropic-compatible assistant tool_call requires id".to_string()
                })?;
            let function = tool_call
                .get("function")
                .and_then(|value| value.as_object())
                .ok_or_else(|| {
                    "anthropic-compatible assistant tool_call requires function".to_string()
                })?;
            let name = function
                .get("name")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    "anthropic-compatible assistant tool_call function requires name".to_string()
                })?;
            let arguments = function
                .get("arguments")
                .and_then(|value| value.as_str())
                .unwrap_or("{}");
            let arguments = serde_json::from_str::<Value>(arguments)
                .map_err(|error| format!("parse assistant tool_call arguments: {error}"))?;
            blocks.push(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": arguments,
            }));
        }
    }
    if blocks.is_empty() {
        blocks.push(json!({"type": "text", "text": content}));
    }
    Ok(blocks)
}

fn anthropic_tool_result_block(content: &str) -> Result<Value, String> {
    let parsed = serde_json::from_str::<Value>(content)
        .map_err(|error| format!("parse anthropic-compatible tool result envelope: {error}"))?;
    let tool_use_id = parsed
        .get("tool_call_id")
        .or_else(|| parsed.get("tool_use_id"))
        .and_then(|value| value.as_str())
        .ok_or_else(|| "anthropic-compatible tool result requires tool_call_id".to_string())?;
    let text = parsed
        .get("content")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let is_error = parsed
        .get("is_error")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    Ok(json!({
        "type": "tool_result",
        "tool_use_id": tool_use_id,
        "content": text,
        "is_error": is_error,
    }))
}

fn openai_body_json(
    model: &str,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
) -> String {
    let projected_messages = messages
        .iter()
        .map(|message| {
            json!({
                "role": message.role,
                "content": message.content,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "model": model,
        "max_tokens": max_tokens,
        "stream": stream,
        "messages": projected_messages,
    })
    .to_string()
}

fn extract_json_string(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let tail = input[start..].trim_start();
    if !tail.starts_with('"') {
        return None;
    }
    let mut result = String::new();
    let mut escaped = false;
    for character in tail[1..].chars() {
        if escaped {
            result.push(match character {
                'n' => '\n',
                't' => '\t',
                other => other,
            });
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            return Some(result);
        } else {
            result.push(character);
        }
    }
    None
}

fn extract_json_number(input: &str, key: &str) -> Option<u64> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let tail = input[start..].trim_start();
    let digits = tail
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use researchcode_kernel::model::{ProviderCapabilityHints, ProviderHealthCheck};

    fn provider(protocol: &str, base_url: &str) -> CompatibleProviderConfig {
        CompatibleProviderConfig {
            provider_id: "lab".to_string(),
            schema_version: "v0".to_string(),
            display_name: "Lab".to_string(),
            protocol: protocol.to_string(),
            base_url: base_url.to_string(),
            api_key_env: Some("LAB_API_KEY".to_string()),
            actual_model_name: "lab-model".to_string(),
            display_model_name: "Lab Model".to_string(),
            model_alias: Some("lab".to_string()),
            capability_hints: ProviderCapabilityHints::default(),
            request_transform_id: None,
            response_transform_id: None,
            health_check: ProviderHealthCheck::default(),
            enabled_by_default: false,
            optimization_level: OptimizationLevel::Compatible,
        }
    }

    #[test]
    fn builds_openai_compatible_request_without_native_optimization() {
        let request = build_compatible_provider_request(&CompatibleProviderRequest {
            provider: provider("openai_compatible", "http://127.0.0.1:8000/v1"),
            messages: vec![ModelRequestMessage {
                role: "user".to_string(),
                content: "Hello \"there\"\nsecond line".to_string(),
                cache_control_ttl: None,
            }],
            max_tokens: 32,
            stream: true,
        })
        .unwrap();
        assert_eq!(request.url, "http://127.0.0.1:8000/v1/chat/completions");
        assert_eq!(request.authorization_env, "LAB_API_KEY");
        assert!(request.body_json.contains("\"model\":\"lab-model\""));
        assert!(request.body_json.contains("\"stream\":true"));
        let body: Value = serde_json::from_str(&request.body_json).unwrap();
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(
            body["messages"][0]["content"],
            "Hello \"there\"\nsecond line"
        );
    }

    #[test]
    fn builds_anthropic_compatible_request_with_system_split() {
        let request = build_compatible_provider_request(&CompatibleProviderRequest {
            provider: provider("anthropic_compatible", "https://example.test/anthropic"),
            messages: vec![
                ModelRequestMessage {
                    role: "system".to_string(),
                    content: "System".to_string(),
                    cache_control_ttl: None,
                },
                ModelRequestMessage {
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                    cache_control_ttl: None,
                },
            ],
            max_tokens: 32,
            stream: false,
        })
        .unwrap();
        assert_eq!(request.url, "https://example.test/anthropic/v1/messages");
        assert!(request.body_json.contains("\"system\":\"System\""));
        assert!(!request.body_json.contains("\"role\":\"system\""));
    }

    #[test]
    fn anthropic_compatible_projects_tool_calls_and_results_as_content_blocks() {
        let request = build_compatible_provider_request(&CompatibleProviderRequest {
            provider: provider("anthropic_compatible", "https://example.test/anthropic"),
            messages: vec![
                ModelRequestMessage {
                    role: "user".to_string(),
                    content: "Read README".to_string(),
                    cache_control_ttl: None,
                },
                ModelRequestMessage {
                    role: "assistant".to_string(),
                    content: r#"{"content":"I will read it.","tool_calls":[{"id":"call_read","function":{"name":"file_read","arguments":"{\"path\":\"README.md\"}"}}]}"#
                        .to_string(),
                    cache_control_ttl: None,
                },
                ModelRequestMessage {
                    role: "tool".to_string(),
                    content: r##"{"tool_call_id":"call_read","content":"# Demo","is_error":false}"##
                        .to_string(),
                    cache_control_ttl: None,
                },
            ],
            max_tokens: 64,
            stream: true,
        })
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&request.body_json).unwrap();
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["type"], "text");
        assert_eq!(messages[1]["content"][1]["type"], "tool_use");
        assert_eq!(messages[1]["content"][1]["id"], "call_read");
        assert_eq!(messages[1]["content"][1]["name"], "file_read");
        assert_eq!(messages[1]["content"][1]["input"]["path"], "README.md");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "call_read");
        assert_eq!(messages[2]["content"][0]["content"], "# Demo");
        assert!(!request.body_json.contains("\"role\":\"tool\""));
    }

    #[test]
    fn anthropic_compatible_rejects_tool_result_without_tool_call_id() {
        let result = build_compatible_provider_request(&CompatibleProviderRequest {
            provider: provider("anthropic_compatible", "https://example.test/anthropic"),
            messages: vec![ModelRequestMessage {
                role: "tool".to_string(),
                content: r#"{"content":"orphaned result"}"#.to_string(),
                cache_control_ttl: None,
            }],
            max_tokens: 64,
            stream: true,
        });
        assert!(
            matches!(result, Err(error) if error.contains("tool result requires tool_call_id"))
        );
    }

    #[test]
    fn custom_provider_requires_explicit_transform() {
        let result = build_compatible_provider_request(&CompatibleProviderRequest {
            provider: provider("custom", "http://127.0.0.1:9000/invoke"),
            messages: vec![ModelRequestMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
                cache_control_ttl: None,
            }],
            max_tokens: 32,
            stream: false,
        });
        assert!(matches!(result, Err(error) if error.contains("custom_passthrough_v0")));
    }

    #[test]
    fn normalizes_openai_compatible_response_generically() {
        let response = normalize_compatible_provider_response(
            "openai_compatible",
            r#"{"choices":[{"message":{"content":"Visible"}}],"usage":{"prompt_tokens":10,"completion_tokens":4}}"#,
        )
        .unwrap();
        assert_eq!(response.visible_text, "Visible");
        assert_eq!(response.prompt_tokens, 10);
        assert_eq!(response.completion_tokens, 4);
        assert_eq!(response.parser_profile, "compatible_generic_parser");
    }

    #[test]
    fn normalizes_anthropic_compatible_response_generically() {
        let response = normalize_compatible_provider_response(
            "anthropic_compatible",
            r#"{"content":[{"type":"text","text":"Visible"}],"usage":{"input_tokens":10,"output_tokens":4}}"#,
        )
        .unwrap();
        assert_eq!(response.visible_text, "Visible");
        assert_eq!(response.prompt_tokens, 10);
        assert_eq!(response.completion_tokens, 4);
    }
}
