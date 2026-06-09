//! Kernel-level subagent specifications.

use crate::message::ContentBlock;
use crate::model::NativeModelProfile;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentSpec {
    pub spec_id: String,
    pub parent_session_id: String,
    pub system_prompt: String,
    pub allowed_tools: Vec<String>,
    pub budget: SubagentBudget,
    pub model_override: Option<NativeModelProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentBudget {
    pub max_turns: u32,
    pub max_input_tokens: u64,
    pub max_tool_calls: u32,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentResult {
    Completed {
        summary: String,
        output_blocks: Vec<ContentBlock>,
    },
    BudgetExhausted {
        reason: String,
        partial: String,
    },
    Cancelled,
    Failed {
        error: String,
    },
}

impl SubagentSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.spec_id.trim().is_empty() {
            return Err("subagent spec_id is required".to_string());
        }
        if self.parent_session_id.trim().is_empty() {
            return Err("parent_session_id is required".to_string());
        }
        if self.system_prompt.trim().is_empty() {
            return Err("system_prompt is required".to_string());
        }
        self.budget.validate()?;
        Ok(())
    }
}

impl SubagentBudget {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_turns == 0 {
            return Err("subagent max_turns must be positive".to_string());
        }
        if self.max_input_tokens == 0 {
            return Err("subagent max_input_tokens must be positive".to_string());
        }
        if self.max_tool_calls == 0 {
            return Err("subagent max_tool_calls must be positive".to_string());
        }
        if self.timeout.is_zero() {
            return Err("subagent timeout must be positive".to_string());
        }
        Ok(())
    }
}

// ── SubagentSession lifecycle ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentStatus {
    Created,
    Running,
    WaitingForPermission,
    Completed,
    Failed,
    Cancelled,
}

impl SubagentStatus {
    /// Check if this status is terminal (no further transitions possible).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentSession {
    pub session_id: String,
    pub spec: SubagentSpec,
    pub status: SubagentStatus,
}

impl SubagentSession {
    pub fn new(session_id: String, spec: SubagentSpec) -> Self {
        Self {
            session_id,
            spec,
            status: SubagentStatus::Created,
        }
    }

    /// Transition to a target status, failing if the transition is illegal.
    pub fn transition_to(&mut self, target: SubagentStatus) -> Result<(), String> {
        let valid = match (&self.status, &target) {
            // Created → Running | Cancelled
            (SubagentStatus::Created, SubagentStatus::Running)
            | (SubagentStatus::Created, SubagentStatus::Cancelled) => true,
            // Running → WaitingForPermission | Completed | Failed | Cancelled
            (SubagentStatus::Running, SubagentStatus::WaitingForPermission)
            | (SubagentStatus::Running, SubagentStatus::Completed)
            | (SubagentStatus::Running, SubagentStatus::Failed)
            | (SubagentStatus::Running, SubagentStatus::Cancelled) => true,
            // WaitingForPermission → Running | Cancelled
            (SubagentStatus::WaitingForPermission, SubagentStatus::Running)
            | (SubagentStatus::WaitingForPermission, SubagentStatus::Cancelled) => true,
            // Terminal states → no transitions allowed
            _ => false,
        };
        if !valid {
            return Err(format!(
                "illegal SubagentSession transition: {:?} -> {:?}",
                self.status, target
            ));
        }
        self.status = target;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subagent_spec_requires_explicit_budget() {
        let spec = SubagentSpec {
            spec_id: "explorer".to_string(),
            parent_session_id: "session_parent".to_string(),
            system_prompt: "Inspect files only.".to_string(),
            allowed_tools: vec!["file.read".to_string(), "search.ripgrep".to_string()],
            budget: SubagentBudget {
                max_turns: 3,
                max_input_tokens: 16_000,
                max_tool_calls: 10,
                timeout: Duration::from_secs(60),
            },
            model_override: None,
        };

        assert!(spec.validate().is_ok());
    }

    #[test]
    fn subagent_budget_rejects_unbounded_shape() {
        let budget = SubagentBudget {
            max_turns: 0,
            max_input_tokens: 16_000,
            max_tool_calls: 10,
            timeout: Duration::from_secs(60),
        };

        assert!(budget.validate().is_err());
    }
}
