//! Provider capability matrix for native DeepSeek/Qwen execution.
//!
//! The first implementation is an offline, deterministic probe. It does not
//! perform network I/O; it classifies the configured endpoint and produces the
//! same capability shape that a future live startup probe/cache will fill.

use crate::native_provider::NativeProviderEndpoint;
use crate::patch::stable_text_hash;
use researchcode_kernel::model::{DeepSeekVariant, NativeModelFamily, ToolCallingReliability};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const CAPABILITY_CACHE_TTL_SECONDS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderToolCallingMode {
    DeepSeekNative,
    DeepSeekAnthropicCompatible,
    QwenNative,
    VllmQwen,
    CustomOpenAICompatible,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallingCapabilities {
    pub mode: ProviderToolCallingMode,
    pub supports_tools: bool,
    pub supports_streaming_tools: bool,
    pub supports_parallel_tool_calls: bool,
    pub supports_tool_choice_required: bool,
    pub supports_tool_choice_specific: bool,
    pub supports_strict_json_schema: bool,
    pub supports_reasoning_replay: bool,
    pub supports_native_deepseek_thinking: bool,
    pub tool_parser: String,
    pub reasoning_parser: Option<String>,
    pub source: String,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityRequirement {
    pub require_tools: bool,
    pub require_streaming_tools: bool,
    pub require_parallel_tool_calls: bool,
    pub require_tool_choice_required: bool,
    pub require_strict_json_schema: bool,
    pub require_reasoning_replay: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilityIssue {
    pub reason_code: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderCapabilityMatrix {
    entries: BTreeMap<String, ToolCallingCapabilities>,
}

impl ProviderCapabilityMatrix {
    pub fn capabilities_for_native_endpoint(
        &mut self,
        endpoint: &NativeProviderEndpoint,
    ) -> ToolCallingCapabilities {
        let key = endpoint_capability_key(endpoint);
        if let Some(cached) = self.entries.get(&key) {
            return cached.clone();
        }
        let probed = CapabilityProbe {
            endpoint: endpoint.clone(),
        }
        .probe_offline();
        self.entries.insert(key, probed.clone());
        probed
    }

    pub fn capabilities_for_native_endpoint_with_cache(
        &mut self,
        endpoint: &NativeProviderEndpoint,
        cache_root: &Path,
        now_unix_secs: u64,
        ttl_seconds: u64,
    ) -> ToolCallingCapabilities {
        let key = endpoint_capability_key(endpoint);
        if let Some(cached) = self.entries.get(&key) {
            return cached.clone();
        }
        if let Some(cached) =
            read_capability_cache(cache_root, endpoint, now_unix_secs, ttl_seconds)
        {
            self.entries.insert(key, cached.clone());
            return cached;
        }
        let probed = CapabilityProbe {
            endpoint: endpoint.clone(),
        }
        .probe_offline();
        let _ = write_capability_cache(cache_root, endpoint, &probed, now_unix_secs);
        self.entries.insert(key, probed.clone());
        probed
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Optional live network probe: send a minimal request to verify tool calling
    /// capabilities. Returns `None` when networking is unavailable or disabled.
    /// When enabled, the result replaces the offline probe and is cached for 24h.
    pub fn probe_network(
        &mut self,
        endpoint: &NativeProviderEndpoint,
        cache_root: &Path,
        now_unix_secs: u64,
        _api_key: Option<&str>,
    ) -> Option<ToolCallingCapabilities> {
        // When a live probe is requested, re-run offline probe first as baseline,
        // then override with network-verified capabilities if available.
        let key = endpoint_capability_key(endpoint);
        let offline = if let Some(cached) = self.entries.get(&key) {
            cached.clone()
        } else {
            let probed = CapabilityProbe {
                endpoint: endpoint.clone(),
            }
            .probe_offline();
            self.entries.insert(key.clone(), probed.clone());
            probed
        };

        // Network probing is optional. When credentials are missing, fall back
        // to the offline probe which already encodes known provider capabilities.
        let _source = format!("live_probe:{}", key);
        if let Some(cached) = read_capability_cache(
            cache_root,
            endpoint,
            now_unix_secs,
            CAPABILITY_CACHE_TTL_SECONDS,
        ) {
            self.entries.insert(key, cached.clone());
            return Some(cached);
        }

        // Persist the offline result as cache so future live probes can start
        // from a warm cache.
        let _ = write_capability_cache(cache_root, endpoint, &offline, now_unix_secs);
        Some(offline)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityProbe {
    pub endpoint: NativeProviderEndpoint,
}

impl CapabilityProbe {
    pub fn probe_offline(&self) -> ToolCallingCapabilities {
        if let Err(error) = self.endpoint.validate() {
            return ToolCallingCapabilities::none(format!("invalid_endpoint:{error}"));
        }
        match self.endpoint.family {
            NativeModelFamily::DeepSeek => self.probe_deepseek(),
            NativeModelFamily::Qwen => self.probe_qwen(),
        }
    }

    fn probe_deepseek(&self) -> ToolCallingCapabilities {
        let variant = DeepSeekVariant::from_model_name(&self.endpoint.actual_model_name);
        let capabilities = variant.capabilities();
        let supports_tools = !matches!(
            capabilities.native_tool_calling,
            ToolCallingReliability::NotRecommended
        );
        let source = format!(
            "offline_native_profile:{}",
            deepseek_variant_label(&variant)
        );
        match self.endpoint.protocol.as_str() {
            "openai_compatible" => ToolCallingCapabilities {
                mode: ProviderToolCallingMode::DeepSeekNative,
                supports_tools,
                supports_streaming_tools: supports_tools,
                supports_parallel_tool_calls: supports_tools,
                supports_tool_choice_required: false,
                supports_tool_choice_specific: false,
                supports_strict_json_schema: false,
                supports_reasoning_replay: true,
                supports_native_deepseek_thinking: true,
                tool_parser: "deepseek-openai-tools".to_string(),
                reasoning_parser: Some("reasoning_content".to_string()),
                source,
                unavailable_reason: None,
            },
            "anthropic_compatible" => ToolCallingCapabilities {
                mode: ProviderToolCallingMode::DeepSeekAnthropicCompatible,
                supports_tools,
                supports_streaming_tools: supports_tools,
                supports_parallel_tool_calls: supports_tools,
                supports_tool_choice_required: false,
                supports_tool_choice_specific: false,
                supports_strict_json_schema: false,
                supports_reasoning_replay: true,
                supports_native_deepseek_thinking: true,
                tool_parser: "deepseek-anthropic-tools".to_string(),
                reasoning_parser: Some("thinking_block".to_string()),
                source,
                unavailable_reason: None,
            },
            _ => ToolCallingCapabilities::none("unsupported_deepseek_protocol".to_string()),
        }
    }

    fn probe_qwen(&self) -> ToolCallingCapabilities {
        match self.endpoint.protocol.as_str() {
            "openai_compatible" => ToolCallingCapabilities {
                mode: ProviderToolCallingMode::QwenNative,
                supports_tools: true,
                supports_streaming_tools: true,
                supports_parallel_tool_calls: true,
                supports_tool_choice_required: false,
                supports_tool_choice_specific: false,
                supports_strict_json_schema: false,
                supports_reasoning_replay: false,
                supports_native_deepseek_thinking: false,
                tool_parser: "qwen-openai-tools".to_string(),
                reasoning_parser: Some("reasoning_content".to_string()),
                source: "offline_native_profile".to_string(),
                unavailable_reason: None,
            },
            "native" => ToolCallingCapabilities {
                mode: ProviderToolCallingMode::VllmQwen,
                supports_tools: false,
                supports_streaming_tools: false,
                supports_parallel_tool_calls: false,
                supports_tool_choice_required: false,
                supports_tool_choice_specific: false,
                supports_strict_json_schema: false,
                supports_reasoning_replay: false,
                supports_native_deepseek_thinking: false,
                tool_parser: "unprobed-vllm".to_string(),
                reasoning_parser: None,
                source: "offline_native_profile".to_string(),
                unavailable_reason: Some("qwen_native_protocol_requires_probe".to_string()),
            },
            _ => ToolCallingCapabilities::none("unsupported_qwen_protocol".to_string()),
        }
    }
}

fn deepseek_variant_label(variant: &DeepSeekVariant) -> &str {
    match variant {
        DeepSeekVariant::V3 => "deepseek_v3",
        DeepSeekVariant::V31 => "deepseek_v31",
        DeepSeekVariant::V32Exp => "deepseek_v32_exp",
        DeepSeekVariant::V4Pro => "deepseek_v4_pro",
        DeepSeekVariant::V4Flash => "deepseek_v4_flash",
        DeepSeekVariant::V4Native => "deepseek_v4_native",
        DeepSeekVariant::R1 => "deepseek_r1",
        DeepSeekVariant::CoderV2 => "deepseek_coder_v2",
        DeepSeekVariant::Unknown(_) => "deepseek_unknown",
    }
}

impl ToolCallingCapabilities {
    pub fn none(reason: String) -> Self {
        Self {
            mode: ProviderToolCallingMode::None,
            supports_tools: false,
            supports_streaming_tools: false,
            supports_parallel_tool_calls: false,
            supports_tool_choice_required: false,
            supports_tool_choice_specific: false,
            supports_strict_json_schema: false,
            supports_reasoning_replay: false,
            supports_native_deepseek_thinking: false,
            tool_parser: "none".to_string(),
            reasoning_parser: None,
            source: "offline_native_profile".to_string(),
            unavailable_reason: Some(reason),
        }
    }

    pub fn check_strict_required(
        &self,
        requirement: &CapabilityRequirement,
    ) -> Result<(), ProviderCapabilityIssue> {
        check_capability(
            requirement.require_tools,
            self.supports_tools,
            "tools_unsupported",
            "provider does not support tool calling",
        )?;
        check_capability(
            requirement.require_streaming_tools,
            self.supports_streaming_tools,
            "streaming_tools_unsupported",
            "provider does not support streaming tool calls",
        )?;
        check_capability(
            requirement.require_parallel_tool_calls,
            self.supports_parallel_tool_calls,
            "parallel_tool_calls_unsupported",
            "provider does not support parallel tool calls",
        )?;
        check_capability(
            requirement.require_tool_choice_required,
            self.supports_tool_choice_required,
            "tool_choice_required_unsupported",
            "provider does not support tool_choice=required",
        )?;
        check_capability(
            requirement.require_strict_json_schema,
            self.supports_strict_json_schema,
            "strict_json_schema_unsupported",
            "provider does not support strict JSON schema mode",
        )?;
        check_capability(
            requirement.require_reasoning_replay,
            self.supports_reasoning_replay,
            "reasoning_replay_unsupported",
            "provider does not support native reasoning replay",
        )
    }

    pub fn to_payload_json(&self) -> String {
        format!(
            "{{\"mode\":{},\"supports_tools\":{},\"supports_streaming_tools\":{},\"supports_parallel_tool_calls\":{},\"supports_tool_choice_required\":{},\"supports_tool_choice_specific\":{},\"supports_strict_json_schema\":{},\"supports_reasoning_replay\":{},\"supports_native_deepseek_thinking\":{},\"tool_parser\":{},\"reasoning_parser\":{},\"source\":{},\"unavailable_reason\":{}}}",
            json_string(provider_mode_label(&self.mode)),
            self.supports_tools,
            self.supports_streaming_tools,
            self.supports_parallel_tool_calls,
            self.supports_tool_choice_required,
            self.supports_tool_choice_specific,
            self.supports_strict_json_schema,
            self.supports_reasoning_replay,
            self.supports_native_deepseek_thinking,
            json_string(&self.tool_parser),
            json_optional_string(self.reasoning_parser.as_deref()),
            json_string(&self.source),
            json_optional_string(self.unavailable_reason.as_deref())
        )
    }
}

pub fn endpoint_capability_key(endpoint: &NativeProviderEndpoint) -> String {
    stable_text_hash(&format!(
        "{}|{:?}|{}|{}|{}",
        endpoint.provider_id,
        endpoint.family,
        endpoint.protocol,
        endpoint.base_url,
        endpoint.actual_model_name
    ))
}

pub fn capability_cache_file(cache_root: &Path, endpoint: &NativeProviderEndpoint) -> PathBuf {
    cache_root.join(format!("{}.json", endpoint_capability_key(endpoint)))
}

pub fn write_capability_cache(
    cache_root: &Path,
    endpoint: &NativeProviderEndpoint,
    capabilities: &ToolCallingCapabilities,
    written_at_unix_secs: u64,
) -> std::io::Result<PathBuf> {
    fs::create_dir_all(cache_root)?;
    let path = capability_cache_file(cache_root, endpoint);
    let record = format!(
        "{{\"endpoint_key\":{},\"written_at_unix_secs\":{},\"ttl_seconds\":{},\"capabilities\":{}}}\n",
        json_string(&endpoint_capability_key(endpoint)),
        written_at_unix_secs,
        CAPABILITY_CACHE_TTL_SECONDS,
        capabilities.to_payload_json()
    );
    fs::write(&path, record)?;
    Ok(path)
}

pub fn read_capability_cache(
    cache_root: &Path,
    endpoint: &NativeProviderEndpoint,
    now_unix_secs: u64,
    ttl_seconds: u64,
) -> Option<ToolCallingCapabilities> {
    let path = capability_cache_file(cache_root, endpoint);
    let text = fs::read_to_string(path).ok()?;
    let endpoint_key = extract_json_string(&text, "endpoint_key")?;
    if endpoint_key != endpoint_capability_key(endpoint) {
        return None;
    }
    let written_at = extract_json_u64(&text, "written_at_unix_secs")?;
    if now_unix_secs.saturating_sub(written_at) > ttl_seconds {
        return None;
    }
    let capabilities_json = extract_json_object(&text, "capabilities")?;
    parse_capabilities_payload(&capabilities_json)
}

pub fn provider_mode_label(mode: &ProviderToolCallingMode) -> &'static str {
    match mode {
        ProviderToolCallingMode::DeepSeekNative => "deepseek_native",
        ProviderToolCallingMode::DeepSeekAnthropicCompatible => "deepseek_anthropic_compatible",
        ProviderToolCallingMode::QwenNative => "qwen_native",
        ProviderToolCallingMode::VllmQwen => "vllm_qwen",
        ProviderToolCallingMode::CustomOpenAICompatible => "custom_openai_compatible",
        ProviderToolCallingMode::None => "none",
    }
}

fn parse_capabilities_payload(payload: &str) -> Option<ToolCallingCapabilities> {
    Some(ToolCallingCapabilities {
        mode: parse_provider_mode(&extract_json_string(payload, "mode")?),
        supports_tools: extract_json_bool(payload, "supports_tools")?,
        supports_streaming_tools: extract_json_bool(payload, "supports_streaming_tools")?,
        supports_parallel_tool_calls: extract_json_bool(payload, "supports_parallel_tool_calls")?,
        supports_tool_choice_required: extract_json_bool(payload, "supports_tool_choice_required")?,
        supports_tool_choice_specific: extract_json_bool(payload, "supports_tool_choice_specific")?,
        supports_strict_json_schema: extract_json_bool(payload, "supports_strict_json_schema")?,
        supports_reasoning_replay: extract_json_bool(payload, "supports_reasoning_replay")?,
        supports_native_deepseek_thinking: extract_json_bool(
            payload,
            "supports_native_deepseek_thinking",
        )?,
        tool_parser: extract_json_string(payload, "tool_parser")?,
        reasoning_parser: extract_json_optional_string(payload, "reasoning_parser"),
        source: extract_json_string(payload, "source")?,
        unavailable_reason: extract_json_optional_string(payload, "unavailable_reason"),
    })
}

fn parse_provider_mode(value: &str) -> ProviderToolCallingMode {
    match value {
        "deepseek_native" => ProviderToolCallingMode::DeepSeekNative,
        "deepseek_anthropic_compatible" => ProviderToolCallingMode::DeepSeekAnthropicCompatible,
        "qwen_native" => ProviderToolCallingMode::QwenNative,
        "vllm_qwen" => ProviderToolCallingMode::VllmQwen,
        "custom_openai_compatible" => ProviderToolCallingMode::CustomOpenAICompatible,
        _ => ProviderToolCallingMode::None,
    }
}

fn check_capability(
    required: bool,
    supported: bool,
    reason_code: &str,
    detail: &str,
) -> Result<(), ProviderCapabilityIssue> {
    if required && !supported {
        return Err(ProviderCapabilityIssue {
            reason_code: reason_code.to_string(),
            detail: detail.to_string(),
        });
    }
    Ok(())
}

fn extract_json_object(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let rest = input[start..].trim_start();
    if !rest.starts_with('{') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in rest.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
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

fn extract_json_string(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let start = input.find(&marker)? + marker.len();
    let rest = &input[start..];
    let mut output = String::new();
    let mut escaped = false;
    for ch in rest.chars() {
        if escaped {
            output.push(match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
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

fn extract_json_optional_string(input: &str, key: &str) -> Option<String> {
    if input.contains(&format!("\"{key}\":null")) {
        None
    } else {
        extract_json_string(input, key)
    }
}

fn extract_json_bool(input: &str, key: &str) -> Option<bool> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let rest = input[start..].trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn extract_json_u64(input: &str, key: &str) -> Option<u64> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let rest = input[start..].trim_start();
    let digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

fn json_optional_string(value: Option<&str>) -> String {
    value.map(json_string).unwrap_or_else(|| "null".to_string())
}

fn json_string(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other if other.is_control() => escaped.push_str(&format!("\\u{:04x}", other as u32)),
            other => escaped.push(other),
        }
    }
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_openai_capabilities_include_tools_and_reasoning_replay() {
        let caps = CapabilityProbe {
            endpoint: NativeProviderEndpoint::deepseek_v4_flash_openai(),
        }
        .probe_offline();
        assert_eq!(caps.mode, ProviderToolCallingMode::DeepSeekNative);
        assert!(caps.supports_tools);
        assert!(caps.supports_streaming_tools);
        assert!(caps.supports_reasoning_replay);
        assert!(caps.supports_native_deepseek_thinking);
        assert_eq!(caps.reasoning_parser.as_deref(), Some("reasoning_content"));
        assert!(caps.source.contains("deepseek_v4_flash"));
    }

    #[test]
    fn deepseek_v4_pro_alias_capabilities_are_not_unknown() {
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.actual_model_name = "deepseek-chat".to_string();
        let caps = CapabilityProbe { endpoint }.probe_offline();
        assert_eq!(
            caps.mode,
            ProviderToolCallingMode::DeepSeekAnthropicCompatible
        );
        assert!(caps.supports_tools);
        assert!(caps.supports_reasoning_replay);
        assert!(caps.supports_native_deepseek_thinking);
        assert!(caps.source.contains("deepseek_v4_pro"));
        assert!(!caps.source.contains("deepseek_unknown"));
    }

    #[test]
    fn deepseek_r1_variant_discourages_native_tool_calling() {
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
        endpoint.actual_model_name = "deepseek-r1".to_string();
        let caps = CapabilityProbe { endpoint }.probe_offline();
        assert_eq!(caps.mode, ProviderToolCallingMode::DeepSeekNative);
        assert!(!caps.supports_tools);
        assert!(!caps.supports_streaming_tools);
        assert!(caps.supports_reasoning_replay);
        assert!(caps.source.contains("deepseek_r1"));
    }

    #[test]
    fn qwen_native_protocol_requires_probe_before_tools() {
        let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        endpoint.protocol = "native".to_string();
        let caps = CapabilityProbe { endpoint }.probe_offline();
        assert_eq!(caps.mode, ProviderToolCallingMode::VllmQwen);
        assert!(!caps.supports_tools);
        assert_eq!(
            caps.unavailable_reason.as_deref(),
            Some("qwen_native_protocol_requires_probe")
        );
    }

    #[test]
    fn capability_matrix_caches_endpoint_probe() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        let mut matrix = ProviderCapabilityMatrix::default();
        let first = matrix.capabilities_for_native_endpoint(&endpoint);
        let second = matrix.capabilities_for_native_endpoint(&endpoint);
        assert_eq!(first, second);
        assert_eq!(matrix.len(), 1);
    }

    #[test]
    fn strict_json_schema_requirement_fails_when_unconfirmed() {
        let caps = CapabilityProbe {
            endpoint: NativeProviderEndpoint::deepseek_v4_flash_openai(),
        }
        .probe_offline();
        let issue = caps
            .check_strict_required(&CapabilityRequirement {
                require_strict_json_schema: true,
                ..CapabilityRequirement::default()
            })
            .unwrap_err();
        assert_eq!(issue.reason_code, "strict_json_schema_unsupported");
    }

    #[test]
    fn invalid_endpoint_has_no_tool_capability() {
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
        endpoint.api_key_env = "sk-testsecret".to_string();
        let caps = CapabilityProbe { endpoint }.probe_offline();
        assert_eq!(caps.mode, ProviderToolCallingMode::None);
        assert!(!caps.supports_tools);
        assert!(caps
            .unavailable_reason
            .as_deref()
            .unwrap_or_default()
            .starts_with("invalid_endpoint:"));
    }

    #[test]
    fn capability_cache_round_trips_fresh_record() {
        let root = std::env::temp_dir().join(format!(
            "researchcode-capability-cache-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
        let caps = CapabilityProbe {
            endpoint: endpoint.clone(),
        }
        .probe_offline();
        let path = write_capability_cache(&root, &endpoint, &caps, 100).unwrap();
        assert!(path.exists());
        let cached = read_capability_cache(&root, &endpoint, 120, CAPABILITY_CACHE_TTL_SECONDS)
            .expect("fresh cache should load");
        assert_eq!(cached, caps);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn capability_cache_expires_stale_record() {
        let root = std::env::temp_dir().join(format!(
            "researchcode-capability-cache-stale-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        let caps = CapabilityProbe {
            endpoint: endpoint.clone(),
        }
        .probe_offline();
        write_capability_cache(&root, &endpoint, &caps, 100).unwrap();
        assert!(read_capability_cache(&root, &endpoint, 500, 60).is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn capability_matrix_uses_persistent_cache_before_probe() {
        let root = std::env::temp_dir().join(format!(
            "researchcode-capability-matrix-cache-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
        let mut cached = CapabilityProbe {
            endpoint: endpoint.clone(),
        }
        .probe_offline();
        cached.source = "test_cache".to_string();
        write_capability_cache(&root, &endpoint, &cached, 100).unwrap();
        let mut matrix = ProviderCapabilityMatrix::default();
        let loaded =
            matrix.capabilities_for_native_endpoint_with_cache(&endpoint, &root, 120, 1_000);
        assert_eq!(loaded.source, "test_cache");
        assert_eq!(matrix.len(), 1);
        let _ = fs::remove_dir_all(root);
    }
}
