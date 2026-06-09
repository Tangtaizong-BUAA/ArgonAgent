//! Multi-agent orchestration policy.
//!
//! Multi-agent execution is policy-driven, not default. This module decides
//! whether a requested parallel run is allowed before any worker is spawned.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultiAgentMode {
    SingleAgent,
    ResearchSwarm,
    SpikeParallel,
    ImplementationShards,
    AdversarialReview,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentWriteScope {
    ReadOnly,
    ReportOnly,
    IsolatedSpike,
    Implementation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiAgentRequest {
    pub mode: MultiAgentMode,
    pub requested_agents: u32,
    pub write_scope: AgentWriteScope,
    pub target_paths: Vec<String>,
    pub interface_frozen: bool,
    pub worktree_isolated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultiAgentDecision {
    Allow,
    ForceSingleAgent,
    DenyForbiddenCorePath(String),
    DenyMissingWorktreeIsolation,
    DenyInterfaceNotFrozen,
    DenyTooManyAgents,
    DenyImplementationInReadOnlyMode,
}

pub fn decide_multi_agent(request: &MultiAgentRequest) -> MultiAgentDecision {
    if request.mode == MultiAgentMode::SingleAgent || request.requested_agents <= 1 {
        return MultiAgentDecision::ForceSingleAgent;
    }
    if request.requested_agents > max_agents_for_mode(&request.mode) {
        return MultiAgentDecision::DenyTooManyAgents;
    }
    for path in &request.target_paths {
        if is_forbidden_core_path(path) {
            return MultiAgentDecision::DenyForbiddenCorePath(path.clone());
        }
    }
    match request.mode {
        MultiAgentMode::ResearchSwarm | MultiAgentMode::AdversarialReview => {
            if matches!(
                request.write_scope,
                AgentWriteScope::Implementation | AgentWriteScope::IsolatedSpike
            ) {
                return MultiAgentDecision::DenyImplementationInReadOnlyMode;
            }
            MultiAgentDecision::Allow
        }
        MultiAgentMode::SpikeParallel => {
            if request.write_scope != AgentWriteScope::IsolatedSpike {
                return MultiAgentDecision::DenyImplementationInReadOnlyMode;
            }
            if !request.worktree_isolated {
                return MultiAgentDecision::DenyMissingWorktreeIsolation;
            }
            MultiAgentDecision::Allow
        }
        MultiAgentMode::ImplementationShards => {
            if !request.interface_frozen {
                return MultiAgentDecision::DenyInterfaceNotFrozen;
            }
            if !request.worktree_isolated {
                return MultiAgentDecision::DenyMissingWorktreeIsolation;
            }
            if request.write_scope != AgentWriteScope::Implementation {
                return MultiAgentDecision::DenyImplementationInReadOnlyMode;
            }
            MultiAgentDecision::Allow
        }
        MultiAgentMode::SingleAgent => MultiAgentDecision::ForceSingleAgent,
    }
}

fn max_agents_for_mode(mode: &MultiAgentMode) -> u32 {
    match mode {
        MultiAgentMode::SingleAgent => 1,
        MultiAgentMode::ResearchSwarm => 4,
        MultiAgentMode::SpikeParallel => 4,
        MultiAgentMode::ImplementationShards => 3,
        MultiAgentMode::AdversarialReview => 2,
    }
}

fn is_forbidden_core_path(path: &str) -> bool {
    const FORBIDDEN_PREFIXES: &[&str] = &[
        "crates/kernel/",
        "crates/runtime/src/permission",
        "crates/runtime/src/patch",
        "crates/runtime/src/model_router",
        "crates/runtime/src/deepseek",
        "crates/runtime/src/qwen",
        "crates/runtime/src/native_provider",
        "docs/security/",
        "docs/schemas/kernel/",
        "docs/storage/",
        "AGENTS.md",
    ];
    FORBIDDEN_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_single_agent() {
        let decision = decide_multi_agent(&MultiAgentRequest {
            mode: MultiAgentMode::SingleAgent,
            requested_agents: 1,
            write_scope: AgentWriteScope::ReadOnly,
            target_paths: vec![],
            interface_frozen: false,
            worktree_isolated: false,
        });
        assert_eq!(decision, MultiAgentDecision::ForceSingleAgent);
    }

    #[test]
    fn allows_research_swarm_read_only() {
        let decision = decide_multi_agent(&MultiAgentRequest {
            mode: MultiAgentMode::ResearchSwarm,
            requested_agents: 3,
            write_scope: AgentWriteScope::ReportOnly,
            target_paths: vec!["docs/analysis/".to_string()],
            interface_frozen: false,
            worktree_isolated: false,
        });
        assert_eq!(decision, MultiAgentDecision::Allow);
    }

    #[test]
    fn blocks_core_path_parallel_writes() {
        let decision = decide_multi_agent(&MultiAgentRequest {
            mode: MultiAgentMode::ImplementationShards,
            requested_agents: 2,
            write_scope: AgentWriteScope::Implementation,
            target_paths: vec!["crates/kernel/src/task.rs".to_string()],
            interface_frozen: true,
            worktree_isolated: true,
        });
        assert!(matches!(
            decision,
            MultiAgentDecision::DenyForbiddenCorePath(_)
        ));
    }

    #[test]
    fn implementation_shards_require_frozen_interfaces_and_worktrees() {
        let not_frozen = decide_multi_agent(&MultiAgentRequest {
            mode: MultiAgentMode::ImplementationShards,
            requested_agents: 2,
            write_scope: AgentWriteScope::Implementation,
            target_paths: vec![
                "apps/desktop/".to_string(),
                "workers/research_worker/".to_string(),
            ],
            interface_frozen: false,
            worktree_isolated: true,
        });
        assert_eq!(not_frozen, MultiAgentDecision::DenyInterfaceNotFrozen);
        let no_worktree = decide_multi_agent(&MultiAgentRequest {
            mode: MultiAgentMode::ImplementationShards,
            requested_agents: 2,
            write_scope: AgentWriteScope::Implementation,
            target_paths: vec![
                "apps/desktop/".to_string(),
                "workers/research_worker/".to_string(),
            ],
            interface_frozen: true,
            worktree_isolated: false,
        });
        assert_eq!(
            no_worktree,
            MultiAgentDecision::DenyMissingWorktreeIsolation
        );
    }
}
