//! Structured context compaction summary.

use researchcode_kernel::context::{ContextBundle, ContextItemKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionSummary {
    pub source_bundle_id: String,
    pub goal: String,
    pub active_plan: Vec<String>,
    pub constraints: Vec<String>,
    pub relevant_files: Vec<String>,
    pub latest_tool_evidence: Vec<String>,
    pub pending_permissions: Vec<String>,
    pub progress: Vec<String>,
    pub recovery_notes: Vec<String>,
    pub next_steps: Vec<String>,
    pub token_estimate_before: u64,
    pub token_estimate_after: u64,
    pub compaction_reason: String,
}

impl CompactionSummary {
    pub fn to_markdown(&self) -> String {
        format!(
            "# Context Summary\n\n## Goal\n{}\n\n## Active Plan\n{}\n\n## Constraints\n{}\n\n## Relevant Files\n{}\n\n## Latest Tool Evidence\n{}\n\n## Pending Permissions\n{}\n\n## Progress\n{}\n\n## Recovery Notes\n{}\n\n## Next Steps\n{}\n\nToken estimate before compaction: {}\nToken estimate after compaction: {}\nCompaction reason: {}\n",
            self.goal,
            bullet_list(&self.active_plan),
            bullet_list(&self.constraints),
            bullet_list(&self.relevant_files),
            bullet_list(&self.latest_tool_evidence),
            bullet_list(&self.pending_permissions),
            bullet_list(&self.progress),
            bullet_list(&self.recovery_notes),
            bullet_list(&self.next_steps),
            self.token_estimate_before,
            self.token_estimate_after,
            self.compaction_reason
        )
    }
}

pub fn compact_context(bundle: &ContextBundle) -> CompactionSummary {
    let mut goal = "No explicit user task captured.".to_string();
    let mut relevant_files = Vec::new();
    let mut active_plan = Vec::new();
    let mut latest_tool_evidence = Vec::new();
    let mut pending_permissions = Vec::new();
    let mut recovery_notes = Vec::new();
    let mut progress = Vec::new();
    let mut constraints = vec![
        "DeepSeek/Qwen are native optimized; other providers are compatible-only.".to_string(),
        "Plan approval and permission approval are separate event types.".to_string(),
        "Patch writes require read-before-write, stale hash checks, and protected path checks."
            .to_string(),
    ];
    for item in &bundle.items {
        match item.kind {
            ContextItemKind::UserTask => goal = item.content.clone(),
            ContextItemKind::ProjectInstructions => constraints.push(format!(
                "Project instructions retained from {}",
                item.source
            )),
            ContextItemKind::FileSnippet => relevant_files.push(item.source.clone()),
            ContextItemKind::SearchResult => {
                progress.push(format!("Search context from {}", item.source))
            }
            ContextItemKind::Plan => active_plan.push(format!("Active plan from {}", item.source)),
            ContextItemKind::GitStatus => {
                progress.push(format!("Git status source: {}", item.source))
            }
            ContextItemKind::Memory => {
                progress.push(format!("Memory retained from {}", item.source))
            }
            ContextItemKind::PrivacyReport => {
                constraints.push("Sensitive data requires cloud-model approval.".to_string())
            }
            _ => {
                if item.source.contains("tool") {
                    latest_tool_evidence
                        .push(format!("Tool evidence retained from {}", item.source));
                }
                if item.source.contains("permission") {
                    pending_permissions
                        .push(format!("Permission context retained from {}", item.source));
                }
                if item.source.contains("recovery") {
                    recovery_notes.push(format!("Recovery context retained from {}", item.source));
                }
            }
        }
    }
    if active_plan.is_empty() {
        active_plan.push("No active plan captured.".to_string());
    }
    if relevant_files.is_empty() {
        relevant_files.push("No file snippets included.".to_string());
    }
    if latest_tool_evidence.is_empty() {
        latest_tool_evidence.push("No recent tool evidence included.".to_string());
    }
    if pending_permissions.is_empty() {
        pending_permissions.push("No pending permission captured.".to_string());
    }
    if recovery_notes.is_empty() {
        recovery_notes.push("No recovery notes captured.".to_string());
    }
    if progress.is_empty() {
        progress.push("No prior tool progress included.".to_string());
    }
    let token_estimate_before = bundle.token_estimate();
    CompactionSummary {
        source_bundle_id: bundle.bundle_id.clone(),
        goal,
        active_plan,
        constraints,
        relevant_files,
        latest_tool_evidence,
        pending_permissions,
        progress,
        recovery_notes,
        next_steps: vec!["Refresh relevant files before editing.".to_string()],
        token_estimate_before,
        token_estimate_after: (token_estimate_before * 40 / 100).max(1),
        compaction_reason: "structured_context_compaction".to_string(),
    }
}

fn bullet_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use researchcode_kernel::context::{ContextBundle, ContextItem, ContextItemKind};

    #[test]
    fn compact_context_keeps_structured_sections() {
        let bundle = ContextBundle {
            bundle_id: "bundle_1".to_string(),
            model_family: "deepseek".to_string(),
            max_context_tokens: 1000,
            items: vec![
                ContextItem {
                    item_id: "ctx_1".to_string(),
                    kind: ContextItemKind::UserTask,
                    source: "user".to_string(),
                    content: "Fix parser".to_string(),
                    token_estimate: 3,
                    privacy_class: "internal".to_string(),
                },
                ContextItem {
                    item_id: "ctx_project".to_string(),
                    kind: ContextItemKind::ProjectInstructions,
                    source: "AGENTS.md".to_string(),
                    content: "Use RuntimeFacade.".to_string(),
                    token_estimate: 5,
                    privacy_class: "internal".to_string(),
                },
                ContextItem {
                    item_id: "ctx_2".to_string(),
                    kind: ContextItemKind::Plan,
                    source: "plan_1".to_string(),
                    content: "Plan: read then patch".to_string(),
                    token_estimate: 4,
                    privacy_class: "internal".to_string(),
                },
                ContextItem {
                    item_id: "ctx_3".to_string(),
                    kind: ContextItemKind::FileSnippet,
                    source: "src/parser.rs".to_string(),
                    content: "fn parse() {}".to_string(),
                    token_estimate: 4,
                    privacy_class: "internal".to_string(),
                },
                ContextItem {
                    item_id: "ctx_4".to_string(),
                    kind: ContextItemKind::Memory,
                    source: "eval/qwen".to_string(),
                    content: "Qwen patch needs base hash.".to_string(),
                    token_estimate: 5,
                    privacy_class: "internal".to_string(),
                },
            ],
        };
        let summary = compact_context(&bundle);
        let markdown = summary.to_markdown();
        assert!(markdown.contains("## Goal"));
        assert!(markdown.contains("Fix parser"));
        assert!(markdown.contains("Project instructions retained"));
        assert!(markdown.contains("src/parser.rs"));
        assert!(summary
            .active_plan
            .iter()
            .any(|item| item.contains("Active plan")));
        assert!(summary
            .progress
            .iter()
            .any(|item| item.contains("Memory retained")));
        assert_eq!(summary.token_estimate_before, 21);
        assert!(summary.token_estimate_after < summary.token_estimate_before);
    }
}
