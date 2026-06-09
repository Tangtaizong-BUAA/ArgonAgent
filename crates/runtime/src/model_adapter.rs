//! ModelAdapter skeletons with no network calls.
//!
//! This layer protects the product boundary: DeepSeek and Qwen get native
//! request planning, while all other models stay compatible-only.

use researchcode_kernel::model::{
    CompatibleProviderConfig, NativeModelFamily, NativeModelProfile, OptimizationLevel,
};

use crate::agent_kernel::AgentRole;
use crate::native_profile::deepseek::role_split::{AgentRoleKey, RoleSplit, RoleStage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelRole {
    Planner,
    Executor,
    Reviewer,
    Researcher,
    Summarizer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThinkingMode {
    Thinking,
    NonThinking,
    PreserveThinking,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelAdapterRequest {
    pub role: ModelRole,
    pub task_summary: String,
    pub requires_tools: bool,
    pub context_tokens_estimate: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedModelCall {
    pub adapter_id: String,
    pub optimization_level: OptimizationLevel,
    pub actual_model_name: String,
    pub display_model_name: String,
    pub thinking_mode: ThinkingMode,
    pub native_tool_calls: bool,
    pub stable_prompt_prefix: bool,
    pub parser_profile: String,
    pub max_context_tokens: u64,
    pub notes: Vec<String>,
    pub temperature_milli: Option<u32>,
    pub role_model_name: Option<String>,
    pub agent_role: AgentRole,
    pub role_stage: RoleStage,
}

pub trait ModelAdapter {
    fn adapter_id(&self) -> &str;
    fn plan_call(&self, request: &ModelAdapterRequest) -> Result<PlannedModelCall, String>;
}

#[derive(Debug, Clone)]
pub struct DeepSeekNativeAdapter {
    profile: NativeModelProfile,
    actual_model_name: String,
}

impl DeepSeekNativeAdapter {
    pub fn new(
        profile: NativeModelProfile,
        actual_model_name: impl Into<String>,
    ) -> Result<Self, String> {
        if profile.family != NativeModelFamily::DeepSeek {
            return Err("DeepSeek adapter requires DeepSeek native profile".to_string());
        }
        if profile.optimization_level != OptimizationLevel::Native {
            return Err("DeepSeek adapter requires native optimization".to_string());
        }
        Ok(Self {
            profile,
            actual_model_name: actual_model_name.into(),
        })
    }
}

impl ModelAdapter for DeepSeekNativeAdapter {
    fn adapter_id(&self) -> &str {
        &self.profile.profile_id
    }

    fn plan_call(&self, request: &ModelAdapterRequest) -> Result<PlannedModelCall, String> {
        let thinking_mode = match request.role {
            ModelRole::Executor if !request.requires_tools => ThinkingMode::NonThinking,
            ModelRole::Summarizer => ThinkingMode::NonThinking,
            _ => ThinkingMode::Thinking,
        };
        let split = RoleSplit::deepseek_default();
        let agent_role = agent_role_for_model_role(&request.role);
        let role_stage = role_stage_for_model_role(&request.role, request.requires_tools);
        Ok(PlannedModelCall {
            adapter_id: self.profile.profile_id.clone(),
            optimization_level: OptimizationLevel::Native,
            actual_model_name: self.actual_model_name.clone(),
            display_model_name: "DeepSeek Native".to_string(),
            thinking_mode,
            native_tool_calls: request.requires_tools,
            stable_prompt_prefix: true,
            parser_profile: "deepseek_v4_native_parser".to_string(),
            max_context_tokens: 1_000_000,
            notes: vec![
                "reasoning_content_replay".to_string(),
                "reasoning_content_sanitizer".to_string(),
                "prefix_cache_stable_prompt".to_string(),
                "dsml_xml_fallback".to_string(),
            ],
            temperature_milli: split
                .temperatures
                .get(&role_stage)
                .map(|temperature| (temperature * 1000.0).round() as u32),
            role_model_name: split
                .role_models
                .get(&AgentRoleKey::from(agent_role))
                .map(|model| (*model).to_string()),
            agent_role,
            role_stage,
        })
    }
}

#[derive(Debug, Clone)]
pub struct QwenNativeAdapter {
    profile: NativeModelProfile,
    actual_model_name: String,
}

impl QwenNativeAdapter {
    pub fn new(
        profile: NativeModelProfile,
        actual_model_name: impl Into<String>,
    ) -> Result<Self, String> {
        if profile.family != NativeModelFamily::Qwen {
            return Err("Qwen adapter requires Qwen native profile".to_string());
        }
        if profile.optimization_level != OptimizationLevel::Native {
            return Err("Qwen adapter requires native optimization".to_string());
        }
        let actual_model_name = actual_model_name.into();
        if !actual_model_name.contains("Qwen3.6-27B") {
            return Err("Qwen native adapter canonical target is Qwen3.6-27B".to_string());
        }
        Ok(Self {
            profile,
            actual_model_name,
        })
    }
}

impl ModelAdapter for QwenNativeAdapter {
    fn adapter_id(&self) -> &str {
        &self.profile.profile_id
    }

    fn plan_call(&self, request: &ModelAdapterRequest) -> Result<PlannedModelCall, String> {
        let thinking_mode = match request.role {
            ModelRole::Planner | ModelRole::Reviewer => ThinkingMode::Thinking,
            ModelRole::Executor if request.requires_tools => ThinkingMode::PreserveThinking,
            _ => ThinkingMode::NonThinking,
        };
        let role_stage = role_stage_for_model_role(&request.role, request.requires_tools);
        Ok(PlannedModelCall {
            adapter_id: self.profile.profile_id.clone(),
            optimization_level: OptimizationLevel::Native,
            actual_model_name: self.actual_model_name.clone(),
            display_model_name: "Qwen3.6-27B Native".to_string(),
            thinking_mode,
            native_tool_calls: request.requires_tools,
            stable_prompt_prefix: false,
            parser_profile: "qwen3_6_27b_native_parser".to_string(),
            max_context_tokens: 262_000,
            notes: vec![
                "qwen_specific_chat_template".to_string(),
                "qwen_reasoning_parser".to_string(),
                "patch_sized_edits".to_string(),
                "stale_file_detection".to_string(),
            ],
            temperature_milli: None,
            role_model_name: None,
            agent_role: agent_role_for_model_role(&request.role),
            role_stage,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CompatibleProviderAdapter {
    config: CompatibleProviderConfig,
}

impl CompatibleProviderAdapter {
    pub fn new(config: CompatibleProviderConfig) -> Result<Self, String> {
        config.validate()?;
        Ok(Self { config })
    }
}

impl ModelAdapter for CompatibleProviderAdapter {
    fn adapter_id(&self) -> &str {
        &self.config.provider_id
    }

    fn plan_call(&self, _request: &ModelAdapterRequest) -> Result<PlannedModelCall, String> {
        Ok(PlannedModelCall {
            adapter_id: self.config.provider_id.clone(),
            optimization_level: self.config.optimization_level.clone(),
            actual_model_name: self.config.actual_model_name.clone(),
            display_model_name: self.config.display_model_name.clone(),
            thinking_mode: ThinkingMode::NonThinking,
            native_tool_calls: false,
            stable_prompt_prefix: false,
            parser_profile: "compatible_generic_parser".to_string(),
            max_context_tokens: 32_000,
            notes: vec!["compatible_provider_no_native_optimization".to_string()],
            temperature_milli: None,
            role_model_name: None,
            agent_role: AgentRole::Executor,
            role_stage: RoleStage::Executing,
        })
    }
}

fn agent_role_for_model_role(role: &ModelRole) -> AgentRole {
    match role {
        ModelRole::Planner | ModelRole::Executor | ModelRole::Researcher => AgentRole::Executor,
        ModelRole::Reviewer => AgentRole::Reviewer,
        ModelRole::Summarizer => AgentRole::Summarizer,
    }
}

fn role_stage_for_model_role(role: &ModelRole, requires_tools: bool) -> RoleStage {
    match role {
        ModelRole::Planner => RoleStage::PlanDrafting,
        ModelRole::Reviewer => RoleStage::Reviewing,
        ModelRole::Summarizer => RoleStage::Compacting,
        ModelRole::Executor if requires_tools => RoleStage::Executing,
        ModelRole::Executor => RoleStage::NarrativeAnswer,
        ModelRole::Researcher => RoleStage::Executing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use researchcode_kernel::model::{ProviderCapabilityHints, ProviderHealthCheck};

    fn request(role: ModelRole, requires_tools: bool) -> ModelAdapterRequest {
        ModelAdapterRequest {
            role,
            task_summary: "edit code".to_string(),
            requires_tools,
            context_tokens_estimate: 4_000,
        }
    }

    #[test]
    fn deepseek_native_plan_preserves_native_invariants() {
        let adapter = DeepSeekNativeAdapter::new(
            NativeModelProfile {
                profile_id: "deepseek-v4-native".to_string(),
                family: NativeModelFamily::DeepSeek,
                optimization_level: OptimizationLevel::Native,
            },
            "deepseek-v4",
        )
        .unwrap();
        let plan = adapter
            .plan_call(&request(ModelRole::Planner, true))
            .unwrap();
        assert_eq!(plan.optimization_level, OptimizationLevel::Native);
        assert_eq!(plan.max_context_tokens, 1_000_000);
        assert!(plan.stable_prompt_prefix);
        assert!(plan.notes.contains(&"reasoning_content_replay".to_string()));
        assert_eq!(plan.role_model_name.as_deref(), Some("deepseek-chat"));
        assert_eq!(plan.temperature_milli, Some(500));
    }

    #[test]
    fn qwen_native_plan_requires_canonical_target() {
        let profile = NativeModelProfile {
            profile_id: "qwen3-6-27b-native".to_string(),
            family: NativeModelFamily::Qwen,
            optimization_level: OptimizationLevel::Native,
        };
        assert!(QwenNativeAdapter::new(profile.clone(), "Qwen2-7B").is_err());
        let adapter = QwenNativeAdapter::new(profile, "Qwen/Qwen3.6-27B").unwrap();
        let plan = adapter
            .plan_call(&request(ModelRole::Executor, true))
            .unwrap();
        assert_eq!(plan.max_context_tokens, 262_000);
        assert_eq!(plan.thinking_mode, ThinkingMode::PreserveThinking);
        assert!(plan
            .notes
            .contains(&"qwen_specific_chat_template".to_string()));
    }

    #[test]
    fn compatible_adapter_never_reports_native_optimization() {
        let adapter = CompatibleProviderAdapter::new(CompatibleProviderConfig {
            provider_id: "openai-compatible-lab".to_string(),
            schema_version: "v0".to_string(),
            display_name: "OpenAI Compatible Lab".to_string(),
            protocol: "openai_compatible".to_string(),
            base_url: "http://127.0.0.1:8000/v1".to_string(),
            api_key_env: Some("LAB_API_KEY".to_string()),
            actual_model_name: "custom-model".to_string(),
            display_model_name: "Custom Model".to_string(),
            model_alias: Some("lab-coder".to_string()),
            capability_hints: ProviderCapabilityHints {
                supports_streaming: true,
                supports_tools: true,
                max_context_tokens: 32_000,
            },
            request_transform_id: Some("openai_chat_default_v0".to_string()),
            response_transform_id: Some("openai_chat_default_v0".to_string()),
            health_check: ProviderHealthCheck::default(),
            enabled_by_default: false,
            optimization_level: OptimizationLevel::Compatible,
        })
        .unwrap();
        let plan = adapter
            .plan_call(&request(ModelRole::Planner, true))
            .unwrap();
        assert_eq!(plan.optimization_level, OptimizationLevel::Compatible);
        assert!(!plan.native_tool_calls);
        assert_eq!(plan.parser_profile, "compatible_generic_parser");
    }
}
