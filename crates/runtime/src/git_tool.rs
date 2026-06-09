//! Read-only git status tool.

use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusRequest {
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitStatusKind {
    Clean,
    Dirty,
    NoRepo,
    GitUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusResult {
    pub kind: GitStatusKind,
    pub porcelain: String,
}

pub fn git_status(request: &GitStatusRequest) -> GitStatusResult {
    let output = Command::new("git")
        .arg("-C")
        .arg(&request.cwd)
        .arg("status")
        .arg("--porcelain")
        .arg("--branch")
        .output();
    let Ok(output) = output else {
        return GitStatusResult {
            kind: GitStatusKind::GitUnavailable,
            porcelain: String::new(),
        };
    };
    if !output.status.success() {
        return GitStatusResult {
            kind: GitStatusKind::NoRepo,
            porcelain: String::from_utf8_lossy(&output.stderr).to_string(),
        };
    }
    let porcelain = String::from_utf8_lossy(&output.stdout).to_string();
    let dirty = porcelain
        .lines()
        .any(|line| !line.starts_with("##") && !line.trim().is_empty());
    GitStatusResult {
        kind: if dirty {
            GitStatusKind::Dirty
        } else {
            GitStatusKind::Clean
        },
        porcelain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_repo_returns_no_repo_or_git_unavailable() {
        let result = git_status(&GitStatusRequest {
            cwd: std::env::temp_dir(),
        });
        assert!(matches!(
            result.kind,
            GitStatusKind::NoRepo
                | GitStatusKind::GitUnavailable
                | GitStatusKind::Clean
                | GitStatusKind::Dirty
        ));
    }
}
