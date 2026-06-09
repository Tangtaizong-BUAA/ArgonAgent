//! Plan and PlanStep primitives for task governance.

use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    pub step_id: String,
    pub title: String,
    pub goal: String,
    pub allowed_tools: Vec<String>,
    pub expected_artifacts: Vec<String>,
    pub status: PlanStepStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub plan_id: String,
    pub task_id: String,
    pub summary: String,
    pub steps: Vec<PlanStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanValidationError {
    MissingPlanId,
    MissingTaskId,
    MissingSummary,
    EmptySteps,
    TooManySteps,
    MissingStepId,
    DuplicateStepId(String),
    MissingStepTitle(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanProgress {
    pub total: usize,
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub blocked: usize,
    pub failed: usize,
}

impl Plan {
    pub fn validate(&self) -> Result<(), PlanValidationError> {
        if self.plan_id.trim().is_empty() {
            return Err(PlanValidationError::MissingPlanId);
        }
        if self.task_id.trim().is_empty() {
            return Err(PlanValidationError::MissingTaskId);
        }
        if self.summary.trim().is_empty() {
            return Err(PlanValidationError::MissingSummary);
        }
        if self.steps.is_empty() {
            return Err(PlanValidationError::EmptySteps);
        }
        if self.steps.len() > 20 {
            return Err(PlanValidationError::TooManySteps);
        }
        let mut seen = HashSet::new();
        for step in &self.steps {
            if step.step_id.trim().is_empty() {
                return Err(PlanValidationError::MissingStepId);
            }
            if !seen.insert(step.step_id.clone()) {
                return Err(PlanValidationError::DuplicateStepId(step.step_id.clone()));
            }
            if step.title.trim().is_empty() {
                return Err(PlanValidationError::MissingStepTitle(step.step_id.clone()));
            }
        }
        Ok(())
    }

    pub fn progress(&self) -> PlanProgress {
        let mut progress = PlanProgress {
            total: self.steps.len(),
            pending: 0,
            in_progress: 0,
            completed: 0,
            blocked: 0,
            failed: 0,
        };
        for step in &self.steps {
            match step.status {
                PlanStepStatus::Pending => progress.pending += 1,
                PlanStepStatus::InProgress => progress.in_progress += 1,
                PlanStepStatus::Completed => progress.completed += 1,
                PlanStepStatus::Blocked => progress.blocked += 1,
                PlanStepStatus::Failed => progress.failed += 1,
            }
        }
        progress
    }

    pub fn next_actionable_step(&self) -> Option<&PlanStep> {
        self.steps
            .iter()
            .find(|step| matches!(step.status, PlanStepStatus::InProgress))
            .or_else(|| {
                self.steps
                    .iter()
                    .find(|step| matches!(step.status, PlanStepStatus::Pending))
            })
    }

    pub fn to_context_text(&self) -> String {
        let mut lines = vec![
            format!("Plan: {}", self.summary),
            format!("plan_id={} task_id={}", self.plan_id, self.task_id),
        ];
        for step in &self.steps {
            lines.push(format!(
                "- [{}] {}: {} tools={} artifacts={}",
                plan_step_status_to_str(&step.status),
                step.step_id,
                step.title,
                step.allowed_tools.join(","),
                step.expected_artifacts.join(",")
            ));
        }
        lines.join("\n")
    }
}

pub fn plan_step_status_to_str(status: &PlanStepStatus) -> &'static str {
    match status {
        PlanStepStatus::Pending => "pending",
        PlanStepStatus::InProgress => "in_progress",
        PlanStepStatus::Completed => "completed",
        PlanStepStatus::Blocked => "blocked",
        PlanStepStatus::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_plan_and_progress() {
        let plan = sample_plan();
        assert_eq!(plan.validate(), Ok(()));
        let progress = plan.progress();
        assert_eq!(progress.total, 3);
        assert_eq!(progress.completed, 1);
        assert_eq!(progress.in_progress, 1);
        assert_eq!(
            plan.next_actionable_step()
                .map(|step| step.step_id.as_str()),
            Some("step_2")
        );
        assert!(plan.to_context_text().contains("Plan:"));
    }

    #[test]
    fn rejects_duplicate_step_ids() {
        let mut plan = sample_plan();
        plan.steps[2].step_id = "step_2".to_string();
        assert_eq!(
            plan.validate(),
            Err(PlanValidationError::DuplicateStepId("step_2".to_string()))
        );
    }

    fn sample_plan() -> Plan {
        Plan {
            plan_id: "plan_1".to_string(),
            task_id: "task_1".to_string(),
            summary: "Harden command execution".to_string(),
            steps: vec![
                PlanStep {
                    step_id: "step_1".to_string(),
                    title: "Inspect current command boundary".to_string(),
                    goal: "Find tokenizer and permission gaps".to_string(),
                    allowed_tools: vec!["file.read".to_string(), "search.ripgrep".to_string()],
                    expected_artifacts: vec!["notes".to_string()],
                    status: PlanStepStatus::Completed,
                },
                PlanStep {
                    step_id: "step_2".to_string(),
                    title: "Patch tokenizer".to_string(),
                    goal: "Preserve quoted args without invoking shell".to_string(),
                    allowed_tools: vec!["patch.apply".to_string()],
                    expected_artifacts: vec!["diff".to_string()],
                    status: PlanStepStatus::InProgress,
                },
                PlanStep {
                    step_id: "step_3".to_string(),
                    title: "Run tests".to_string(),
                    goal: "Verify classifier and executor behavior".to_string(),
                    allowed_tools: vec!["shell.command".to_string()],
                    expected_artifacts: vec!["test log".to_string()],
                    status: PlanStepStatus::Pending,
                },
            ],
        }
    }
}
