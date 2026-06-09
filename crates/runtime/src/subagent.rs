//! Subagent Runtime v1.
//!
//! This module keeps child-agent orchestration inside the runtime boundary so
//! TUI and future GUI clients consume the same event stream instead of
//! reimplementing ClaudeCode/OpenCode-style task delegation in UI code.

use crate::patch::stable_text_hash;
use researchcode_kernel::model::NativeModelFamily;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentType {
    Explorer,
    Reviewer,
    Worker,
    Integrator,
    Judge,
    Reproducer,
}

impl SubagentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Explorer => "explorer",
            Self::Reviewer => "reviewer",
            Self::Worker => "worker",
            Self::Integrator => "integrator",
            Self::Judge => "judge",
            Self::Reproducer => "reproducer",
        }
    }

    pub fn default_tool_allowlist(&self) -> Vec<String> {
        match self {
            Self::Explorer => ["file.read", "search.ripgrep", "repo.map", "git.status"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            Self::Reviewer => [
                "file.read",
                "search.ripgrep",
                "repo.map",
                "git.status",
                "lsp.diagnostics",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            Self::Reproducer => [
                "file.read",
                "search.ripgrep",
                "repo.map",
                "git.status",
                "shell.command",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            Self::Worker => [
                "file.read",
                "search.ripgrep",
                "repo.map",
                "git.status",
                "file.write",
                "file.edit",
                "patch.apply",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            Self::Integrator | Self::Judge => [
                "file.read",
                "search.ripgrep",
                "repo.map",
                "git.status",
                "todo.write",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    pub fn writes_allowed_by_default(&self) -> bool {
        false
    }
}

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
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Running => "running",
            Self::WaitingForPermission => "waiting_for_permission",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPack {
    pub context_pack_id: String,
    pub parent_session_id: String,
    pub summary: String,
    pub evidence_refs: Vec<String>,
    pub content_hash: String,
}

impl ContextPack {
    pub fn new(parent_session_id: impl Into<String>, summary: impl Into<String>) -> Self {
        let parent_session_id = parent_session_id.into();
        let summary = summary.into();
        let content_hash = stable_text_hash(&format!("{parent_session_id}:{summary}"));
        Self {
            context_pack_id: format!("ctx_{content_hash}"),
            parent_session_id,
            summary,
            evidence_refs: Vec::new(),
            content_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentRequest {
    pub agent_type: SubagentType,
    pub task: String,
    pub model_family: NativeModelFamily,
    pub tool_allowlist: Vec<String>,
    pub write_scope: Vec<String>,
    pub worktree_required: bool,
    pub worktree_ready: bool,
    pub context_pack: ContextPack,
}

impl SubagentRequest {
    pub fn readonly(
        parent_session_id: impl Into<String>,
        agent_type: SubagentType,
        task: impl Into<String>,
        model_family: NativeModelFamily,
    ) -> Self {
        let parent_session_id = parent_session_id.into();
        let tool_allowlist = agent_type.default_tool_allowlist();
        Self {
            agent_type,
            task: task.into(),
            model_family,
            tool_allowlist,
            write_scope: Vec::new(),
            worktree_required: false,
            worktree_ready: false,
            context_pack: ContextPack::new(parent_session_id, "readonly context pack"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentSession {
    pub subagent_id: String,
    pub parent_session_id: String,
    pub agent_type: SubagentType,
    pub model_family: NativeModelFamily,
    pub tool_allowlist: Vec<String>,
    pub write_scope: Vec<String>,
    pub context_pack_id: String,
    pub status: SubagentStatus,
    pub event_log_ref: String,
    pub summary: Option<SubagentSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentSummary {
    pub subagent_id: String,
    pub agent_type: SubagentType,
    pub status: SubagentStatus,
    pub summary: String,
    pub evidence_refs: Vec<String>,
}

pub fn validate_subagent_request(request: &SubagentRequest) -> Result<(), String> {
    if request.task.trim().is_empty() {
        return Err("subagent task cannot be empty".to_string());
    }
    if matches!(request.agent_type, SubagentType::Worker)
        && (!request.worktree_required || !request.worktree_ready || request.write_scope.is_empty())
    {
        return Err(
            "agent.worker requires worktree isolation, worktree_ready=true, and non-empty write_scope"
                .to_string(),
        );
    }
    if !matches!(request.agent_type, SubagentType::Worker) && !request.write_scope.is_empty() {
        return Err("read-only subagents cannot receive write_scope".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_is_read_only() {
        let request = SubagentRequest::readonly(
            "parent",
            SubagentType::Explorer,
            "scan docs",
            NativeModelFamily::DeepSeek,
        );
        validate_subagent_request(&request).unwrap();
        assert!(request.tool_allowlist.contains(&"file.read".to_string()));
        assert!(!request.tool_allowlist.contains(&"patch.apply".to_string()));
    }

    #[test]
    fn worker_requires_isolation() {
        let request = SubagentRequest::readonly(
            "parent",
            SubagentType::Worker,
            "edit file",
            NativeModelFamily::DeepSeek,
        );
        assert!(validate_subagent_request(&request).is_err());
    }
}
