//! ContextBundle builder.

use crate::file_tool::FileReadResult;
use crate::git_tool::{GitStatusKind, GitStatusResult};
use crate::repo_map::RepoMapResult;
use crate::search_tool::SearchMatch;
use researchcode_kernel::context::{estimate_tokens, ContextBundle, ContextItem, ContextItemKind};
use researchcode_kernel::memory::MemoryItem;
use researchcode_kernel::plan::Plan;

pub struct ContextBundleBuilder {
    bundle: ContextBundle,
    next_index: u64,
}

impl ContextBundleBuilder {
    pub fn new(
        bundle_id: impl Into<String>,
        model_family: impl Into<String>,
        max_context_tokens: u64,
    ) -> Self {
        Self {
            bundle: ContextBundle {
                bundle_id: bundle_id.into(),
                model_family: model_family.into(),
                max_context_tokens,
                items: Vec::new(),
            },
            next_index: 1,
        }
    }

    pub fn add_user_task(&mut self, task: &str) -> bool {
        self.push(ContextItemKind::UserTask, "user", task, "internal")
    }

    pub fn add_project_instructions(&mut self, source: &str, instructions: &str) -> bool {
        self.push(
            ContextItemKind::ProjectInstructions,
            source,
            instructions,
            "internal",
        )
    }

    pub fn add_plan(&mut self, plan: &Plan) -> bool {
        self.push(
            ContextItemKind::Plan,
            &plan.plan_id,
            &plan.to_context_text(),
            "internal",
        )
    }

    pub fn add_memory(&mut self, memory: &MemoryItem) -> bool {
        if memory.validate().is_err() {
            return false;
        }
        self.push(
            ContextItemKind::Memory,
            &memory.source,
            &memory.to_context_text(),
            &memory.privacy_class,
        )
    }

    pub fn add_file_read(&mut self, result: &FileReadResult) -> bool {
        self.push(
            ContextItemKind::FileSnippet,
            &result.path.to_string_lossy(),
            &result.content,
            "internal",
        )
    }

    pub fn add_repo_map(&mut self, result: &RepoMapResult) -> bool {
        self.push(
            ContextItemKind::RepoMap,
            "repo.map",
            &result.to_context_text(),
            "internal",
        )
    }

    pub fn add_search_matches(&mut self, matches: &[SearchMatch]) -> bool {
        let content = matches
            .iter()
            .map(|item| {
                format!(
                    "{}:{}:{}",
                    item.path.to_string_lossy(),
                    item.line_number,
                    item.line
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.push(
            ContextItemKind::SearchResult,
            "search.ripgrep",
            &content,
            "internal",
        )
    }

    pub fn add_git_status(&mut self, status: &GitStatusResult) -> bool {
        let source = match status.kind {
            GitStatusKind::Clean => "git.clean",
            GitStatusKind::Dirty => "git.dirty",
            GitStatusKind::NoRepo => "git.no_repo",
            GitStatusKind::GitUnavailable => "git.unavailable",
        };
        self.push(
            ContextItemKind::GitStatus,
            source,
            &status.porcelain,
            "internal",
        )
    }

    pub fn add_tool_result_preview(&mut self, source: &str, preview: &str) -> bool {
        self.push(
            ContextItemKind::ToolResultPreview,
            source,
            preview,
            "internal",
        )
    }

    pub fn build(self) -> ContextBundle {
        self.bundle
    }

    fn push(
        &mut self,
        kind: ContextItemKind,
        source: &str,
        content: &str,
        privacy_class: &str,
    ) -> bool {
        let item = ContextItem {
            item_id: format!("ctx_{:04}", self.next_index),
            kind,
            source: source.to_string(),
            content: content.to_string(),
            token_estimate: estimate_tokens(content),
            privacy_class: privacy_class.to_string(),
        };
        let pushed = self.bundle.push_if_fits(item);
        if pushed {
            self.next_index += 1;
        }
        pushed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn builds_context_from_file_search_and_git() {
        let mut builder = ContextBundleBuilder::new("bundle_1", "qwen", 10_000);
        assert!(builder.add_user_task("fix parser"));
        assert!(builder.add_project_instructions("AGENTS.md", "Read before editing."));
        assert!(builder.add_plan(&researchcode_kernel::plan::Plan {
            plan_id: "plan_1".to_string(),
            task_id: "task_1".to_string(),
            summary: "Fix parser safely".to_string(),
            steps: vec![researchcode_kernel::plan::PlanStep {
                step_id: "step_1".to_string(),
                title: "Read parser".to_string(),
                goal: "Understand parser behavior".to_string(),
                allowed_tools: vec!["file.read".to_string()],
                expected_artifacts: vec!["context".to_string()],
                status: researchcode_kernel::plan::PlanStepStatus::Pending,
            }],
        }));
        assert!(
            builder.add_memory(&researchcode_kernel::memory::MemoryItem {
                memory_id: "mem_1".to_string(),
                scope: researchcode_kernel::memory::MemoryScope::RepoFact,
                source: "repo.fact".to_string(),
                content: "Parser lives in src/parser.rs.".to_string(),
                privacy_class: "internal".to_string(),
                content_hash: "fnv64_mem".to_string(),
            })
        );
        assert!(builder.add_file_read(&FileReadResult {
            path: PathBuf::from("src/parser.rs"),
            content: "fn parse() {}".to_string(),
            truncated: false,
            size_bytes: 13,
        }));
        assert!(builder.add_repo_map(&RepoMapResult {
            root: PathBuf::from("."),
            file_count: 1,
            omitted_count: 0,
            tech_stack: vec!["rust".to_string()],
            important_files: vec![PathBuf::from("Cargo.toml")],
            tree_lines: vec!["Cargo.toml".to_string()],
        }));
        assert!(builder.add_search_matches(&[SearchMatch {
            path: PathBuf::from("src/parser.rs"),
            line_number: 1,
            line: "fn parse() {}".to_string(),
        }]));
        assert!(builder.add_git_status(&GitStatusResult {
            kind: GitStatusKind::NoRepo,
            porcelain: "".to_string(),
        }));
        let bundle = builder.build();
        assert_eq!(bundle.items.len(), 8);
        assert!(bundle.token_estimate() > 0);
    }
}
