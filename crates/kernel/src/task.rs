//! Bounded autonomy contract primitives.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskContract {
    pub task_id: String,
    pub goal: String,
    pub scope: String,
    pub allowed_paths: Vec<String>,
    pub denied_paths: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
    pub max_duration_minutes: u32,
    pub max_retries: u32,
    pub max_parallel_agents: u32,
    pub required_tests: Vec<String>,
    pub required_artifacts: Vec<String>,
    pub stop_conditions: Vec<String>,
    pub reviewer_required: bool,
    pub integrator_required: bool,
}

impl TaskContract {
    pub fn can_write_path(&self, path: &str) -> bool {
        let denied = self
            .denied_paths
            .iter()
            .any(|prefix| path.starts_with(prefix));
        let allowed = self
            .allowed_paths
            .iter()
            .any(|prefix| path.starts_with(prefix));
        allowed && !denied
    }

    pub fn can_use_tool(&self, tool: &str) -> bool {
        if self.denied_tools.iter().any(|denied| denied == tool) {
            return false;
        }
        self.allowed_tools.iter().any(|allowed| allowed == tool)
    }

    pub fn can_retry(&self, retry_count: u32) -> bool {
        retry_count <= self.max_retries
    }

    pub fn can_use_parallel_agents(&self, requested_agents: u32) -> bool {
        requested_agents >= 1 && requested_agents <= self.max_parallel_agents
    }

    pub fn hits_stop_condition(&self, observation: &str) -> bool {
        let lower = observation.to_ascii_lowercase();
        self.stop_conditions
            .iter()
            .any(|condition| lower.contains(&condition.to_ascii_lowercase()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskContractViolation {
    PathDenied,
    ToolDenied,
    TooManyRetries,
    TooManyParallelAgents,
    StopCondition(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskContractCheck {
    pub write_path: Option<String>,
    pub tool: Option<String>,
    pub retry_count: u32,
    pub parallel_agents: u32,
    pub observation: Option<String>,
}

impl TaskContract {
    pub fn validate_action(&self, check: &TaskContractCheck) -> Result<(), TaskContractViolation> {
        if let Some(path) = &check.write_path {
            if !self.can_write_path(path) {
                return Err(TaskContractViolation::PathDenied);
            }
        }
        if let Some(tool) = &check.tool {
            if !self.can_use_tool(tool) {
                return Err(TaskContractViolation::ToolDenied);
            }
        }
        if !self.can_retry(check.retry_count) {
            return Err(TaskContractViolation::TooManyRetries);
        }
        if !self.can_use_parallel_agents(check.parallel_agents) {
            return Err(TaskContractViolation::TooManyParallelAgents);
        }
        if let Some(observation) = &check.observation {
            if self.hits_stop_condition(observation) {
                return Err(TaskContractViolation::StopCondition(observation.clone()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denied_path_wins_over_allowed_path() {
        let contract = TaskContract {
            task_id: "t".to_string(),
            goal: "g".to_string(),
            scope: "s".to_string(),
            allowed_paths: vec!["docs/".to_string()],
            denied_paths: vec!["docs/secret".to_string()],
            allowed_tools: vec!["read".to_string(), "apply_patch".to_string()],
            denied_tools: vec!["network".to_string()],
            max_duration_minutes: 60,
            max_retries: 1,
            max_parallel_agents: 1,
            required_tests: vec![],
            required_artifacts: vec![],
            stop_conditions: vec!["requires network".to_string()],
            reviewer_required: false,
            integrator_required: false,
        };
        assert!(contract.can_write_path("docs/readme.md"));
        assert!(!contract.can_write_path("docs/secret/key.md"));
    }

    #[test]
    fn validates_tool_retry_parallel_and_stop_boundaries() {
        let contract = TaskContract {
            task_id: "t".to_string(),
            goal: "g".to_string(),
            scope: "s".to_string(),
            allowed_paths: vec!["docs/".to_string()],
            denied_paths: vec![".env".to_string()],
            allowed_tools: vec!["read".to_string(), "apply_patch".to_string()],
            denied_tools: vec!["network".to_string()],
            max_duration_minutes: 60,
            max_retries: 1,
            max_parallel_agents: 1,
            required_tests: vec!["python3 scripts/check_all.py".to_string()],
            required_artifacts: vec!["docs/".to_string()],
            stop_conditions: vec!["requires network".to_string()],
            reviewer_required: true,
            integrator_required: false,
        };
        assert!(contract
            .validate_action(&TaskContractCheck {
                write_path: Some("docs/status.md".to_string()),
                tool: Some("apply_patch".to_string()),
                retry_count: 1,
                parallel_agents: 1,
                observation: None,
            })
            .is_ok());
        assert_eq!(
            contract.validate_action(&TaskContractCheck {
                write_path: Some(".env".to_string()),
                tool: Some("read".to_string()),
                retry_count: 0,
                parallel_agents: 1,
                observation: None,
            }),
            Err(TaskContractViolation::PathDenied)
        );
        assert_eq!(
            contract.validate_action(&TaskContractCheck {
                write_path: None,
                tool: Some("network".to_string()),
                retry_count: 0,
                parallel_agents: 1,
                observation: None,
            }),
            Err(TaskContractViolation::ToolDenied)
        );
        assert_eq!(
            contract.validate_action(&TaskContractCheck {
                write_path: None,
                tool: Some("read".to_string()),
                retry_count: 2,
                parallel_agents: 1,
                observation: None,
            }),
            Err(TaskContractViolation::TooManyRetries)
        );
        assert!(matches!(
            contract.validate_action(&TaskContractCheck {
                write_path: None,
                tool: Some("read".to_string()),
                retry_count: 0,
                parallel_agents: 1,
                observation: Some("This requires network access".to_string()),
            }),
            Err(TaskContractViolation::StopCondition(_))
        ));
    }
}
