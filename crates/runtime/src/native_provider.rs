//! Native provider endpoint gates for DeepSeek and Qwen.
//!
//! This module deliberately performs no network I/O. It validates endpoint
//! metadata and decides whether a future live call is allowed. API keys are
//! referenced only by environment variable name; raw key material must never be
//! stored in config, events, or artifacts.

use researchcode_kernel::model::NativeModelFamily;
use std::env;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeProviderEndpoint {
    pub provider_id: String,
    pub family: NativeModelFamily,
    pub protocol: String,
    pub base_url: String,
    pub api_key_env: String,
    pub actual_model_name: String,
    pub display_model_name: String,
    pub live_calls_enabled_by_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeLiveCallGate {
    Allowed,
    DisabledByDefault,
    MissingApiKeyEnv,
    MissingApiKeyValue,
    NetworkApprovalRequired,
    SecretDetected,
    InvalidEndpoint(String),
}

impl NativeProviderEndpoint {
    pub fn deepseek_v4_flash_openai() -> Self {
        Self {
            provider_id: "deepseek-v4-flash-native".to_string(),
            family: NativeModelFamily::DeepSeek,
            protocol: "openai_compatible".to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            api_key_env: "DEEPSEEK_API_KEY".to_string(),
            actual_model_name: "deepseek-v4-flash".to_string(),
            display_model_name: "DeepSeek V4 Flash".to_string(),
            live_calls_enabled_by_default: false,
        }
    }

    pub fn deepseek_v4_flash_anthropic() -> Self {
        Self {
            provider_id: "deepseek-v4-flash-native".to_string(),
            family: NativeModelFamily::DeepSeek,
            protocol: "anthropic_compatible".to_string(),
            base_url: "https://api.deepseek.com/anthropic".to_string(),
            api_key_env: "DEEPSEEK_API_KEY".to_string(),
            actual_model_name: "deepseek-v4-flash".to_string(),
            display_model_name: "DeepSeek V4 Flash".to_string(),
            live_calls_enabled_by_default: false,
        }
    }

    pub fn qwen36_27b_custom_endpoint() -> Self {
        Self {
            provider_id: "qwen3-6-27b-native".to_string(),
            family: NativeModelFamily::Qwen,
            protocol: "openai_compatible".to_string(),
            base_url: "QWEN_BASE_URL".to_string(),
            api_key_env: "QWEN_API_KEY".to_string(),
            actual_model_name: "Qwen/Qwen3.6-27B".to_string(),
            display_model_name: "Qwen3.6-27B".to_string(),
            live_calls_enabled_by_default: false,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.provider_id.trim().is_empty() {
            return Err("provider_id is required".to_string());
        }
        if !matches!(
            self.protocol.as_str(),
            "anthropic_compatible" | "openai_compatible" | "native"
        ) {
            return Err("unsupported native provider protocol".to_string());
        }
        if self.base_url.trim().is_empty() {
            return Err("base_url is required".to_string());
        }
        if self.api_key_env.trim().is_empty() {
            return Err("api_key_env is required".to_string());
        }
        if looks_like_secret(&self.api_key_env) {
            return Err(
                "api_key_env must name an environment variable, not contain a key".to_string(),
            );
        }
        if self.actual_model_name.trim().is_empty() {
            return Err("actual_model_name is required".to_string());
        }
        if self.family == NativeModelFamily::Qwen && !self.actual_model_name.contains("Qwen3.6-27B")
        {
            return Err("Qwen native endpoint must target Qwen3.6-27B".to_string());
        }
        Ok(())
    }
}

pub fn evaluate_native_live_call_gate(
    endpoint: &NativeProviderEndpoint,
    live_calls_enabled: bool,
    network_approved: bool,
) -> NativeLiveCallGate {
    if let Err(error) = endpoint.validate() {
        return NativeLiveCallGate::InvalidEndpoint(error);
    }
    if !live_calls_enabled || !endpoint.live_calls_enabled_by_default {
        return NativeLiveCallGate::DisabledByDefault;
    }
    if endpoint.api_key_env.trim().is_empty() {
        return NativeLiveCallGate::MissingApiKeyEnv;
    }
    if env::var(&endpoint.api_key_env)
        .unwrap_or_default()
        .is_empty()
    {
        return NativeLiveCallGate::MissingApiKeyValue;
    }
    if !network_approved {
        return NativeLiveCallGate::NetworkApprovalRequired;
    }
    NativeLiveCallGate::Allowed
}

fn looks_like_secret(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("sk-") || trimmed.starts_with("AKIA") || trimmed.len() > 80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_anthropic_endpoint_is_valid_but_disabled_by_default() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.validate().unwrap();
        assert_eq!(endpoint.protocol, "anthropic_compatible");
        assert_eq!(endpoint.actual_model_name, "deepseek-v4-flash");
        assert_eq!(
            evaluate_native_live_call_gate(&endpoint, true, true),
            NativeLiveCallGate::DisabledByDefault
        );
    }

    #[test]
    fn deepseek_openai_endpoint_is_valid_for_live_default() {
        let endpoint = NativeProviderEndpoint::deepseek_v4_flash_openai();
        endpoint.validate().unwrap();
        assert_eq!(endpoint.protocol, "openai_compatible");
        assert_eq!(endpoint.base_url, "https://api.deepseek.com");
        assert_eq!(endpoint.actual_model_name, "deepseek-v4-flash");
    }

    #[test]
    fn raw_key_in_api_key_env_is_rejected() {
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.api_key_env = "sk-testsecret".to_string();
        assert!(endpoint.validate().is_err());
    }

    #[test]
    fn qwen_endpoint_requires_qwen36_27b() {
        let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        endpoint.actual_model_name = "Qwen/Qwen2-7B".to_string();
        assert!(endpoint.validate().is_err());
    }

    #[test]
    fn live_gate_requires_api_key_and_network_after_explicit_enable() {
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.live_calls_enabled_by_default = true;
        endpoint.api_key_env = "RESEARCHCODE_TEST_MISSING_API_KEY".to_string();
        assert_eq!(
            evaluate_native_live_call_gate(&endpoint, true, true),
            NativeLiveCallGate::MissingApiKeyValue
        );
    }
}
