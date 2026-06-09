use crate::agent_kernel::{
    conversation_messages_from_event_log, conversation_messages_to_openai_json,
};
use crate::compaction::compact_context;
use crate::context_builder::ContextBundleBuilder;
use crate::context_policy::{decide_context_action, native_context_policy, ContextAction};
use crate::git_tool::{git_status, GitStatusRequest};
use crate::patch::stable_text_hash;
use crate::repo_map::{build_repo_map, RepoMapRequest};
use crate::runtime::session_store::{RuntimeFileState, RuntimeSessionRecord};
use researchcode_kernel::context::ContextBundle;
use researchcode_kernel::memory::{MemoryItem, MemoryScope};
use std::fs;

#[derive(Debug, Default)]
pub struct ContextService;

impl ContextService {
    pub fn new() -> Self {
        Self
    }

    pub(crate) fn build_context_bundle(
        &self,
        record: &RuntimeSessionRecord,
    ) -> Result<ContextBundle, String> {
        let policy = native_context_policy(record.handle.model_mode.family());
        let max_context_tokens = policy.max_context_tokens;
        let mut builder = ContextBundleBuilder::new(
            format!("{}_context", record.handle.session_id),
            record.handle.model_mode.as_str(),
            max_context_tokens,
        );
        builder.add_user_task("ResearchCode runtime facade session");
        for instruction_file in ["AGENTS.md", "RESEARCHCODE.md"] {
            let path = record.handle.workspace_root.join(instruction_file);
            if let Ok(text) = fs::read_to_string(&path) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    builder.add_project_instructions(instruction_file, trimmed);
                }
            }
        }
        if let Ok(repo_map) = build_repo_map(&RepoMapRequest {
            root: record.handle.workspace_root.clone(),
            max_files: 160,
            max_depth: 4,
        }) {
            builder.add_repo_map(&repo_map);
        }
        builder.add_git_status(&git_status(&GitStatusRequest {
            cwd: record.handle.workspace_root.clone(),
        }));
        for (index, note) in record
            .session_memory
            .iter()
            .rev()
            .take(12)
            .rev()
            .filter(|n| !is_plateau_fallback_note(n))
            .enumerate()
        {
            let memory = MemoryItem {
                memory_id: format!("{}_mem_{index}", record.handle.session_id),
                scope: MemoryScope::Project,
                source: "runtime.session_memory".to_string(),
                content: note.clone(),
                privacy_class: "internal".to_string(),
                content_hash: stable_text_hash(note),
            };
            let _ = builder.add_memory(&memory);
        }
        for file_state in record.file_state.values().take(32) {
            let ranges = format_file_state_ranges(file_state);
            let _ = builder.add_tool_result_preview(
                &format!("file_state:{}", file_state.path),
                &format!(
                    "read file {} hash={}{}",
                    file_state.path, file_state.content_hash, ranges
                ),
            );
        }
        for root in record.discovered_roots.iter().rev().take(8).rev() {
            let _ = builder.add_tool_result_preview(
                &format!("discovered_root:{root}"),
                &format!(
                    "discovered directory/root: {root}; use file.list_directory/file.list_tree before file.read"
                ),
            );
        }
        for (bad_path, correction) in record.path_corrections.iter().take(16) {
            let _ = builder.add_tool_result_preview(
                &format!("path_correction:{bad_path}"),
                &format!("path correction: {bad_path} -> {correction}"),
            );
        }
        let bundle = builder.build();
        match decide_context_action(&policy, bundle.token_estimate()) {
            ContextAction::KeepFullContext => Ok(bundle),
            ContextAction::CompactHistory | ContextAction::StopAndSummarize => {
                let summary = compact_context(&bundle);
                let mut compacted = ContextBundleBuilder::new(
                    format!("{}_context_compacted", record.handle.session_id),
                    record.handle.model_mode.as_str(),
                    max_context_tokens,
                );
                compacted.add_user_task("ResearchCode runtime facade session");
                compacted.add_tool_result_preview("context.compaction", &summary.to_markdown());
                Ok(compacted.build())
            }
        }
    }

    pub(crate) fn conversation_history_openai_json(&self, record: &RuntimeSessionRecord) -> String {
        let messages = conversation_messages_from_event_log(record.session.event_log());
        conversation_messages_to_openai_json(&messages)
    }
}

pub(crate) fn is_plateau_fallback_note(note: &str) -> bool {
    note.contains("runtime 已停止")
        || note.contains("模型这轮没有产出可展示文本")
        || note.contains("runtime 已根据已收集工具证据收束")
        || note.contains("工具预算已用完")
        || note.contains("重复探索或没有新增证据")
        || note.contains("platform_finalizer_fallback")
}

pub(crate) fn format_file_state_ranges(file_state: &RuntimeFileState) -> String {
    if !file_state.read_ranges.is_empty() {
        let ranges = file_state
            .read_ranges
            .iter()
            .map(|(start, end)| format!("{start}..{end}"))
            .collect::<Vec<_>>()
            .join(",");
        return format!(" lines={ranges}");
    }
    match (file_state.line_start, file_state.line_end) {
        (Some(start), Some(end)) if start <= end => format!(" lines={start}..{end}"),
        _ => String::new(),
    }
}
