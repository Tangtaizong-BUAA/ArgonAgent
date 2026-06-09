//! Model scope primitives.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeModelFamily {
    DeepSeek,
    Qwen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallingReliability {
    Stable,
    ModerateUnreliability,
    NotRecommended,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeProtocol {
    AnthropicCompatible,
    OpenAiCompatible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepSeekVariant {
    V3,
    V31,
    V32Exp,
    V4Pro,
    V4Flash,
    V4Native,
    R1,
    CoderV2,
    Unknown(String),
}

impl DeepSeekVariant {
    pub fn from_model_name(name: &str) -> Self {
        let lower = name.to_lowercase();
        if lower.contains("r1") {
            Self::R1
        } else if lower.contains("v4-flash")
            || lower.contains("v4 flash")
            || lower.contains("chat-flash")
        {
            Self::V4Flash
        } else if lower.contains("v4-pro")
            || lower.contains("v4 pro")
            || lower == "reasoner"
            || lower == "deepseek-reasoner"
            || lower == "deepseek-chat"
        {
            Self::V4Pro
        } else if lower.contains("v4") {
            Self::V4Native
        } else if lower.contains("v3.1") || lower.contains("v31") {
            Self::V31
        } else if lower.contains("v3.2") || lower.contains("v32") {
            Self::V32Exp
        } else if lower.contains("v3") {
            Self::V3
        } else if lower.contains("coder") && lower.contains("v2") {
            Self::CoderV2
        } else {
            Self::Unknown(name.to_string())
        }
    }

    pub fn capabilities(&self) -> DeepSeekCapabilities {
        match self {
            Self::V3 => DeepSeekCapabilities {
                native_tool_calling: ToolCallingReliability::ModerateUnreliability,
                reasoning: false,
                max_context_tokens: 128_000,
                preferred_protocol: NativeProtocol::OpenAiCompatible,
                supports_context_caching: false,
                supports_fim: false,
            },
            Self::V31 => DeepSeekCapabilities {
                native_tool_calling: ToolCallingReliability::Stable,
                reasoning: false,
                max_context_tokens: 128_000,
                preferred_protocol: NativeProtocol::AnthropicCompatible,
                supports_context_caching: true,
                supports_fim: false,
            },
            Self::V32Exp => DeepSeekCapabilities {
                native_tool_calling: ToolCallingReliability::Stable,
                reasoning: true,
                max_context_tokens: 128_000,
                preferred_protocol: NativeProtocol::AnthropicCompatible,
                supports_context_caching: true,
                supports_fim: false,
            },
            Self::V4Pro | Self::V4Flash | Self::V4Native => DeepSeekCapabilities {
                native_tool_calling: ToolCallingReliability::Stable,
                reasoning: true,
                max_context_tokens: 128_000,
                preferred_protocol: NativeProtocol::AnthropicCompatible,
                supports_context_caching: true,
                supports_fim: false,
            },
            Self::R1 => DeepSeekCapabilities {
                native_tool_calling: ToolCallingReliability::NotRecommended,
                reasoning: true,
                max_context_tokens: 128_000,
                preferred_protocol: NativeProtocol::AnthropicCompatible,
                supports_context_caching: false,
                supports_fim: false,
            },
            Self::CoderV2 => DeepSeekCapabilities {
                native_tool_calling: ToolCallingReliability::Stable,
                reasoning: false,
                max_context_tokens: 16_000,
                preferred_protocol: NativeProtocol::OpenAiCompatible,
                supports_context_caching: false,
                supports_fim: true,
            },
            Self::Unknown(_) => DeepSeekCapabilities {
                native_tool_calling: ToolCallingReliability::Unknown,
                reasoning: false,
                max_context_tokens: 64_000,
                preferred_protocol: NativeProtocol::OpenAiCompatible,
                supports_context_caching: false,
                supports_fim: false,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepSeekCapabilities {
    pub native_tool_calling: ToolCallingReliability,
    pub reasoning: bool,
    pub max_context_tokens: u64,
    pub preferred_protocol: NativeProtocol,
    pub supports_context_caching: bool,
    pub supports_fim: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptimizationLevel {
    Native,
    Compatible,
    Baseline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeModelProfile {
    pub profile_id: String,
    pub family: NativeModelFamily,
    pub optimization_level: OptimizationLevel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibleProviderConfig {
    pub provider_id: String,
    pub schema_version: String,
    pub display_name: String,
    pub protocol: String,
    pub base_url: String,
    pub api_key_env: Option<String>,
    pub actual_model_name: String,
    pub display_model_name: String,
    pub model_alias: Option<String>,
    pub capability_hints: ProviderCapabilityHints,
    pub request_transform_id: Option<String>,
    pub response_transform_id: Option<String>,
    pub health_check: ProviderHealthCheck,
    pub enabled_by_default: bool,
    pub optimization_level: OptimizationLevel,
}

impl CompatibleProviderConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.optimization_level == OptimizationLevel::Native {
            return Err("compatible provider cannot be native".to_string());
        }
        if self.actual_model_name.trim().is_empty() {
            return Err("actual_model_name is required".to_string());
        }
        if self.display_model_name.trim().is_empty() {
            return Err("display_model_name is required".to_string());
        }
        if self.schema_version.trim().is_empty() {
            return Err("schema_version is required".to_string());
        }
        if self.base_url.trim().is_empty() {
            return Err("base_url is required".to_string());
        }
        if !matches!(
            self.protocol.as_str(),
            "openai_compatible" | "anthropic_compatible" | "custom"
        ) {
            return Err("unsupported compatible provider protocol".to_string());
        }
        if let Some(api_key_env) = &self.api_key_env {
            if api_key_env.trim().is_empty() {
                return Err("api_key_env cannot be empty when set".to_string());
            }
            if looks_like_secret(api_key_env) {
                return Err(
                    "api_key_env must name an environment variable, not contain a key".to_string(),
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilityHints {
    pub supports_streaming: bool,
    pub supports_tools: bool,
    pub max_context_tokens: u64,
}

impl Default for ProviderCapabilityHints {
    fn default() -> Self {
        Self {
            supports_streaming: false,
            supports_tools: false,
            max_context_tokens: 32_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHealthCheck {
    pub enabled: bool,
    pub timeout_ms: u64,
    pub requires_auth: bool,
}

impl Default for ProviderHealthCheck {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_ms: 5_000,
            requires_auth: true,
        }
    }
}

fn looks_like_secret(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("sk-") || trimmed.starts_with("AKIA") || trimmed.len() > 80
}

#[derive(Debug, Default)]
pub struct ModelRegistry {
    native_profiles: Vec<NativeModelProfile>,
    compatible_providers: Vec<CompatibleProviderConfig>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_native_profile(&mut self, profile: NativeModelProfile) -> Result<(), String> {
        if profile.optimization_level != OptimizationLevel::Native {
            return Err("native profile must use native optimization".to_string());
        }
        self.native_profiles.push(profile);
        Ok(())
    }

    pub fn add_compatible_provider(
        &mut self,
        provider: CompatibleProviderConfig,
    ) -> Result<(), String> {
        provider.validate()?;
        self.compatible_providers.push(provider);
        Ok(())
    }

    pub fn native_profile_count(&self) -> usize {
        self.native_profiles.len()
    }

    pub fn compatible_provider_count(&self) -> usize {
        self.compatible_providers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_v4_model_names_are_classified() {
        assert_eq!(
            DeepSeekVariant::from_model_name("deepseek-v4-flash"),
            DeepSeekVariant::V4Flash
        );
        assert_eq!(
            DeepSeekVariant::from_model_name("DeepSeek V4 Pro"),
            DeepSeekVariant::V4Pro
        );
        assert_eq!(
            DeepSeekVariant::from_model_name("deepseek-chat"),
            DeepSeekVariant::V4Pro
        );
        assert_eq!(
            DeepSeekVariant::from_model_name("deepseek-v4"),
            DeepSeekVariant::V4Native
        );
    }

    #[test]
    fn deepseek_v4_capabilities_enable_reasoning_and_cache() {
        let capabilities = DeepSeekVariant::V4Flash.capabilities();
        assert_eq!(
            capabilities.native_tool_calling,
            ToolCallingReliability::Stable
        );
        assert!(capabilities.reasoning);
        assert!(capabilities.supports_context_caching);
        assert_eq!(capabilities.max_context_tokens, 128_000);
    }

    #[test]
    fn compatible_provider_cannot_be_native() {
        let config = CompatibleProviderConfig {
            provider_id: "lab".to_string(),
            schema_version: "v0".to_string(),
            display_name: "Lab".to_string(),
            protocol: "openai_compatible".to_string(),
            base_url: "http://127.0.0.1:8000/v1".to_string(),
            api_key_env: Some("LAB_API_KEY".to_string()),
            actual_model_name: "custom".to_string(),
            display_model_name: "Custom".to_string(),
            model_alias: Some("lab-custom".to_string()),
            capability_hints: ProviderCapabilityHints::default(),
            request_transform_id: Some("openai_chat_default_v0".to_string()),
            response_transform_id: Some("openai_chat_default_v0".to_string()),
            health_check: ProviderHealthCheck::default(),
            enabled_by_default: false,
            optimization_level: OptimizationLevel::Native,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn compatible_provider_rejects_secret_value_in_api_key_env() {
        let config = CompatibleProviderConfig {
            provider_id: "lab".to_string(),
            schema_version: "v0".to_string(),
            display_name: "Lab".to_string(),
            protocol: "openai_compatible".to_string(),
            base_url: "http://127.0.0.1:8000/v1".to_string(),
            api_key_env: Some("sk-testsecret".to_string()),
            actual_model_name: "custom".to_string(),
            display_model_name: "Custom".to_string(),
            model_alias: None,
            capability_hints: ProviderCapabilityHints::default(),
            request_transform_id: None,
            response_transform_id: None,
            health_check: ProviderHealthCheck::default(),
            enabled_by_default: false,
            optimization_level: OptimizationLevel::Compatible,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn registry_separates_native_and_compatible() {
        let mut registry = ModelRegistry::new();
        registry
            .add_native_profile(NativeModelProfile {
                profile_id: "deepseek-v4-native".to_string(),
                family: NativeModelFamily::DeepSeek,
                optimization_level: OptimizationLevel::Native,
            })
            .unwrap();
        registry
            .add_compatible_provider(CompatibleProviderConfig {
                provider_id: "lab".to_string(),
                schema_version: "v0".to_string(),
                display_name: "Lab".to_string(),
                protocol: "openai_compatible".to_string(),
                base_url: "http://127.0.0.1:8000/v1".to_string(),
                api_key_env: Some("LAB_API_KEY".to_string()),
                actual_model_name: "custom".to_string(),
                display_model_name: "Custom".to_string(),
                model_alias: Some("lab-custom".to_string()),
                capability_hints: ProviderCapabilityHints::default(),
                request_transform_id: Some("openai_chat_default_v0".to_string()),
                response_transform_id: Some("openai_chat_default_v0".to_string()),
                health_check: ProviderHealthCheck::default(),
                enabled_by_default: false,
                optimization_level: OptimizationLevel::Compatible,
            })
            .unwrap();
        assert_eq!(registry.native_profile_count(), 1);
        assert_eq!(registry.compatible_provider_count(), 1);
    }
}
