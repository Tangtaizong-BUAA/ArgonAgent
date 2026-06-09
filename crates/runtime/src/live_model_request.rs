//! Disabled-by-default live model request builders.
//!
//! These builders create auditable request shapes only. They do not open
//! sockets, read API-key values, or execute provider calls.

use crate::native_provider::NativeProviderEndpoint;
use researchcode_kernel::model::NativeModelFamily;
use serde_json::{json, Map, Value};

/// Maximum allowed `max_tokens` value. Requests exceeding this limit are
/// rejected to prevent accidentally huge context windows (e.g. from integer
/// overflow or misconfiguration).
const MAX_TOKENS_LIMIT: u64 = 128_000;

/// Validate that `tools_json` is syntactically valid JSON before it is
/// injected into a request body string. Without this check, malformed or
/// malicious input could produce broken or attacker-controlled JSON payloads.
fn validate_tools_json(tools_json: &str) -> Result<(), String> {
    if tools_json.trim().is_empty() {
        return Err("tools_json must not be empty".to_string());
    }
    serde_json::from_str::<serde_json::Value>(tools_json)
        .map(|_| ())
        .map_err(|e| format!("tools_json is not valid JSON: {e}"))
}

/// Validate that `max_tokens` is within the allowed range.
fn validate_max_tokens(max_tokens: u64) -> Result<(), String> {
    if max_tokens == 0 {
        return Err("max_tokens must be greater than 0".to_string());
    }
    if max_tokens > MAX_TOKENS_LIMIT {
        return Err(format!(
            "max_tokens {} exceeds the limit of {}",
            max_tokens, MAX_TOKENS_LIMIT
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRequestMessage {
    pub role: String,
    pub content: String,
    pub cache_control_ttl: Option<u32>,
}

impl ModelRequestMessage {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            cache_control_ttl: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedModelHttpRequest {
    pub method: String,
    pub url: String,
    pub authorization_env: String,
    pub body_json: String,
    pub stream: bool,
}

pub fn apply_role_sampling_to_prepared_request(
    request: &mut PreparedModelHttpRequest,
    role_model_name: Option<&str>,
    temperature_milli: Option<u32>,
) -> Result<(), String> {
    if role_model_name.is_none() && temperature_milli.is_none() {
        return Ok(());
    }
    let Ok(mut body) = serde_json::from_str::<serde_json::Value>(&request.body_json) else {
        request.body_json = inject_role_sampling_fields_lossy(
            &request.body_json,
            role_model_name,
            temperature_milli,
        );
        return Ok(());
    };
    if let Some(model) = role_model_name.filter(|value| !value.trim().is_empty()) {
        body["model"] = serde_json::Value::String(model.to_string());
    }
    if let Some(temperature_milli) = temperature_milli {
        let temperature = (temperature_milli as f64) / 1000.0;
        let number = serde_json::Number::from_f64(temperature)
            .ok_or_else(|| "invalid temperature value".to_string())?;
        body["temperature"] = serde_json::Value::Number(number);
    }
    request.body_json = serde_json::to_string(&body)
        .map_err(|error| format!("failed to serialize prepared request body: {error}"))?;
    Ok(())
}

fn inject_role_sampling_fields_lossy(
    body_json: &str,
    role_model_name: Option<&str>,
    temperature_milli: Option<u32>,
) -> String {
    let mut body = body_json.to_string();
    if let Some(model) = role_model_name.filter(|value| !value.trim().is_empty()) {
        body = replace_first_json_string_field(&body, "model", model).unwrap_or(body);
    }
    if let Some(temperature_milli) = temperature_milli {
        let field = format!(",\"temperature\":{}", (temperature_milli as f64) / 1000.0);
        if !body.contains("\"temperature\"") {
            if let Some(index) = body.rfind('}') {
                body.insert_str(index, &field);
            }
        }
    }
    body
}

fn replace_first_json_string_field(input: &str, field: &str, value: &str) -> Option<String> {
    let marker = format!("\"{field}\":\"");
    let start = input.find(&marker)? + marker.len();
    let end = input[start..].find('"')? + start;
    let mut output = String::new();
    output.push_str(&input[..start]);
    output.push_str(&escape(value));
    output.push_str(&input[end..]);
    Some(output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepSeekAnthropicToolUseBlock {
    pub id: String,
    pub name: String,
    pub input_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepSeekAnthropicToolResultBlock {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwenOpenAiToolCallBlock {
    pub id: String,
    pub name: String,
    pub arguments_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwenOpenAiToolResultBlock {
    pub tool_call_id: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepSeekOpenAiToolCallBlock {
    pub id: String,
    pub name: String,
    pub arguments_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepSeekOpenAiToolResultBlock {
    pub tool_call_id: String,
    pub content: String,
}

pub fn build_deepseek_anthropic_request(
    endpoint: &NativeProviderEndpoint,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
) -> Result<PreparedModelHttpRequest, String> {
    build_deepseek_anthropic_request_inner(endpoint, messages, max_tokens, stream, None)
}

pub fn build_deepseek_anthropic_request_with_tools(
    endpoint: &NativeProviderEndpoint,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
    tools_json: &str,
) -> Result<PreparedModelHttpRequest, String> {
    if tools_json.trim().is_empty() {
        return Err("tools_json is required".to_string());
    }
    build_deepseek_anthropic_request_inner(endpoint, messages, max_tokens, stream, Some(tools_json))
}

pub fn build_deepseek_openai_request(
    endpoint: &NativeProviderEndpoint,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
) -> Result<PreparedModelHttpRequest, String> {
    build_deepseek_openai_request_inner(endpoint, messages, max_tokens, stream, None)
}

pub fn build_deepseek_openai_request_with_tools(
    endpoint: &NativeProviderEndpoint,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
    tools_json: &str,
) -> Result<PreparedModelHttpRequest, String> {
    if tools_json.trim().is_empty() {
        return Err("DeepSeek OpenAI tools_json is required when tools are enabled".to_string());
    }
    build_deepseek_openai_request_inner(endpoint, messages, max_tokens, stream, Some(tools_json))
}

#[allow(clippy::too_many_arguments)]
pub fn build_deepseek_anthropic_tool_result_request(
    endpoint: &NativeProviderEndpoint,
    system_prompt: &str,
    user_prompt: &str,
    tool_use_id: &str,
    tool_name: &str,
    tool_input_json: &str,
    tool_result_content: &str,
    max_tokens: u64,
    stream: bool,
    tools_json: &str,
) -> Result<PreparedModelHttpRequest, String> {
    // Reasoning replay is not forwarded here; callers that need to preserve
    // the assistant thinking context must use the _with_thinking variant.
    build_deepseek_anthropic_tool_result_request_inner(
        endpoint,
        system_prompt,
        user_prompt,
        tool_use_id,
        tool_name,
        tool_input_json,
        tool_result_content,
        max_tokens,
        stream,
        tools_json,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_deepseek_anthropic_tool_result_request_with_thinking(
    endpoint: &NativeProviderEndpoint,
    system_prompt: &str,
    user_prompt: &str,
    tool_use_id: &str,
    tool_name: &str,
    tool_input_json: &str,
    tool_result_content: &str,
    max_tokens: u64,
    stream: bool,
    tools_json: &str,
    thinking: &str,
    thinking_signature: Option<&str>,
) -> Result<PreparedModelHttpRequest, String> {
    build_deepseek_anthropic_tool_result_request_inner(
        endpoint,
        system_prompt,
        user_prompt,
        tool_use_id,
        tool_name,
        tool_input_json,
        tool_result_content,
        max_tokens,
        stream,
        tools_json,
        if thinking.trim().is_empty() {
            None
        } else {
            Some(thinking)
        },
        thinking_signature,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_deepseek_anthropic_multi_tool_result_request_with_thinking(
    endpoint: &NativeProviderEndpoint,
    system_prompt: &str,
    user_prompt: &str,
    tool_uses: &[DeepSeekAnthropicToolUseBlock],
    tool_results: &[DeepSeekAnthropicToolResultBlock],
    max_tokens: u64,
    stream: bool,
    tools_json: &str,
    thinking: Option<&str>,
    thinking_signature: Option<&str>,
) -> Result<PreparedModelHttpRequest, String> {
    endpoint.validate()?;
    validate_max_tokens(max_tokens)?;
    if endpoint.family != NativeModelFamily::DeepSeek {
        return Err("DeepSeek tool_result request requires a DeepSeek endpoint".to_string());
    }
    if endpoint.protocol != "anthropic_compatible" {
        return Err("DeepSeek tool_result requires anthropic_compatible transport".to_string());
    }
    if tool_uses.is_empty() || tool_results.is_empty() {
        return Err("tool_uses and tool_results are required".to_string());
    }
    if tool_uses.len() != tool_results.len() {
        return Err("each DeepSeek tool_use must have a matching tool_result".to_string());
    }
    validate_tools_json(tools_json)?;
    let tools_value = serde_json::from_str::<Value>(tools_json)
        .map_err(|error| format!("tools_json is not valid JSON: {error}"))?;
    let mut assistant_content = Vec::new();
    if let Some(thinking) = thinking.filter(|value| !value.trim().is_empty()) {
        let mut thinking_block = Map::new();
        thinking_block.insert("type".to_string(), json!("thinking"));
        thinking_block.insert("thinking".to_string(), json!(thinking));
        if let Some(signature) = thinking_signature.filter(|value| !value.trim().is_empty()) {
            thinking_block.insert("signature".to_string(), json!(signature));
        }
        assistant_content.push(Value::Object(thinking_block));
    }
    for (index, tool_use) in tool_uses.iter().enumerate() {
        if tool_use.id.trim().is_empty() || tool_use.name.trim().is_empty() {
            return Err("tool_use id and name are required".to_string());
        }
        let input = match serde_json::from_str::<Value>(&tool_use.input_json) {
            Ok(value) => value,
            Err(error) if tool_results[index].is_error => json!({
                "__researchcode_malformed_tool_input": true,
                "raw_input_json": tool_use.input_json,
                "parse_error": error.to_string(),
            }),
            Err(error) => {
                return Err(format!(
                    "tool_use input_json for {} is not valid JSON: {error}",
                    tool_use.id
                ));
            }
        };
        assistant_content.push(json!({
            "type": "tool_use",
            "id": tool_use.id,
            "name": tool_use.name,
            "input": input,
        }));
    }
    let mut user_content = Vec::new();
    for (index, tool_result) in tool_results.iter().enumerate() {
        if tool_result.tool_use_id != tool_uses[index].id {
            return Err("tool_result order must match tool_use order".to_string());
        }
        let mut block = Map::new();
        block.insert("type".to_string(), json!("tool_result"));
        block.insert("tool_use_id".to_string(), json!(tool_result.tool_use_id));
        block.insert("content".to_string(), json!(tool_result.content));
        if tool_result.is_error {
            block.insert("is_error".to_string(), json!(true));
        }
        user_content.push(Value::Object(block));
    }
    let body = json!({
        "model": endpoint.actual_model_name,
        "max_tokens": max_tokens,
        "stream": stream,
        "system": system_prompt,
        "tools": tools_value,
        "tool_choice": { "type": "auto" },
        "messages": [
            { "role": "user", "content": user_prompt },
            { "role": "assistant", "content": assistant_content },
            { "role": "user", "content": user_content },
        ],
    });
    let body_json = serde_json::to_string(&body)
        .map_err(|error| format!("failed to serialize DeepSeek Anthropic request: {error}"))?;
    Ok(PreparedModelHttpRequest {
        method: "POST".to_string(),
        url: endpoint.base_url.clone(),
        authorization_env: endpoint.api_key_env.clone(),
        body_json,
        stream,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_deepseek_anthropic_tool_result_request_inner(
    endpoint: &NativeProviderEndpoint,
    system_prompt: &str,
    user_prompt: &str,
    tool_use_id: &str,
    tool_name: &str,
    tool_input_json: &str,
    tool_result_content: &str,
    max_tokens: u64,
    stream: bool,
    tools_json: &str,
    thinking: Option<&str>,
    thinking_signature: Option<&str>,
) -> Result<PreparedModelHttpRequest, String> {
    if tool_use_id.trim().is_empty() || tool_name.trim().is_empty() {
        return Err("tool_use_id and tool_name are required".to_string());
    }
    build_deepseek_anthropic_multi_tool_result_request_with_thinking(
        endpoint,
        system_prompt,
        user_prompt,
        &[DeepSeekAnthropicToolUseBlock {
            id: tool_use_id.to_string(),
            name: tool_name.to_string(),
            input_json: tool_input_json.to_string(),
        }],
        &[DeepSeekAnthropicToolResultBlock {
            tool_use_id: tool_use_id.to_string(),
            content: tool_result_content.to_string(),
            is_error: false,
        }],
        max_tokens,
        stream,
        tools_json,
        thinking,
        thinking_signature,
    )
}

fn build_deepseek_anthropic_request_inner(
    endpoint: &NativeProviderEndpoint,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
    tools_json: Option<&str>,
) -> Result<PreparedModelHttpRequest, String> {
    endpoint.validate()?;
    if endpoint.family != NativeModelFamily::DeepSeek {
        return Err("DeepSeek request builder requires a DeepSeek endpoint".to_string());
    }
    if endpoint.protocol != "anthropic_compatible" {
        return Err(
            "DeepSeek V4 Flash endpoint must use anthropic_compatible transport".to_string(),
        );
    }
    if messages.is_empty() {
        return Err("at least one message is required".to_string());
    }
    Ok(PreparedModelHttpRequest {
        method: "POST".to_string(),
        url: endpoint.base_url.clone(),
        authorization_env: endpoint.api_key_env.clone(),
        body_json: anthropic_body_json(
            &endpoint.actual_model_name,
            messages,
            max_tokens,
            stream,
            tools_json,
        )?,
        stream,
    })
}

fn build_deepseek_openai_request_inner(
    endpoint: &NativeProviderEndpoint,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
    tools_json: Option<&str>,
) -> Result<PreparedModelHttpRequest, String> {
    endpoint.validate()?;
    if endpoint.family != NativeModelFamily::DeepSeek {
        return Err("DeepSeek OpenAI request builder requires a DeepSeek endpoint".to_string());
    }
    if endpoint.protocol != "openai_compatible" {
        return Err("DeepSeek OpenAI live path requires openai_compatible transport".to_string());
    }
    if messages.is_empty() {
        return Err("at least one message is required".to_string());
    }
    Ok(PreparedModelHttpRequest {
        method: "POST".to_string(),
        url: endpoint.base_url.clone(),
        authorization_env: endpoint.api_key_env.clone(),
        body_json: deepseek_openai_body_json(
            &endpoint.actual_model_name,
            messages,
            max_tokens,
            stream,
            tools_json,
        )?,
        stream,
    })
}

pub fn build_qwen_openai_request(
    endpoint: &NativeProviderEndpoint,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
) -> Result<PreparedModelHttpRequest, String> {
    build_qwen_openai_request_inner(endpoint, messages, max_tokens, stream, None)
}

pub fn build_qwen_openai_request_with_tools(
    endpoint: &NativeProviderEndpoint,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
    tools_json: &str,
) -> Result<PreparedModelHttpRequest, String> {
    if tools_json.trim().is_empty() {
        return Err("Qwen tools_json is required when tools are enabled".to_string());
    }
    build_qwen_openai_request_inner(endpoint, messages, max_tokens, stream, Some(tools_json))
}

fn build_qwen_openai_request_inner(
    endpoint: &NativeProviderEndpoint,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
    tools_json: Option<&str>,
) -> Result<PreparedModelHttpRequest, String> {
    endpoint.validate()?;
    if endpoint.family != NativeModelFamily::Qwen {
        return Err("Qwen request builder requires a Qwen endpoint".to_string());
    }
    if !matches!(endpoint.protocol.as_str(), "openai_compatible" | "custom") {
        return Err("Qwen endpoint must use openai_compatible or custom transport".to_string());
    }
    if !endpoint.base_url.starts_with("http://") && !endpoint.base_url.starts_with("https://") {
        return Err(
            "Qwen endpoint base_url must be resolved before building a request".to_string(),
        );
    }
    if messages.is_empty() {
        return Err("at least one message is required".to_string());
    }
    Ok(PreparedModelHttpRequest {
        method: "POST".to_string(),
        url: endpoint.base_url.clone(),
        authorization_env: endpoint.api_key_env.clone(),
        body_json: openai_body_json(
            &endpoint.actual_model_name,
            messages,
            max_tokens,
            stream,
            tools_json,
        )?,
        stream,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn build_qwen_openai_tool_result_request(
    endpoint: &NativeProviderEndpoint,
    system_prompt: &str,
    user_prompt: &str,
    tool_call_id: &str,
    tool_name: &str,
    tool_arguments_json: &str,
    tool_result_content: &str,
    max_tokens: u64,
    stream: bool,
) -> Result<PreparedModelHttpRequest, String> {
    build_qwen_openai_multi_tool_result_request(
        endpoint,
        system_prompt,
        user_prompt,
        &[QwenOpenAiToolCallBlock {
            id: tool_call_id.to_string(),
            name: tool_name.to_string(),
            arguments_json: tool_arguments_json.to_string(),
        }],
        &[QwenOpenAiToolResultBlock {
            tool_call_id: tool_call_id.to_string(),
            content: tool_result_content.to_string(),
        }],
        max_tokens,
        stream,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_deepseek_openai_multi_tool_result_request(
    endpoint: &NativeProviderEndpoint,
    system_prompt: &str,
    user_prompt: &str,
    tool_calls: &[DeepSeekOpenAiToolCallBlock],
    tool_results: &[DeepSeekOpenAiToolResultBlock],
    max_tokens: u64,
    stream: bool,
) -> Result<PreparedModelHttpRequest, String> {
    build_deepseek_openai_multi_tool_result_request_with_reasoning(
        endpoint,
        system_prompt,
        user_prompt,
        tool_calls,
        tool_results,
        max_tokens,
        stream,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_deepseek_openai_multi_tool_result_request_with_reasoning(
    endpoint: &NativeProviderEndpoint,
    system_prompt: &str,
    user_prompt: &str,
    tool_calls: &[DeepSeekOpenAiToolCallBlock],
    tool_results: &[DeepSeekOpenAiToolResultBlock],
    max_tokens: u64,
    stream: bool,
    reasoning_content: Option<&str>,
) -> Result<PreparedModelHttpRequest, String> {
    endpoint.validate()?;
    validate_max_tokens(max_tokens)?;
    if endpoint.family != NativeModelFamily::DeepSeek {
        return Err("DeepSeek OpenAI tool result request requires a DeepSeek endpoint".to_string());
    }
    if endpoint.protocol != "openai_compatible" {
        return Err("DeepSeek OpenAI tool result requires openai_compatible transport".to_string());
    }
    if tool_calls.is_empty() || tool_results.is_empty() {
        return Err("tool_calls and tool_results are required".to_string());
    }
    if tool_calls.len() != tool_results.len() {
        return Err("each DeepSeek OpenAI tool_call must have a matching tool result".to_string());
    }
    let mut assistant_tool_calls = Vec::new();
    let mut tool_messages = Vec::new();
    for (index, tool_call) in tool_calls.iter().enumerate() {
        if tool_call.id.trim().is_empty() || tool_call.name.trim().is_empty() {
            return Err("tool_call id and name are required".to_string());
        }
        if tool_results[index].tool_call_id != tool_call.id {
            return Err("tool result order must match tool_call order".to_string());
        }
        assistant_tool_calls.push(json!({
            "id": tool_call.id,
            "type": "function",
            "function": {
                "name": tool_call.name,
                "arguments": tool_call.arguments_json,
            },
        }));
        tool_messages.push(json!({
            "role": "tool",
            "tool_call_id": tool_results[index].tool_call_id,
            "content": tool_results[index].content,
        }));
    }
    let mut assistant_message = Map::new();
    assistant_message.insert("role".to_string(), Value::String("assistant".to_string()));
    assistant_message.insert("content".to_string(), Value::Null);
    if let Some(reasoning_content) = reasoning_content.filter(|value| !value.trim().is_empty()) {
        assistant_message.insert(
            "reasoning_content".to_string(),
            Value::String(reasoning_content.to_string()),
        );
    }
    assistant_message.insert("tool_calls".to_string(), Value::Array(assistant_tool_calls));
    let mut messages = vec![
        json!({
            "role": "system",
            "content": system_prompt,
        }),
        json!({
            "role": "user",
            "content": user_prompt,
        }),
        Value::Object(assistant_message),
    ];
    messages.extend(tool_messages);
    let body_json = serde_json::to_string(&json!({
        "model": endpoint.actual_model_name,
        "max_tokens": max_tokens,
        "stream": stream,
        "thinking": {
            "type": "enabled",
        },
        "reasoning_effort": "high",
        "stream_options": {
            "include_usage": true,
        },
        "messages": messages,
    }))
    .map_err(|error| format!("failed to serialize DeepSeek OpenAI tool result body: {error}"))?;
    Ok(PreparedModelHttpRequest {
        method: "POST".to_string(),
        url: endpoint.base_url.clone(),
        authorization_env: endpoint.api_key_env.clone(),
        body_json,
        stream,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn build_qwen_openai_multi_tool_result_request(
    endpoint: &NativeProviderEndpoint,
    system_prompt: &str,
    user_prompt: &str,
    tool_calls: &[QwenOpenAiToolCallBlock],
    tool_results: &[QwenOpenAiToolResultBlock],
    max_tokens: u64,
    stream: bool,
) -> Result<PreparedModelHttpRequest, String> {
    endpoint.validate()?;
    validate_max_tokens(max_tokens)?;
    if endpoint.family != NativeModelFamily::Qwen {
        return Err("Qwen tool result request requires a Qwen endpoint".to_string());
    }
    if !matches!(endpoint.protocol.as_str(), "openai_compatible" | "custom") {
        return Err("Qwen tool result requires openai_compatible or custom transport".to_string());
    }
    if tool_calls.is_empty() || tool_results.is_empty() {
        return Err("tool_calls and tool_results are required".to_string());
    }
    if tool_calls.len() != tool_results.len() {
        return Err("each Qwen tool_call must have a matching tool result".to_string());
    }
    let mut assistant_tool_calls = Vec::new();
    let mut tool_messages = Vec::new();
    for (index, tool_call) in tool_calls.iter().enumerate() {
        if tool_call.id.trim().is_empty() || tool_call.name.trim().is_empty() {
            return Err("tool_call id and name are required".to_string());
        }
        if tool_results[index].tool_call_id != tool_call.id {
            return Err("tool result order must match tool_call order".to_string());
        }
        assistant_tool_calls.push(json!({
            "id": tool_call.id,
            "type": "function",
            "function": {
                "name": tool_call.name,
                "arguments": tool_call.arguments_json,
            },
        }));
        tool_messages.push(json!({
            "role": "tool",
            "tool_call_id": tool_results[index].tool_call_id,
            "content": tool_results[index].content,
        }));
    }
    let mut messages = vec![
        json!({
            "role": "system",
            "content": system_prompt,
        }),
        json!({
            "role": "user",
            "content": user_prompt,
        }),
        json!({
            "role": "assistant",
            "content": Value::Null,
            "tool_calls": assistant_tool_calls,
        }),
    ];
    messages.extend(tool_messages);
    let body_json = serde_json::to_string(&json!({
        "model": endpoint.actual_model_name,
        "max_tokens": max_tokens,
        "stream": stream,
        "messages": messages,
    }))
    .map_err(|error| format!("failed to serialize Qwen tool result body: {error}"))?;
    Ok(PreparedModelHttpRequest {
        method: "POST".to_string(),
        url: endpoint.base_url.clone(),
        authorization_env: endpoint.api_key_env.clone(),
        body_json,
        stream,
    })
}

fn anthropic_body_json(
    model: &str,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
    tools_json: Option<&str>,
) -> Result<String, String> {
    validate_max_tokens(max_tokens)?;
    if let Some(tools_json) = tools_json {
        validate_tools_json(tools_json)?;
    }
    let system = messages
        .iter()
        .filter(|message| message.role == "system")
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut body = Map::new();
    body.insert("model".to_string(), Value::String(model.to_string()));
    body.insert("max_tokens".to_string(), Value::from(max_tokens));
    body.insert("stream".to_string(), Value::Bool(stream));
    body.insert(
        "reasoning_effort".to_string(),
        Value::String("none".to_string()),
    );
    if !system.is_empty() {
        body.insert("system".to_string(), Value::String(system));
    }
    if let Some(tools_json) = tools_json {
        let tools = serde_json::from_str::<Value>(tools_json)
            .map_err(|error| format!("tools_json is not valid JSON: {error}"))?;
        body.insert("tools".to_string(), tools);
        body.insert(
            "tool_choice".to_string(),
            json!({
                "type": "auto",
            }),
        );
    }
    let projected = projected_message_values(
        project_anthropic_history_messages(messages),
        "projected Anthropic message",
    )?;
    body.insert("messages".to_string(), Value::Array(projected));
    serde_json::to_string(&Value::Object(body))
        .map_err(|error| format!("failed to serialize Anthropic request body: {error}"))
}

fn project_anthropic_history_messages(messages: &[ModelRequestMessage]) -> Vec<String> {
    let mut output = Vec::new();
    let mut inserted_history = false;
    for message in messages.iter().filter(|message| message.role != "system") {
        if message.role == "user" && !inserted_history {
            if let Some((clean_content, history_json)) =
                split_legacy_openai_history_section(&message.content)
            {
                output.extend(anthropic_history_array_items(&history_json));
                output.push(anthropic_message_json(message, &clean_content));
                inserted_history = true;
                continue;
            }
        }
        output.push(anthropic_message_json(message, &message.content));
    }
    output
}

fn anthropic_message_json(message: &ModelRequestMessage, content: &str) -> String {
    if let Some(ttl) = message.cache_control_ttl {
        format!(
            "{{\"role\":\"{}\",\"content\":[{{\"type\":\"text\",\"text\":\"{}\",\"cache_control\":{{\"type\":\"ephemeral\",\"ttl_seconds\":{}}}}}]}}",
            escape(&message.role),
            escape(content),
            ttl
        )
    } else {
        format!(
            "{{\"role\":\"{}\",\"content\":\"{}\"}}",
            escape(&message.role),
            escape(content)
        )
    }
}

fn anthropic_history_array_items(history_json: &str) -> Vec<String> {
    let Ok(serde_json::Value::Array(items)) =
        serde_json::from_str::<serde_json::Value>(history_json)
    else {
        return Vec::new();
    };
    let mut output = Vec::new();
    for item in items {
        let role = item
            .get("role")
            .and_then(|value| value.as_str())
            .unwrap_or("assistant");
        if role == "system" {
            continue;
        }
        let anthropic_role = if role == "assistant" {
            "assistant"
        } else {
            "user"
        };
        let content = anthropic_history_content(role, &item);
        output.push(format!(
            "{{\"role\":\"{}\",\"content\":\"{}\"}}",
            escape(anthropic_role),
            escape(&content)
        ));
    }
    output
}

fn anthropic_history_content(role: &str, item: &serde_json::Value) -> String {
    let base_content = item
        .get("content")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty());
    let tool_call_id = item.get("tool_call_id").and_then(|value| value.as_str());
    let tool_call_ids = item
        .get("tool_calls")
        .and_then(|value| value.as_array())
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| call.get("id").and_then(|value| value.as_str()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    match (role, base_content, tool_call_id, tool_call_ids.is_empty()) {
        ("tool", Some(content), Some(id), _) => format!("tool_result {id}: {content}"),
        ("tool", None, Some(id), _) => format!("tool_result {id}"),
        ("assistant", Some(content), _, false) => {
            format!(
                "assistant tool_calls [{}]: {content}",
                tool_call_ids.join(",")
            )
        }
        ("assistant", None, _, false) => {
            format!("assistant tool_calls [{}]", tool_call_ids.join(","))
        }
        (_, Some(content), _, _) => content.to_string(),
        _ => "[structured tool call]".to_string(),
    }
}

fn deepseek_openai_body_json(
    model: &str,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
    tools_json: Option<&str>,
) -> Result<String, String> {
    validate_max_tokens(max_tokens)?;
    if let Some(tools_json) = tools_json {
        validate_tools_json(tools_json)?;
    }
    let mut body = Map::new();
    body.insert("model".to_string(), Value::String(model.to_string()));
    body.insert("max_tokens".to_string(), Value::from(max_tokens));
    body.insert("stream".to_string(), Value::Bool(stream));
    body.insert(
        "thinking".to_string(),
        json!({
            "type": "enabled",
        }),
    );
    body.insert(
        "reasoning_effort".to_string(),
        Value::String("high".to_string()),
    );
    if let Some(tools_json) = tools_json {
        let tools = serde_json::from_str::<Value>(tools_json)
            .map_err(|error| format!("tools_json is not valid JSON: {error}"))?;
        body.insert("tools".to_string(), tools);
        body.insert("tool_choice".to_string(), Value::String("auto".to_string()));
    }
    if stream {
        body.insert(
            "stream_options".to_string(),
            json!({
                "include_usage": true,
            }),
        );
    }
    let projected = projected_message_values(
        project_openai_history_messages(messages),
        "projected OpenAI message",
    )?;
    body.insert("messages".to_string(), Value::Array(projected));
    serde_json::to_string(&Value::Object(body))
        .map_err(|error| format!("failed to serialize DeepSeek OpenAI request body: {error}"))
}

fn openai_body_json(
    model: &str,
    messages: &[ModelRequestMessage],
    max_tokens: u64,
    stream: bool,
    tools_json: Option<&str>,
) -> Result<String, String> {
    validate_max_tokens(max_tokens)?;
    if let Some(tools_json) = tools_json {
        validate_tools_json(tools_json)?;
    }
    let mut body = Map::new();
    body.insert("model".to_string(), Value::String(model.to_string()));
    body.insert("max_tokens".to_string(), Value::from(max_tokens));
    body.insert("stream".to_string(), Value::Bool(stream));
    body.insert(
        "reasoning_effort".to_string(),
        Value::String("none".to_string()),
    );
    if let Some(tools_json) = tools_json {
        let tools = serde_json::from_str::<Value>(tools_json)
            .map_err(|error| format!("tools_json is not valid JSON: {error}"))?;
        body.insert("tools".to_string(), tools);
        body.insert("tool_choice".to_string(), Value::String("auto".to_string()));
    }
    let projected = projected_message_values(
        project_openai_history_messages(messages),
        "projected OpenAI message",
    )?;
    body.insert("messages".to_string(), Value::Array(projected));
    serde_json::to_string(&Value::Object(body))
        .map_err(|error| format!("failed to serialize OpenAI request body: {error}"))
}

fn projected_message_values(messages: Vec<String>, label: &str) -> Result<Vec<Value>, String> {
    messages
        .into_iter()
        .map(|message| {
            serde_json::from_str::<Value>(&message)
                .map_err(|error| format!("{label} is invalid JSON: {error}"))
        })
        .collect()
}

fn project_openai_history_messages(messages: &[ModelRequestMessage]) -> Vec<String> {
    let mut output = Vec::new();
    let mut inserted_history = false;
    for message in messages {
        if message.role == "user" && !inserted_history {
            if let Some((clean_content, history_json)) =
                split_legacy_openai_history_section(&message.content)
            {
                output.extend(openai_history_array_items(&history_json));
                output.push(openai_message_json(&message.role, &clean_content));
                inserted_history = true;
                continue;
            }
        }
        output.push(openai_message_json(&message.role, &message.content));
    }
    output
}

fn openai_message_json(role: &str, content: &str) -> String {
    format!(
        "{{\"role\":\"{}\",\"content\":\"{}\"}}",
        escape(role),
        escape(content)
    )
}

fn split_legacy_openai_history_section(content: &str) -> Option<(String, String)> {
    let marker = "\n\n# Conversation History (OpenAI JSON)\n";
    let marker_start = content.find(marker)?;
    let after_marker = marker_start + marker.len();
    let array_start_relative = content[after_marker..].find('[')?;
    let array_start = after_marker + array_start_relative;
    let array_end = matching_json_array_end(content, array_start)?;
    let history_json = content[array_start..=array_end].trim().to_string();
    serde_json::from_str::<serde_json::Value>(&history_json).ok()?;
    let mut clean = String::new();
    clean.push_str(content[..marker_start].trim_end());
    clean.push_str(content[array_end + 1..].trim_start_matches('\n'));
    Some((clean, history_json))
}

fn matching_json_array_end(input: &str, start: usize) -> Option<usize> {
    let mut depth = 0i64;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in input[start..].char_indices() {
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
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn openai_history_array_items(history_json: &str) -> Vec<String> {
    let Ok(serde_json::Value::Array(items)) =
        serde_json::from_str::<serde_json::Value>(history_json)
    else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| match item {
            serde_json::Value::Object(mut object) => {
                object.remove("reasoning_content");
                serde_json::to_string(&serde_json::Value::Object(object)).ok()
            }
            _ => None,
        })
        .collect()
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0c}' => escaped.push_str("\\f"),
            ch if ch.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", ch as u32));
            }
            ch => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_deepseek_anthropic_request_without_key_material() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        let request = build_deepseek_anthropic_request(
            &endpoint,
            &[
                ModelRequestMessage {
                    role: "system".to_string(),
                    content: "Native DeepSeek system prompt".to_string(),
                    cache_control_ttl: None,
                },
                ModelRequestMessage {
                    role: "user".to_string(),
                    content: "Plan the task".to_string(),
                    cache_control_ttl: None,
                },
            ],
            1024,
            true,
        )
        .unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.url, "https://api.deepseek.com/anthropic");
        assert_eq!(request.authorization_env, "DEEPSEEK_API_KEY");
        assert!(request
            .body_json
            .contains("\"model\":\"deepseek-v4-flash\""));
        assert!(request
            .body_json
            .contains("\"system\":\"Native DeepSeek system prompt\""));
        assert!(request.body_json.contains("\"stream\":true"));
        assert!(!request.body_json.contains("\"role\":\"system\""));
        assert!(!request.body_json.contains("sk-"));
    }

    #[test]
    fn request_json_escapes_control_characters_from_context() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        let request = build_deepseek_anthropic_request(
            &endpoint,
            &[ModelRequestMessage {
                role: "user".to_string(),
                content: "line1\tTabbed\rCarriage\u{0007}Bell\nNext".to_string(),
                cache_control_ttl: None,
            }],
            1024,
            true,
        )
        .unwrap();
        assert!(request.body_json.contains("\\tTabbed"));
        assert!(request.body_json.contains("\\rCarriage"));
        assert!(request.body_json.contains("\\u0007Bell"));
        assert!(!request.body_json.contains('\t'));
        assert!(!request.body_json.contains('\r'));
        assert!(!request.body_json.contains('\u{0007}'));
    }

    #[test]
    fn anthropic_builder_serializes_typed_body_with_tools_and_escaped_text() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        let request = build_deepseek_anthropic_request_with_tools(
            &endpoint,
            &[
                ModelRequestMessage::new("system", "native system\nwith \"quotes\""),
                ModelRequestMessage::new("user", "read \"README.md\"\nthen continue"),
            ],
            1024,
            true,
            file_read_tool_schema_json(),
        )
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&request.body_json).unwrap();
        assert_eq!(
            body.get("model").and_then(|value| value.as_str()),
            Some("deepseek-v4-flash")
        );
        assert_eq!(
            body.get("stream").and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            body.get("reasoning_effort")
                .and_then(|value| value.as_str()),
            Some("none")
        );
        assert_eq!(
            body.get("system").and_then(|value| value.as_str()),
            Some("native system\nwith \"quotes\"")
        );
        assert!(body
            .get("tools")
            .and_then(|value| value.as_array())
            .is_some());
        assert_eq!(
            body.pointer("/tool_choice/type")
                .and_then(|value| value.as_str()),
            Some("auto")
        );
        assert_eq!(
            body.pointer("/messages/0/content")
                .and_then(|value| value.as_str()),
            Some("read \"README.md\"\nthen continue")
        );
    }

    #[test]
    fn applies_role_sampling_to_prepared_request_body() {
        let mut request = PreparedModelHttpRequest {
            method: "POST".to_string(),
            url: "https://example.test".to_string(),
            authorization_env: "KEY".to_string(),
            body_json: r#"{"model":"deepseek-chat","messages":[]}"#.to_string(),
            stream: true,
        };
        apply_role_sampling_to_prepared_request(
            &mut request,
            Some("deepseek-chat-flash"),
            Some(200),
        )
        .unwrap();
        assert!(request
            .body_json
            .contains("\"model\":\"deepseek-chat-flash\""));
        assert!(request.body_json.contains("\"temperature\":0.2"));
    }

    #[test]
    fn openai_builder_projects_legacy_history_section_to_typed_messages() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
        let request = build_deepseek_openai_request(
            &endpoint,
            &[ModelRequestMessage::new(
                "user",
                "Current task\n\n# Conversation History (OpenAI JSON)\nThe following JSON array is authoritative.\n[{\"role\":\"assistant\",\"content\":\"prior answer\"}]\n\n# Runtime Evidence\nfresh",
            )],
            1024,
            true,
        )
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&request.body_json).unwrap();
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .unwrap();
        assert!(messages.iter().any(|message| {
            message.get("role").and_then(|value| value.as_str()) == Some("assistant")
                && message.get("content").and_then(|value| value.as_str()) == Some("prior answer")
        }));
        assert!(!request
            .body_json
            .contains("Conversation History (OpenAI JSON)"));
        assert!(messages.iter().any(|message| {
            message.get("role").and_then(|value| value.as_str()) == Some("user")
                && message
                    .get("content")
                    .and_then(|value| value.as_str())
                    .is_some_and(|content| {
                        content.contains("Current task")
                            && content.contains("# Runtime Evidence")
                            && content.contains("fresh")
                    })
        }));
    }

    #[test]
    fn openai_builder_strips_nonstandard_reasoning_from_legacy_history_messages() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
        let request = build_deepseek_openai_request(
            &endpoint,
            &[ModelRequestMessage::new(
                "user",
                "Current task\n\n# Conversation History (OpenAI JSON)\nHistory.\n[{\"role\":\"assistant\",\"content\":null,\"reasoning_content\":\"private reasoning\",\"tool_calls\":[{\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"file_read\",\"arguments\":\"{}\"}}]},{\"role\":\"tool\",\"tool_call_id\":\"call_1\",\"content\":\"tool output\"}]\n\n# Runtime Evidence\nfresh",
            )],
            1024,
            true,
        )
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&request.body_json).unwrap();
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .unwrap();
        assert!(!request.body_json.contains("reasoning_content"));
        assert!(messages.iter().any(|message| {
            message.get("role").and_then(|value| value.as_str()) == Some("assistant")
                && message.get("tool_calls").is_some()
        }));
        assert!(messages.iter().any(|message| {
            message.get("role").and_then(|value| value.as_str()) == Some("tool")
                && message.get("tool_call_id").and_then(|value| value.as_str()) == Some("call_1")
        }));
    }

    #[test]
    fn anthropic_builder_projects_legacy_history_section_to_messages() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        let request = build_deepseek_anthropic_request(
            &endpoint,
            &[ModelRequestMessage::new(
                "user",
                "Current task\n\n# Conversation History (OpenAI JSON)\nHistory.\n[{\"role\":\"assistant\",\"content\":\"prior answer\"},{\"role\":\"tool\",\"tool_call_id\":\"call_1\",\"content\":\"tool output\"}]",
            )],
            1024,
            true,
        )
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&request.body_json).unwrap();
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .unwrap();
        assert!(messages.iter().any(|message| {
            message.get("role").and_then(|value| value.as_str()) == Some("assistant")
                && message.get("content").and_then(|value| value.as_str()) == Some("prior answer")
        }));
        assert!(messages.iter().any(|message| {
            message.get("role").and_then(|value| value.as_str()) == Some("user")
                && message.get("content").and_then(|value| value.as_str())
                    == Some("tool_result call_1: tool output")
        }));
        assert!(!request
            .body_json
            .contains("Conversation History (OpenAI JSON)"));
    }

    #[test]
    fn builds_deepseek_anthropic_request_with_native_tools() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        let request = build_deepseek_anthropic_request_with_tools(
            &endpoint,
            &[ModelRequestMessage {
                role: "user".to_string(),
                content: "Read README.md".to_string(),
                cache_control_ttl: None,
            }],
            1024,
            true,
            file_read_tool_schema_json(),
        )
        .unwrap();
        assert!(request.body_json.contains("\"tools\":["));
        assert!(request.body_json.contains("\"name\":\"file_read\""));
        assert!(request
            .body_json
            .contains("\"tool_choice\":{\"type\":\"auto\"}"));
        assert!(!request.body_json.contains("sk-"));
    }

    #[test]
    fn builds_deepseek_openai_request_with_native_tools() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
        let request = build_deepseek_openai_request_with_tools(
            &endpoint,
            &[ModelRequestMessage {
                role: "user".to_string(),
                content: "Read README.md".to_string(),
                cache_control_ttl: None,
            }],
            1024,
            true,
            r#"[{"type":"function","function":{"name":"file_read","description":"Read file","parameters":{"type":"object","required":["path"],"properties":{"path":{"type":"string"}}}}}]"#,
        )
        .unwrap();
        assert_eq!(request.url, "https://api.deepseek.com");
        assert!(request
            .body_json
            .contains("\"thinking\":{\"type\":\"enabled\"}"));
        assert!(request.body_json.contains("\"reasoning_effort\":\"high\""));
        assert!(request
            .body_json
            .contains("\"stream_options\":{\"include_usage\":true}"));
        assert!(request.body_json.contains("\"tools\":["));
        assert!(request.body_json.contains("\"tool_choice\":\"auto\""));
        assert!(request.body_json.contains("\"name\":\"file_read\""));
        assert!(!request.body_json.contains("\"input_schema\""));
        assert!(!request.body_json.contains("sk-"));
    }

    #[test]
    fn deepseek_openai_builder_serializes_typed_native_body() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
        let request = build_deepseek_openai_request_with_tools(
            &endpoint,
            &[ModelRequestMessage::new(
                "user",
                "read \"README.md\"\nthen run follow-up",
            )],
            1024,
            true,
            r#"[{"type":"function","function":{"name":"file_read","description":"Read file","parameters":{"type":"object","required":["path"],"properties":{"path":{"type":"string"}}}}}]"#,
        )
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&request.body_json).unwrap();
        assert_eq!(
            body.get("model").and_then(|value| value.as_str()),
            Some("deepseek-v4-flash")
        );
        assert_eq!(
            body.pointer("/thinking/type")
                .and_then(|value| value.as_str()),
            Some("enabled")
        );
        assert_eq!(
            body.get("reasoning_effort")
                .and_then(|value| value.as_str()),
            Some("high")
        );
        assert_eq!(
            body.pointer("/stream_options/include_usage")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            body.get("tool_choice").and_then(|value| value.as_str()),
            Some("auto")
        );
        assert_eq!(
            body.pointer("/messages/0/content")
                .and_then(|value| value.as_str()),
            Some("read \"README.md\"\nthen run follow-up")
        );
    }

    #[test]
    fn deepseek_openai_tool_result_request_uses_tool_messages() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
        let request = build_deepseek_openai_multi_tool_result_request(
            &endpoint,
            "DeepSeek native OpenAI-compatible system",
            "Read README.md",
            &[DeepSeekOpenAiToolCallBlock {
                id: "call_1".to_string(),
                name: "file_read".to_string(),
                arguments_json: "{\"path\":\"README.md\"}".to_string(),
            }],
            &[DeepSeekOpenAiToolResultBlock {
                tool_call_id: "call_1".to_string(),
                content: "README content".to_string(),
            }],
            256,
            true,
        )
        .unwrap();
        assert!(request.body_json.contains("\"tool_calls\""));
        assert!(request
            .body_json
            .contains("\"thinking\":{\"type\":\"enabled\"}"));
        assert!(request.body_json.contains("\"reasoning_effort\":\"high\""));
        assert!(request
            .body_json
            .contains("\"stream_options\":{\"include_usage\":true}"));
        assert!(request.body_json.contains("\"role\":\"tool\""));
        assert!(request.body_json.contains("\"tool_call_id\":\"call_1\""));
        assert!(!request.body_json.contains("sk-"));
    }

    #[test]
    fn deepseek_openai_tool_result_request_replays_reasoning_content() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
        let request = build_deepseek_openai_multi_tool_result_request_with_reasoning(
            &endpoint,
            "DeepSeek native OpenAI-compatible system",
            "Read README.md",
            &[DeepSeekOpenAiToolCallBlock {
                id: "call_1".to_string(),
                name: "file_read".to_string(),
                arguments_json: "{\"path\":\"README.md\"}".to_string(),
            }],
            &[DeepSeekOpenAiToolResultBlock {
                tool_call_id: "call_1".to_string(),
                content: "README content".to_string(),
            }],
            256,
            true,
            Some("provider raw reasoning for this same turn"),
        )
        .unwrap();
        assert!(request
            .body_json
            .contains("\"reasoning_content\":\"provider raw reasoning for this same turn\""));
        assert!(
            request.body_json.find("\"reasoning_content\"").unwrap()
                < request.body_json.find("\"tool_calls\"").unwrap()
        );
        assert!(request
            .body_json
            .contains("\"thinking\":{\"type\":\"enabled\"}"));
    }

    #[test]
    fn deepseek_openai_tool_result_request_serializes_typed_replay_body() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
        let request = build_deepseek_openai_multi_tool_result_request_with_reasoning(
            &endpoint,
            "DeepSeek native system\nwith \"quotes\"",
            "Read README.md",
            &[DeepSeekOpenAiToolCallBlock {
                id: "call_quoted".to_string(),
                name: "file_read".to_string(),
                arguments_json: "{\"path\":\"README.md\",\"note\":\"line\\nnext\"}".to_string(),
            }],
            &[DeepSeekOpenAiToolResultBlock {
                tool_call_id: "call_quoted".to_string(),
                content: "README \"content\"\nline 2".to_string(),
            }],
            256,
            true,
            Some("reasoning stays native"),
        )
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&request.body_json).unwrap();
        assert_eq!(
            body.pointer("/thinking/type")
                .and_then(|value| value.as_str()),
            Some("enabled")
        );
        assert_eq!(
            body.pointer("/stream_options/include_usage")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            body.pointer("/messages/2/reasoning_content")
                .and_then(|value| value.as_str()),
            Some("reasoning stays native")
        );
        assert_eq!(
            body.pointer("/messages/2/tool_calls/0/function/arguments")
                .and_then(|value| value.as_str()),
            Some("{\"path\":\"README.md\",\"note\":\"line\\nnext\"}")
        );
        assert_eq!(
            body.pointer("/messages/3/content")
                .and_then(|value| value.as_str()),
            Some("README \"content\"\nline 2")
        );
    }

    #[test]
    fn deepseek_tool_result_request_uses_anthropic_content_blocks() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        let request = build_deepseek_anthropic_tool_result_request(
            &endpoint,
            "DeepSeek native system",
            "Read README.md",
            "toolu_1",
            "file_read",
            "{\"path\":\"README.md\"}",
            "README.md says hello",
            256,
            true,
            file_read_tool_schema_json(),
        )
        .unwrap();
        assert!(request.body_json.contains("\"type\":\"tool_use\""));
        assert!(request.body_json.contains("\"type\":\"tool_result\""));
        assert!(request.body_json.contains("\"tool_use_id\":\"toolu_1\""));
        assert!(!request.body_json.contains("sk-"));
    }

    #[test]
    fn deepseek_tool_result_request_can_preserve_thinking_block() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        let request = build_deepseek_anthropic_tool_result_request_with_thinking(
            &endpoint,
            "DeepSeek native system",
            "Read README.md",
            "toolu_1",
            "file_read",
            "{\"path\":\"README.md\"}",
            "README.md says hello",
            256,
            true,
            file_read_tool_schema_json(),
            "private chain of thought",
            Some("sig_123"),
        )
        .unwrap();
        assert!(request.body_json.contains("\"type\":\"thinking\""));
        assert!(request
            .body_json
            .contains("\"thinking\":\"private chain of thought\""));
        assert!(request.body_json.contains("\"signature\":\"sig_123\""));
        assert!(!request.body_json.contains("\"reasoning_content\""));
        assert!(
            request.body_json.find("\"type\":\"thinking\"")
                < request.body_json.find("\"type\":\"tool_use\"")
        );
    }

    #[test]
    fn deepseek_tool_result_request_supports_multiple_tool_results() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        let request = build_deepseek_anthropic_multi_tool_result_request_with_thinking(
            &endpoint,
            "DeepSeek native system",
            "Inspect workspace",
            &[
                DeepSeekAnthropicToolUseBlock {
                    id: "toolu_1".to_string(),
                    name: "file_read".to_string(),
                    input_json: "{\"path\":\"README.md\"}".to_string(),
                },
                DeepSeekAnthropicToolUseBlock {
                    id: "toolu_2".to_string(),
                    name: "git_status".to_string(),
                    input_json: "{\"root\":\".\"}".to_string(),
                },
            ],
            &[
                DeepSeekAnthropicToolResultBlock {
                    tool_use_id: "toolu_1".to_string(),
                    content: "README content".to_string(),
                    is_error: false,
                },
                DeepSeekAnthropicToolResultBlock {
                    tool_use_id: "toolu_2".to_string(),
                    content: "dirty".to_string(),
                    is_error: false,
                },
            ],
            256,
            true,
            file_read_tool_schema_json(),
            Some("thinking before tools"),
            None,
        )
        .unwrap();
        assert_eq!(
            request.body_json.matches("\"type\":\"tool_use\"").count(),
            2
        );
        assert_eq!(
            request
                .body_json
                .matches("\"type\":\"tool_result\"")
                .count(),
            2
        );
        assert!(
            request.body_json.find("\"tool_use_id\":\"toolu_1\"")
                < request.body_json.find("\"tool_use_id\":\"toolu_2\"")
        );
    }

    #[test]
    fn deepseek_anthropic_tool_result_request_uses_typed_json_serialization() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        let request = build_deepseek_anthropic_multi_tool_result_request_with_thinking(
            &endpoint,
            "DeepSeek \"native\" system\nsecond line",
            "Inspect paths with \"quotes\"",
            &[DeepSeekAnthropicToolUseBlock {
                id: "toolu_json".to_string(),
                name: "file_read".to_string(),
                input_json: r#"{"path":"docs/README \"draft\".md","lines":[1,2,3]}"#.to_string(),
            }],
            &[DeepSeekAnthropicToolResultBlock {
                tool_use_id: "toolu_json".to_string(),
                content: "line one\nline \"two\"".to_string(),
                is_error: true,
            }],
            256,
            true,
            file_read_tool_schema_json(),
            Some("thinking with \"quotes\"\nand newline"),
            Some("sig_json"),
        )
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&request.body_json).unwrap();
        assert_eq!(body["system"], "DeepSeek \"native\" system\nsecond line");
        assert_eq!(
            body["messages"][0]["content"],
            "Inspect paths with \"quotes\""
        );
        assert_eq!(body["messages"][1]["content"][0]["type"], "thinking");
        assert_eq!(body["messages"][1]["content"][0]["signature"], "sig_json");
        assert_eq!(body["messages"][1]["content"][1]["type"], "tool_use");
        assert_eq!(
            body["messages"][1]["content"][1]["input"]["path"],
            "docs/README \"draft\".md"
        );
        assert_eq!(body["messages"][1]["content"][1]["input"]["lines"][2], 3);
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(
            body["messages"][2]["content"][0]["content"],
            "line one\nline \"two\""
        );
        assert_eq!(body["messages"][2]["content"][0]["is_error"], true);
        assert!(!request.body_json.contains("\"reasoning_content\""));
    }

    #[test]
    fn deepseek_anthropic_tool_result_request_preserves_invalid_input_for_error_result() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        let request = build_deepseek_anthropic_multi_tool_result_request_with_thinking(
            &endpoint,
            "system",
            "task",
            &[DeepSeekAnthropicToolUseBlock {
                id: "toolu_bad".to_string(),
                name: "file_read".to_string(),
                input_json: "{\"path\":".to_string(),
            }],
            &[DeepSeekAnthropicToolResultBlock {
                tool_use_id: "toolu_bad".to_string(),
                content: "malformed tool json".to_string(),
                is_error: true,
            }],
            256,
            true,
            file_read_tool_schema_json(),
            None,
            None,
        )
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&request.body_json).unwrap();
        let input = &body["messages"][1]["content"][0]["input"];
        assert_eq!(input["__researchcode_malformed_tool_input"], true);
        assert_eq!(input["raw_input_json"], "{\"path\":");
        assert!(input["parse_error"].as_str().unwrap().contains("EOF"));
        assert_eq!(body["messages"][2]["content"][0]["is_error"], true);
    }

    #[test]
    fn deepseek_anthropic_tool_result_request_rejects_invalid_input_for_non_error_result() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        let error = build_deepseek_anthropic_multi_tool_result_request_with_thinking(
            &endpoint,
            "system",
            "task",
            &[DeepSeekAnthropicToolUseBlock {
                id: "toolu_bad".to_string(),
                name: "file_read".to_string(),
                input_json: "{\"path\":".to_string(),
            }],
            &[DeepSeekAnthropicToolResultBlock {
                tool_use_id: "toolu_bad".to_string(),
                content: "not an error".to_string(),
                is_error: false,
            }],
            256,
            true,
            file_read_tool_schema_json(),
            None,
            None,
        )
        .unwrap_err();
        assert!(error.contains("tool_use input_json for toolu_bad is not valid JSON"));
    }

    #[test]
    fn rejects_non_deepseek_endpoint() {
        let endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        assert!(build_deepseek_anthropic_request(
            &endpoint,
            &[ModelRequestMessage {
                role: "user".to_string(),
                content: "task".to_string(),
                cache_control_ttl: None,
            }],
            1024,
            false,
        )
        .is_err());
    }

    #[test]
    fn builds_qwen_openai_request_without_key_material() {
        let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
        let request = build_qwen_openai_request(
            &endpoint,
            &[ModelRequestMessage {
                role: "user".to_string(),
                content: "Patch the file".to_string(),
                cache_control_ttl: None,
            }],
            1024,
            true,
        )
        .unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.authorization_env, "QWEN_API_KEY");
        assert!(request.body_json.contains("\"model\":\"Qwen/Qwen3.6-27B\""));
        assert!(request.body_json.contains("\"stream\":true"));
        assert!(request.body_json.contains("\"reasoning_effort\":\"none\""));
        assert!(!request.body_json.contains("sk-"));
    }

    #[test]
    fn builds_qwen_openai_request_with_native_tools() {
        let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
        let request = build_qwen_openai_request_with_tools(
            &endpoint,
            &[ModelRequestMessage {
                role: "user".to_string(),
                content: "Read README.md".to_string(),
                cache_control_ttl: None,
            }],
            1024,
            true,
            r#"[{"type":"function","function":{"name":"file_read","description":"Read file","parameters":{"type":"object","required":["path"],"properties":{"path":{"type":"string"}}}}}]"#,
        )
        .unwrap();
        assert!(request.body_json.contains("\"tools\":["));
        assert!(request.body_json.contains("\"reasoning_effort\":\"none\""));
        assert!(request.body_json.contains("\"tool_choice\":\"auto\""));
        assert!(request.body_json.contains("\"name\":\"file_read\""));
        assert!(!request.body_json.contains("\"input_schema\""));
        assert!(!request.body_json.contains("sk-"));
    }

    #[test]
    fn qwen_openai_builder_serializes_typed_compatible_body_without_deepseek_native_fields() {
        let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
        let request = build_qwen_openai_request_with_tools(
            &endpoint,
            &[ModelRequestMessage::new(
                "user",
                "read \"README.md\"\nthen continue",
            )],
            1024,
            true,
            r#"[{"type":"function","function":{"name":"file_read","description":"Read file","parameters":{"type":"object","required":["path"],"properties":{"path":{"type":"string"}}}}}]"#,
        )
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&request.body_json).unwrap();
        assert_eq!(
            body.get("model").and_then(|value| value.as_str()),
            Some("Qwen/Qwen3.6-27B")
        );
        assert_eq!(
            body.get("reasoning_effort")
                .and_then(|value| value.as_str()),
            Some("none")
        );
        assert!(body.get("thinking").is_none());
        assert!(body.get("stream_options").is_none());
        assert_eq!(
            body.get("tool_choice").and_then(|value| value.as_str()),
            Some("auto")
        );
        assert_eq!(
            body.pointer("/messages/0/content")
                .and_then(|value| value.as_str()),
            Some("read \"README.md\"\nthen continue")
        );
    }

    #[test]
    fn qwen_tool_result_request_uses_openai_tool_messages() {
        let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
        let request = build_qwen_openai_tool_result_request(
            &endpoint,
            "Qwen native system",
            "Read README.md",
            "call_1",
            "file_read",
            "{\"path\":\"README.md\"}",
            "README.md says hello",
            256,
            true,
        )
        .unwrap();
        assert!(request.body_json.contains("\"tool_calls\""));
        assert!(request.body_json.contains("\"role\":\"tool\""));
        assert!(request.body_json.contains("\"tool_call_id\":\"call_1\""));
        assert!(!request.body_json.contains("sk-"));
    }

    #[test]
    fn qwen_tool_result_request_supports_multiple_tool_messages() {
        let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
        let request = build_qwen_openai_multi_tool_result_request(
            &endpoint,
            "Qwen native system",
            "Inspect workspace",
            &[
                QwenOpenAiToolCallBlock {
                    id: "call_1".to_string(),
                    name: "file_read".to_string(),
                    arguments_json: "{\"path\":\"README.md\"}".to_string(),
                },
                QwenOpenAiToolCallBlock {
                    id: "call_2".to_string(),
                    name: "git_status".to_string(),
                    arguments_json: "{\"root\":\".\"}".to_string(),
                },
            ],
            &[
                QwenOpenAiToolResultBlock {
                    tool_call_id: "call_1".to_string(),
                    content: "README content".to_string(),
                },
                QwenOpenAiToolResultBlock {
                    tool_call_id: "call_2".to_string(),
                    content: "dirty".to_string(),
                },
            ],
            256,
            true,
        )
        .unwrap();
        assert_eq!(request.body_json.matches("\"role\":\"tool\"").count(), 2);
        assert_eq!(
            request.body_json.matches("\"type\":\"function\"").count(),
            2
        );
        assert!(
            request.body_json.find("\"tool_call_id\":\"call_1\"")
                < request.body_json.find("\"tool_call_id\":\"call_2\"")
        );
    }

    #[test]
    fn qwen_tool_result_request_serializes_typed_replay_body_without_deepseek_native_fields() {
        let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
        let request = build_qwen_openai_multi_tool_result_request(
            &endpoint,
            "Qwen native system\nwith \"quotes\"",
            "Read README.md",
            &[QwenOpenAiToolCallBlock {
                id: "call_quoted".to_string(),
                name: "file_read".to_string(),
                arguments_json: "{\"path\":\"README.md\",\"note\":\"line\\nnext\"}".to_string(),
            }],
            &[QwenOpenAiToolResultBlock {
                tool_call_id: "call_quoted".to_string(),
                content: "README \"content\"\nline 2".to_string(),
            }],
            256,
            true,
        )
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&request.body_json).unwrap();
        assert!(body.get("thinking").is_none());
        assert!(body.get("stream_options").is_none());
        assert_eq!(
            body.pointer("/messages/2/tool_calls/0/function/arguments")
                .and_then(|value| value.as_str()),
            Some("{\"path\":\"README.md\",\"note\":\"line\\nnext\"}")
        );
        assert_eq!(
            body.pointer("/messages/3/content")
                .and_then(|value| value.as_str()),
            Some("README \"content\"\nline 2")
        );
    }

    #[test]
    fn qwen_request_requires_resolved_base_url() {
        let endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        assert!(build_qwen_openai_request(
            &endpoint,
            &[ModelRequestMessage {
                role: "user".to_string(),
                content: "task".to_string(),
                cache_control_ttl: None,
            }],
            1024,
            false,
        )
        .is_err());
    }

    fn file_read_tool_schema_json() -> &'static str {
        r#"[{"name":"file_read","description":"Read a UTF-8 text file from the current workspace.","input_schema":{"type":"object","properties":{"path":{"type":"string"},"max_bytes":{"type":"integer"}},"required":["path"]}}]"#
    }
}
