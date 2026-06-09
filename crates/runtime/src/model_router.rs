//! Native-first ModelRouter skeleton.

use crate::model_adapter::{
    CompatibleProviderAdapter, DeepSeekNativeAdapter, ModelAdapter, ModelAdapterRequest,
    PlannedModelCall, QwenNativeAdapter,
};
use researchcode_kernel::model::{
    CompatibleProviderConfig, NativeModelFamily, NativeModelProfile, OptimizationLevel,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSelection {
    DeepSeekNative { actual_model_name: String },
    QwenNative { actual_model_name: String },
    Compatible { provider: CompatibleProviderConfig },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelRouterError {
    InvalidNativeProfile(String),
    CompatibleProviderCannotBeNative,
}

pub fn route_model_call(
    selection: ModelSelection,
    request: &ModelAdapterRequest,
) -> Result<PlannedModelCall, ModelRouterError> {
    match selection {
        ModelSelection::DeepSeekNative { actual_model_name } => {
            let adapter = DeepSeekNativeAdapter::new(
                NativeModelProfile {
                    profile_id: "deepseek-v4-native".to_string(),
                    family: NativeModelFamily::DeepSeek,
                    optimization_level: OptimizationLevel::Native,
                },
                actual_model_name,
            )
            .map_err(ModelRouterError::InvalidNativeProfile)?;
            adapter
                .plan_call(request)
                .map_err(ModelRouterError::InvalidNativeProfile)
        }
        ModelSelection::QwenNative { actual_model_name } => {
            let adapter = QwenNativeAdapter::new(
                NativeModelProfile {
                    profile_id: "qwen3-6-27b-native".to_string(),
                    family: NativeModelFamily::Qwen,
                    optimization_level: OptimizationLevel::Native,
                },
                actual_model_name,
            )
            .map_err(ModelRouterError::InvalidNativeProfile)?;
            adapter
                .plan_call(request)
                .map_err(ModelRouterError::InvalidNativeProfile)
        }
        ModelSelection::Compatible { provider } => {
            if provider.optimization_level == OptimizationLevel::Native {
                return Err(ModelRouterError::CompatibleProviderCannotBeNative);
            }
            let adapter = CompatibleProviderAdapter::new(provider)
                .map_err(|_| ModelRouterError::CompatibleProviderCannotBeNative)?;
            adapter
                .plan_call(request)
                .map_err(|_| ModelRouterError::CompatibleProviderCannotBeNative)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_adapter::ModelRole;
    use researchcode_kernel::model::{ProviderCapabilityHints, ProviderHealthCheck};

    fn request() -> ModelAdapterRequest {
        ModelAdapterRequest {
            role: ModelRole::Planner,
            task_summary: "plan task".to_string(),
            requires_tools: true,
            context_tokens_estimate: 1000,
        }
    }

    #[test]
    fn routes_deepseek_and_qwen_to_native_profiles() {
        let deepseek = route_model_call(
            ModelSelection::DeepSeekNative {
                actual_model_name: "deepseek-v4".to_string(),
            },
            &request(),
        )
        .unwrap();
        assert_eq!(deepseek.optimization_level, OptimizationLevel::Native);
        assert_eq!(deepseek.parser_profile, "deepseek_v4_native_parser");

        let qwen = route_model_call(
            ModelSelection::QwenNative {
                actual_model_name: "Qwen/Qwen3.6-27B".to_string(),
            },
            &request(),
        )
        .unwrap();
        assert_eq!(qwen.optimization_level, OptimizationLevel::Native);
        assert_eq!(qwen.parser_profile, "qwen3_6_27b_native_parser");
    }

    #[test]
    fn compatible_provider_cannot_be_native() {
        let result = route_model_call(
            ModelSelection::Compatible {
                provider: CompatibleProviderConfig {
                    provider_id: "bad".to_string(),
                    schema_version: "v0".to_string(),
                    display_name: "Bad".to_string(),
                    protocol: "openai_compatible".to_string(),
                    base_url: "http://127.0.0.1:8000/v1".to_string(),
                    api_key_env: Some("BAD_API_KEY".to_string()),
                    actual_model_name: "custom".to_string(),
                    display_model_name: "Custom".to_string(),
                    model_alias: None,
                    capability_hints: ProviderCapabilityHints::default(),
                    request_transform_id: None,
                    response_transform_id: None,
                    health_check: ProviderHealthCheck::default(),
                    enabled_by_default: false,
                    optimization_level: OptimizationLevel::Native,
                },
            },
            &request(),
        );
        assert_eq!(
            result,
            Err(ModelRouterError::CompatibleProviderCannotBeNative)
        );
    }
}
