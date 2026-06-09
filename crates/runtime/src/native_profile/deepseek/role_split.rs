use std::collections::BTreeMap;

use crate::agent_kernel::AgentRole;

#[derive(Debug, Clone, PartialEq)]
pub struct RoleSplit {
    pub role_models: BTreeMap<AgentRoleKey, &'static str>,
    pub temperatures: BTreeMap<RoleStage, f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentRoleKey {
    Executor,
    Compactor,
    Reviewer,
    Titler,
    Summarizer,
}

impl From<AgentRole> for AgentRoleKey {
    fn from(value: AgentRole) -> Self {
        match value {
            AgentRole::Executor => AgentRoleKey::Executor,
            AgentRole::Compactor => AgentRoleKey::Compactor,
            AgentRole::Reviewer => AgentRoleKey::Reviewer,
            AgentRole::Titler => AgentRoleKey::Titler,
            AgentRole::Summarizer => AgentRoleKey::Summarizer,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RoleStage {
    Routing,
    PlanDrafting,
    Executing,
    Reviewing,
    Compacting,
    NarrativeAnswer,
}

impl RoleSplit {
    pub fn deepseek_default() -> Self {
        let mut role_models = BTreeMap::new();
        role_models.insert(AgentRoleKey::Executor, "deepseek-chat");
        role_models.insert(AgentRoleKey::Compactor, "deepseek-chat-flash");
        role_models.insert(AgentRoleKey::Reviewer, "deepseek-chat");
        role_models.insert(AgentRoleKey::Titler, "deepseek-chat-flash");
        role_models.insert(AgentRoleKey::Summarizer, "deepseek-chat-flash");

        let mut temperatures = BTreeMap::new();
        temperatures.insert(RoleStage::Routing, 0.0);
        temperatures.insert(RoleStage::PlanDrafting, 0.5);
        temperatures.insert(RoleStage::Executing, 0.2);
        temperatures.insert(RoleStage::Reviewing, 0.0);
        temperatures.insert(RoleStage::Compacting, 0.0);
        temperatures.insert(RoleStage::NarrativeAnswer, 0.7);

        Self {
            role_models,
            temperatures,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_compactor_uses_flash_by_default() {
        let split = RoleSplit::deepseek_default();
        assert_eq!(
            split.role_models.get(&AgentRoleKey::Compactor),
            Some(&"deepseek-chat-flash")
        );
        assert_eq!(split.temperatures.get(&RoleStage::Executing), Some(&0.2));
    }
}
