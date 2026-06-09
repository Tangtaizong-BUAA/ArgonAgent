#![allow(unused_imports, dead_code)]

use crate::agent_smokes::*;
use crate::core_smokes::*;
use crate::deepseek::*;
use crate::fixtures::*;
use crate::live_model::*;
use crate::prelude::*;
use crate::qwen_tools::*;
use crate::runtime_smokes::*;

pub(crate) const TUI_LIVE_DEEPSEEK_CHAT_MAX_TOKENS: u64 = 8_192;
pub(crate) const TUI_LIVE_DEEPSEEK_GENERATION_MAX_TOKENS: u64 = 16_384;
pub(crate) const TUI_LIVE_DEEPSEEK_ANALYSIS_MAX_TOKENS: u64 = 20_000;

pub(crate) fn deepseek_live_endpoint_from_env() -> NativeProviderEndpoint {
    let protocol = env::var("RESEARCHCODE_DEEPSEEK_PROTOCOL")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let env_base_url = env::var("DEEPSEEK_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let mut endpoint = if matches!(protocol.as_str(), "openai" | "openai_compatible") {
        NativeProviderEndpoint::deepseek_v4_flash_openai()
    } else if matches!(protocol.as_str(), "anthropic" | "anthropic_compatible") {
        NativeProviderEndpoint::deepseek_v4_flash_anthropic()
    } else if env_base_url
        .as_deref()
        .is_some_and(|value| value.contains("/anthropic"))
    {
        NativeProviderEndpoint::deepseek_v4_flash_anthropic()
    } else {
        // Default to Anthropic-compatible because DeepSeek native tool_use/tool_result
        // continuity is most stable on this transport in the current runtime path.
        NativeProviderEndpoint::deepseek_v4_flash_anthropic()
    };
    if let Some(base_url) = env_base_url {
        endpoint.base_url = base_url;
    }
    if let Ok(model_name) = env::var("DEEPSEEK_MODEL") {
        let model_name = model_name.trim();
        if !model_name.is_empty() {
            endpoint.actual_model_name = model_name.to_string();
        }
    }
    endpoint
}

pub(crate) fn qwen_live_endpoint_from_env() -> NativeProviderEndpoint {
    let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
    if let Ok(base_url) = env::var("QWEN_BASE_URL") {
        let base_url = base_url.trim();
        if !base_url.is_empty() {
            endpoint.base_url = base_url.to_string();
        }
    }
    endpoint
}

pub(crate) fn deepseek_openai_fallback_endpoint_from(
    primary: &NativeProviderEndpoint,
) -> NativeProviderEndpoint {
    let mut fallback = NativeProviderEndpoint::deepseek_v4_flash_openai();
    fallback.actual_model_name = primary.actual_model_name.clone();
    fallback.display_model_name = primary.display_model_name.clone();
    if let Ok(base_url) = env::var("DEEPSEEK_OPENAI_BASE_URL") {
        let base_url = base_url.trim();
        if !base_url.is_empty() {
            fallback.base_url = base_url.to_string();
            return fallback;
        }
    }
    if primary.base_url.contains("/anthropic") {
        fallback.base_url = primary.base_url.replace("/anthropic", "");
    }
    fallback
}

pub(crate) fn truncate_for_panel(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    output.push('…');
    output
}

pub(crate) fn deepseek_tui_max_tokens_for_task(task: &str) -> u64 {
    let lowered = task.to_ascii_lowercase();
    let wants_long_generation = [
        "html",
        "css",
        "javascript",
        "js",
        "小程序",
        "网页",
        "页面",
        "生成",
        "创建",
        "写个",
        "实现",
        "code",
        "app",
        "tool",
    ]
    .iter()
    .any(|needle| lowered.contains(needle));
    let wants_deep_analysis = [
        "深度",
        "分析",
        "解析",
        "代码库",
        "repo",
        "repository",
        "ultraplan",
        "ultrareview",
        "review",
    ]
    .iter()
    .any(|needle| lowered.contains(needle));
    if wants_deep_analysis {
        TUI_LIVE_DEEPSEEK_ANALYSIS_MAX_TOKENS
    } else if wants_long_generation {
        TUI_LIVE_DEEPSEEK_GENERATION_MAX_TOKENS
    } else {
        TUI_LIVE_DEEPSEEK_CHAT_MAX_TOKENS
    }
}

pub(crate) fn workspace_provider_sidecar_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("scripts")
        .join("provider_http_sidecar.py")
}

pub(crate) fn sidecar_stream_visible_input_json(request: &PreparedModelHttpRequest) -> String {
    format!(
        "{{\"mode\":\"stream_visible_text\",\"method\":\"{}\",\"url\":\"{}\",\"authorization_env\":\"{}\",\"body_json\":\"{}\",\"stream\":{},\"response_body_path\":\"/dev/null\"}}",
        escape_json_cli(&request.method),
        escape_json_cli(&request.url),
        escape_json_cli(&request.authorization_env),
        escape_json_cli(&request.body_json),
        request.stream
    )
}

pub(crate) fn escape_json_cli(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

pub(crate) fn json_string_cli(value: &str) -> String {
    format!("\"{}\"", escape_json_cli(value))
}

pub(crate) fn decode_hex_utf8_cli(value: &str) -> Result<String, String> {
    if value.len() % 2 != 0 {
        return Err("hex value has odd length".to_string());
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for index in (0..value.len()).step_by(2) {
        let byte =
            u8::from_str_radix(&value[index..index + 2], 16).map_err(|error| error.to_string())?;
        bytes.push(byte);
    }
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

pub(crate) fn extract_json_string_field_cli(input: &str, key: &str) -> Option<String> {
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
                '"' => '"',
                '\\' => '\\',
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

pub(crate) fn extract_json_u64_field_cli(input: &str, key: &str) -> Option<u64> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let tail = input[start..].trim_start();
    if tail.starts_with("null") {
        return None;
    }
    let digits = tail
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

pub(crate) fn deepseek_tui_tool_schema_json() -> String {
    tui_fastauto_provider_tool_schema_json()
}
