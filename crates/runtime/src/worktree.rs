//! Worktree isolation planner.
//!
//! V0 validates and plans per-agent worktree paths without invoking git. Real
//! `git worktree add` execution must stay behind command permission.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRequest {
    pub project_root: PathBuf,
    pub worktree_root: PathBuf,
    pub agent_id: String,
    pub branch_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreePlan {
    pub agent_id: String,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub git_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreePlanError {
    ProjectRootMissing,
    NotGitRepository,
    UnsafeAgentId,
    UnsafeBranchName,
    WorktreeEscapesRoot,
    DuplicateWorktreePath,
}

pub fn plan_worktree(request: &WorktreeRequest) -> Result<WorktreePlan, WorktreePlanError> {
    if !request.project_root.exists() {
        return Err(WorktreePlanError::ProjectRootMissing);
    }
    if !request.project_root.join(".git").exists() {
        return Err(WorktreePlanError::NotGitRepository);
    }
    if !is_safe_name(&request.agent_id) {
        return Err(WorktreePlanError::UnsafeAgentId);
    }
    if !is_safe_branch(&request.branch_name) {
        return Err(WorktreePlanError::UnsafeBranchName);
    }
    let worktree_path = request.worktree_root.join(&request.agent_id);
    if path_escapes_root(&worktree_path, &request.worktree_root) {
        return Err(WorktreePlanError::WorktreeEscapesRoot);
    }
    Ok(WorktreePlan {
        agent_id: request.agent_id.clone(),
        branch_name: request.branch_name.clone(),
        worktree_path: worktree_path.clone(),
        git_args: vec![
            "worktree".to_string(),
            "add".to_string(),
            worktree_path.to_string_lossy().to_string(),
            request.branch_name.clone(),
        ],
    })
}

pub fn validate_disjoint_worktrees(plans: &[WorktreePlan]) -> Result<(), WorktreePlanError> {
    for (index, left) in plans.iter().enumerate() {
        for right in plans.iter().skip(index + 1) {
            if left.worktree_path == right.worktree_path {
                return Err(WorktreePlanError::DuplicateWorktreePath);
            }
        }
    }
    Ok(())
}

fn is_safe_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
}

fn is_safe_branch(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.starts_with('-')
        && !value.contains("..")
        && !value.contains('@')
        && !value.contains('\\')
        && !value.contains(' ')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '/' | '.'))
}

fn path_escapes_root(path: &Path, root: &Path) -> bool {
    let normalized = normalize(path);
    let normalized_root = normalize(root);
    normalized != normalized_root && !normalized.starts_with(&normalized_root)
}

fn normalize(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                output.pop();
            }
            std::path::Component::CurDir => {}
            other => output.push(other.as_os_str()),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn plans_safe_worktree_without_running_git() {
        let root = temp_git_root("worktree-plan");
        let request = WorktreeRequest {
            project_root: root.join("repo"),
            worktree_root: root.join("worktrees"),
            agent_id: "agent_1".to_string(),
            branch_name: "agent/agent_1".to_string(),
        };
        fs::create_dir_all(&request.worktree_root).unwrap();
        let plan = plan_worktree(&request).unwrap();
        assert_eq!(plan.git_args[0], "worktree");
        assert!(plan.worktree_path.ends_with("agent_1"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_non_git_and_unsafe_names() {
        let root = temp_root("worktree-reject");
        let non_git = WorktreeRequest {
            project_root: root.join("repo"),
            worktree_root: root.join("worktrees"),
            agent_id: "agent_1".to_string(),
            branch_name: "agent/agent_1".to_string(),
        };
        fs::create_dir_all(&non_git.project_root).unwrap();
        assert_eq!(
            plan_worktree(&non_git),
            Err(WorktreePlanError::NotGitRepository)
        );
        let git_root = temp_git_root("worktree-unsafe");
        let unsafe_agent = WorktreeRequest {
            project_root: git_root.join("repo"),
            worktree_root: git_root.join("worktrees"),
            agent_id: "../escape".to_string(),
            branch_name: "agent/good".to_string(),
        };
        assert_eq!(
            plan_worktree(&unsafe_agent),
            Err(WorktreePlanError::UnsafeAgentId)
        );
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(git_root);
    }

    #[test]
    fn rejects_duplicate_worktree_paths() {
        let plan = WorktreePlan {
            agent_id: "agent_1".to_string(),
            branch_name: "a".to_string(),
            worktree_path: PathBuf::from("/tmp/wt/agent_1"),
            git_args: vec![],
        };
        assert_eq!(
            validate_disjoint_worktrees(&[plan.clone(), plan]),
            Err(WorktreePlanError::DuplicateWorktreePath)
        );
    }

    fn temp_git_root(prefix: &str) -> PathBuf {
        let root = temp_root(prefix);
        fs::create_dir_all(root.join("repo/.git")).unwrap();
        root
    }

    fn temp_root(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("researchcode-{prefix}-{nonce}"))
    }
}
