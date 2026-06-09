use crate::agent_kernel::context_spine::ContextSpineState;
use crate::compaction::CompactionSummary;
use crate::context_budget::DEEPSEEK_COMPACTION_THRESHOLD_TOKENS;
use crate::event_log::{EventLog, ProjectedContext};
use researchcode_kernel::KernelEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionPlan {
    pub preserve_recent_turns: usize,
    pub preserve_latest_reasoning: bool,
    pub marker: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionResult {
    pub summary: CompactionSummary,
    pub spine: ContextSpineState,
    pub projection: ProjectedContext,
    pub token_estimate_before: u64,
    pub token_estimate_after: u64,
    pub compaction_reason: String,
    pub marker: &'static str,
}

impl CompactionResult {
    pub fn to_compact_text(&self) -> String {
        format!(
            "{}\n{}\n{}",
            self.marker,
            self.spine.to_markdown(),
            self.summary.to_markdown()
        )
    }
}

#[derive(Debug, Clone)]
pub struct Compactor {
    plan: CompactionPlan,
}

impl Default for Compactor {
    fn default() -> Self {
        Self {
            plan: CompactionPlan {
                preserve_recent_turns: 4,
                preserve_latest_reasoning: true,
                marker: "[compacted-context]",
            },
        }
    }
}

impl Compactor {
    pub fn new(plan: CompactionPlan) -> Self {
        Self { plan }
    }

    pub fn deepseek_default_plan() -> CompactionPlan {
        CompactionPlan {
            preserve_recent_turns: 4,
            preserve_latest_reasoning: true,
            marker: "[compacted-context]",
        }
    }

    /// Check whether compaction should be triggered given the estimated token count.
    pub fn should_compact(&self, estimated_tokens: u64) -> bool {
        estimated_tokens > DEEPSEEK_COMPACTION_THRESHOLD_TOKENS
    }

    /// Compact the event log: summarize old turns, preserve recent turns and latest reasoning.
    pub fn compact(
        &self,
        event_log: &EventLog,
        token_estimate_before: u64,
        reason: &str,
    ) -> CompactionResult {
        let events: Vec<&KernelEvent> = event_log.iter().collect();
        let turn_boundaries = extract_turn_boundaries(&events);
        let total_turns = turn_boundaries.len();

        // Preserve recent turns
        let keep_count = self.plan.preserve_recent_turns.min(total_turns);
        let compact_turns = if total_turns > keep_count {
            total_turns - keep_count
        } else {
            0
        };

        // Build summary from old turns
        let mut goal = String::new();
        let active_plan = Vec::new();
        let constraints = Vec::new();
        let mut relevant_files = Vec::new();
        let mut latest_tool_evidence = Vec::new();
        let mut pending_permissions = Vec::new();
        let mut progress = Vec::new();
        let mut recovery_notes = Vec::new();
        let next_steps = Vec::new();

        for (turn_idx, (start, _end)) in turn_boundaries.iter().enumerate() {
            if turn_idx >= compact_turns {
                break;
            }
            let turn_events = if *start + 1 < events.len() {
                let end = turn_boundaries
                    .get(turn_idx + 1)
                    .map(|(s, _)| *s)
                    .unwrap_or(events.len());
                &events[*start..end.min(events.len())]
            } else {
                &events[*start..]
            };

            for event in turn_events {
                match event.event_type.as_str() {
                    "agent.turn.started" => {
                        // extract task context from turn start
                    }
                    "agent.tool.pending" => {
                        if let Some(tool_name) = extract_tool_name(&event.payload_json) {
                            if !relevant_files.contains(&tool_name) {
                                relevant_files.push(tool_name);
                            }
                        }
                    }
                    "tool.result_recorded" => {
                        if let Some(tool_name) = extract_tool_name(&event.payload_json) {
                            if !relevant_files.contains(&tool_name) {
                                relevant_files.push(tool_name);
                            }
                        }
                        let preview = truncate_preview(&event.payload_json, 120);
                        if !preview.is_empty() {
                            latest_tool_evidence
                                .push(format!("ref://event/{} {preview}", event.sequence));
                        }
                    }
                    "agent.tool.completed" => {
                        if let Some(tool_name) = extract_tool_name(&event.payload_json) {
                            progress.push(format!(
                                "Tool outcome from ref://event/{}: {}",
                                event.sequence, tool_name
                            ));
                        }
                    }
                    "agent.permission.pending" => {
                        if let Some(perm) = extract_permission(&event.payload_json) {
                            pending_permissions.push(perm);
                        }
                    }
                    "agent.recovery.started" => {
                        if let Some(note) = extract_recovery_note(&event.payload_json) {
                            recovery_notes.push(note);
                        }
                    }
                    "agent.turn.ledger_updated" => {
                        if let Some(status) = extract_progress(&event.payload_json) {
                            progress.push(status);
                        }
                    }
                    _ => {}
                }
            }
        }

        // Deduplicate and trim
        relevant_files.dedup();
        relevant_files.truncate(20);
        latest_tool_evidence.dedup();
        latest_tool_evidence.truncate(15);
        pending_permissions.dedup();
        pending_permissions.truncate(10);
        recovery_notes.dedup();
        recovery_notes.truncate(5);
        progress.dedup();
        progress.truncate(10);
        if self.plan.preserve_latest_reasoning {
            if let Some(reasoning) = latest_reasoning_preview(&events) {
                recovery_notes.insert(0, format!("latest_reasoning: {reasoning}"));
                recovery_notes.truncate(5);
            }
        }

        if goal.is_empty() {
            goal = "Continue the task using preserved context below.".to_string();
        }

        let summary = CompactionSummary {
            source_bundle_id: "compacted".to_string(),
            goal,
            active_plan,
            constraints,
            relevant_files,
            latest_tool_evidence,
            pending_permissions,
            progress,
            recovery_notes,
            next_steps,
            token_estimate_before,
            token_estimate_after: token_estimate_before / 4, // conservative estimate
            compaction_reason: reason.to_string(),
        };

        let preserve_event_count = if compact_turns < total_turns {
            turn_boundaries
                .get(compact_turns)
                .map(|(start, _)| events.len().saturating_sub(*start))
                .unwrap_or_else(|| self.plan.preserve_recent_turns.saturating_mul(8))
        } else {
            events.len()
        };
        let boundary_sequence = if compact_turns > 0 {
            turn_boundaries
                .get(compact_turns - 1)
                .and_then(|(_, end)| events.get(end.saturating_sub(1)))
                .map(|event| event.sequence)
        } else {
            None
        };
        let projection = event_log.project_context(
            boundary_sequence,
            preserve_event_count,
            summary.to_markdown(),
        );

        CompactionResult {
            projection,
            summary,
            spine: ContextSpineState::from_event_log(event_log),
            token_estimate_before,
            token_estimate_after: token_estimate_before / 4,
            compaction_reason: reason.to_string(),
            marker: self.plan.marker,
        }
    }
}

fn extract_turn_boundaries(events: &[&KernelEvent]) -> Vec<(usize, usize)> {
    let mut boundaries = Vec::new();
    let mut current_start: Option<usize> = None;

    for (idx, event) in events.iter().enumerate() {
        match event.event_type.as_str() {
            "agent.turn.started" => {
                if let Some(start) = current_start {
                    boundaries.push((start, idx));
                }
                current_start = Some(idx);
            }
            "agent.turn.completed" | "loop.completed" => {
                if let Some(start) = current_start.take() {
                    boundaries.push((start, idx + 1));
                }
            }
            _ => {}
        }
    }

    if let Some(start) = current_start {
        boundaries.push((start, events.len()));
    }

    boundaries
}

fn extract_tool_name(payload_json: &str) -> Option<String> {
    let marker = "\"tool_id\":\"";
    let start = payload_json.find(marker)? + marker.len();
    let end = payload_json[start..].find('"')?;
    Some(payload_json[start..start + end].to_string())
}

fn truncate_preview(payload_json: &str, max_chars: usize) -> String {
    let preview: String = payload_json.chars().take(max_chars).collect();
    if preview.len() >= max_chars {
        format!("{preview}…")
    } else {
        preview
    }
}

fn extract_permission(payload_json: &str) -> Option<String> {
    let marker = "\"request_type\":\"";
    let start = payload_json.find(marker)? + marker.len();
    let end = payload_json[start..].find('"')?;
    Some(payload_json[start..start + end].to_string())
}

fn extract_recovery_note(payload_json: &str) -> Option<String> {
    let marker = "\"reason\":\"";
    let start = payload_json.find(marker)? + marker.len();
    let end = payload_json[start..].find('"')?;
    Some(payload_json[start..start + end].to_string())
}

fn extract_progress(payload_json: &str) -> Option<String> {
    let pending_marker = "\"pending_tools\":";
    let completed_marker = "\"completed_tools\":";
    let pending_start = payload_json.find(pending_marker)?;
    let pending_val: usize = payload_json[pending_start + pending_marker.len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    let completed_start = payload_json.find(completed_marker)?;
    let completed_val: usize = payload_json[completed_start + completed_marker.len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    Some(format!("pending={pending_val} completed={completed_val}"))
}

fn latest_reasoning_preview(events: &[&KernelEvent]) -> Option<String> {
    events.iter().rev().find_map(|event| {
        let is_reasoning_event = matches!(
            event.event_type.as_str(),
            "model.stream_delta" | "thinking.chain.completed" | "thinking.chain.delta"
        ) || event.payload_json.contains("reasoning_sanitized")
            || event.payload_json.contains("thinking_sanitized")
            || event.payload_json.contains("reasoning_content");
        if is_reasoning_event {
            extract_json_string(&event.payload_json, "preview")
                .or_else(|| extract_json_string(&event.payload_json, "text"))
                .or_else(|| extract_json_string(&event.payload_json, "full_sanitized"))
                .or_else(|| Some(truncate_preview(&event.payload_json, 240)))
        } else {
            None
        }
    })
}

fn extract_json_string(payload_json: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let start = payload_json.find(&marker)? + marker.len();
    let mut value = String::new();
    let mut escaped = false;
    for ch in payload_json[start..].chars() {
        if escaped {
            value.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some(value);
        } else {
            value.push(ch);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use researchcode_kernel::Actor;

    fn sample_event(
        sequence: u64,
        prev_hash: Option<String>,
        event_type: &str,
        payload_json: &str,
    ) -> KernelEvent {
        KernelEvent {
            event_id: format!("evt_{}_{}", sequence, event_type.replace('.', "_")),
            schema_version: "v0".to_string(),
            project_id: "proj".to_string(),
            session_id: Some("sess".to_string()),
            task_id: Some("task".to_string()),
            sequence,
            event_type: event_type.to_string(),
            actor: Actor::Runtime,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            payload_json: payload_json.to_string(),
            prev_hash,
            hash: format!("hash_{sequence}"),
        }
    }

    fn append_sample(log: &mut EventLog, sequence: &mut u64, event_type: &str, payload_json: &str) {
        let prev_hash = log.last().map(|event| event.hash.clone());
        log.append(sample_event(*sequence, prev_hash, event_type, payload_json))
            .unwrap();
        *sequence += 1;
    }

    fn sample_event_log() -> EventLog {
        let mut log = EventLog::default();
        let mut sequence = 1;
        append_sample(
            &mut log,
            &mut sequence,
            "model.stream_delta",
            "{\"provider\":\"user\",\"delta_kind\":\"input\",\"preview\":\"Create VoiceNote tests and run shell verification\"}",
        );
        // Turn 1: exploration
        append_sample(
            &mut log,
            &mut sequence,
            "agent.turn.started",
            "{\"turn_id\":\"t1\"}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.tool.pending",
            "{\"tool_id\":\"file.read\",\"iteration\":0}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.tool.completed",
            "{\"tool_id\":\"file.read\",\"ok\":true}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "tool.result_recorded",
            "{\"tool_call_id\":\"tc_read_1\",\"tool_id\":\"file.read\",\"preview\":\"read file from turn 1\"}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.turn.ledger_updated",
            "{\"pending_tools\":0,\"completed_tools\":1}",
        );
        // Turn 2: edit
        append_sample(
            &mut log,
            &mut sequence,
            "agent.turn.started",
            "{\"turn_id\":\"t2\"}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.tool.pending",
            "{\"tool_id\":\"file.edit\",\"iteration\":0}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.tool.completed",
            "{\"tool_id\":\"file.edit\",\"ok\":true}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.permission.pending",
            "{\"request_type\":\"FileWrite\"}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.turn.ledger_updated",
            "{\"pending_tools\":0,\"completed_tools\":1}",
        );
        // Turn 3: shell
        append_sample(
            &mut log,
            &mut sequence,
            "agent.turn.started",
            "{\"turn_id\":\"t3\"}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.tool.pending",
            "{\"tool_id\":\"shell.command\",\"iteration\":0}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.tool.completed",
            "{\"tool_id\":\"shell.command\",\"ok\":true}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.turn.ledger_updated",
            "{\"pending_tools\":0,\"completed_tools\":1}",
        );
        // Turn 4: read
        append_sample(
            &mut log,
            &mut sequence,
            "agent.turn.started",
            "{\"turn_id\":\"t4\"}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.tool.pending",
            "{\"tool_id\":\"file.read\",\"iteration\":0}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.tool.completed",
            "{\"tool_id\":\"file.read\",\"ok\":true}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.turn.ledger_updated",
            "{\"pending_tools\":0,\"completed_tools\":1}",
        );
        // Turn 5: write
        append_sample(
            &mut log,
            &mut sequence,
            "agent.turn.started",
            "{\"turn_id\":\"t5\"}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.tool.pending",
            "{\"tool_id\":\"file.write\",\"iteration\":0}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.tool.completed",
            "{\"tool_id\":\"file.write\",\"ok\":true}",
        );
        append_sample(
            &mut log,
            &mut sequence,
            "agent.turn.ledger_updated",
            "{\"pending_tools\":0,\"completed_tools\":1}",
        );
        log
    }

    #[test]
    fn deepseek_compaction_plan_preserves_recent_turns_and_reasoning() {
        let plan = Compactor::deepseek_default_plan();
        assert_eq!(plan.preserve_recent_turns, 4);
        assert!(plan.preserve_latest_reasoning);
        assert_eq!(plan.marker, "[compacted-context]");
    }

    #[test]
    fn context_over_deepseek_threshold_triggers_compaction() {
        let compactor = Compactor::default();
        assert!(!compactor.should_compact(192_000));
        assert!(compactor.should_compact(192_001));
        assert!(compactor.should_compact(250_000));
    }

    #[test]
    fn compaction_preserves_recent_turns() {
        let compactor = Compactor::default();
        let log = sample_event_log();
        let result = compactor.compact(&log, 200_000, "test");
        assert_eq!(result.marker, "[compacted-context]");
        assert!(result.token_estimate_before > result.token_estimate_after);
        assert!(result.summary.to_markdown().contains("Goal"));
        assert!(result.summary.to_markdown().contains("Continue the task"));
        assert!(result
            .summary
            .latest_tool_evidence
            .iter()
            .any(|item| item.contains("ref://event/")));
        let evidence_ref = result
            .summary
            .latest_tool_evidence
            .iter()
            .find_map(|item| item.split_whitespace().next())
            .expect("expected evidence ref");
        assert!(log.page_ref(evidence_ref).is_some());
        assert_eq!(
            result.spine.goal,
            "Create VoiceNote tests and run shell verification"
        );
        assert!(result.to_compact_text().contains("[pinned-context-spine]"));
        assert!(result.to_compact_text().contains("ref://event/"));
    }

    #[test]
    fn compaction_keeps_recent_turn_window_and_folds_old_observations_with_refs() {
        let compactor = Compactor::default();
        let mut log = EventLog::default();
        let mut sequence = 1;
        append_sample(
            &mut log,
            &mut sequence,
            "model.stream_delta",
            "{\"provider\":\"user\",\"delta_kind\":\"input\",\"preview\":\"Run a long observation task\"}",
        );
        for turn in 1..=6 {
            append_sample(
                &mut log,
                &mut sequence,
                "agent.turn.started",
                &format!("{{\"turn_id\":\"t{turn}\"}}"),
            );
            append_sample(
                &mut log,
                &mut sequence,
                "tool.call_requested",
                &format!(
                    "{{\"tool_call_id\":\"tc{turn}\",\"tool_id\":\"file.read\",\"arguments\":{{\"path\":\"file_{turn}.rs\"}}}}"
                ),
            );
            append_sample(
                &mut log,
                &mut sequence,
                "tool.call_completed",
                &format!("{{\"tool_call_id\":\"tc{turn}\",\"tool_id\":\"file.read\",\"ok\":true}}"),
            );
            append_sample(
                &mut log,
                &mut sequence,
                "tool.result_recorded",
                &format!(
                    "{{\"tool_call_id\":\"tc{turn}\",\"tool_id\":\"file.read\",\"preview\":\"observed file_{turn}.rs\"}}"
                ),
            );
            append_sample(
                &mut log,
                &mut sequence,
                "agent.turn.completed",
                &format!("{{\"turn_id\":\"t{turn}\"}}"),
            );
        }

        let result = compactor.compact(&log, 250_000, "test");
        let preserved = result.projection.preserved_messages.join("\n");

        assert_eq!(
            result.projection.boundary_event.as_deref(),
            Some("evt_11_agent_turn_completed")
        );
        assert!(!preserved.contains("\"turn_id\":\"t2\""));
        assert!(preserved.contains("\"turn_id\":\"t3\""));
        assert!(preserved.contains("\"turn_id\":\"t6\""));
        assert!(result
            .summary
            .latest_tool_evidence
            .iter()
            .any(|item| item.contains("ref://event/") && item.contains("file_1.rs")));
        assert!(result
            .summary
            .latest_tool_evidence
            .iter()
            .any(|item| item.contains("ref://event/") && item.contains("file_2.rs")));
        let old_ref = result
            .summary
            .latest_tool_evidence
            .iter()
            .find(|item| item.contains("file_1.rs"))
            .and_then(|item| item.split_whitespace().next())
            .expect("old observation ref");
        let paged = log.page_ref(old_ref).expect("page old observation");
        assert_eq!(paged.event.event_type, "tool.result_recorded");
        assert!(paged.event.payload_json.contains("file_1.rs"));
        for item in &result.summary.latest_tool_evidence {
            let reference = item.split_whitespace().next().expect("evidence ref");
            assert_eq!(
                log.page_ref(reference).unwrap().event.event_type,
                "tool.result_recorded"
            );
        }
    }

    #[test]
    fn compaction_emits_structured_result() {
        let compactor = Compactor::default();
        let log = sample_event_log();
        let result = compactor.compact(&log, 250_000, "deepseek_preflight");
        assert_eq!(result.compaction_reason, "deepseek_preflight");
        assert!(!result.to_compact_text().is_empty());
        assert!(result.to_compact_text().contains("[compacted-context]"));
        assert!(result.projection.summary_text.contains("Context Summary"));
    }

    #[test]
    fn compaction_below_threshold_does_not_trigger() {
        let compactor = Compactor::default();
        assert!(!compactor.should_compact(16_000));
        assert!(!compactor.should_compact(192_000)); // exactly at threshold, not above
        assert!(compactor.should_compact(192_001));
    }

    #[test]
    fn compaction_preserves_latest_reasoning_when_enabled() {
        let mut log = sample_event_log();
        let mut sequence = log.len() as u64 + 1;
        append_sample(
            &mut log,
            &mut sequence,
            "model.stream_delta",
            "{\"delta_kind\":\"reasoning_sanitized\",\"preview\":\"important reasoning retained\"}",
        );
        let compactor = Compactor::default();
        let result = compactor.compact(&log, 250_000, "deepseek_preflight");
        let summary = result.summary.to_markdown();
        assert!(summary.contains("latest_reasoning"));
        assert!(summary.contains("important reasoning retained"));
    }

    #[test]
    fn deterministic_projection_is_smaller_than_raw_event_log() {
        let mut log = EventLog::default();
        let mut sequence = 1;
        append_sample(
            &mut log,
            &mut sequence,
            "agent.turn.started",
            "{\"turn_id\":\"large\"}",
        );
        for _ in 0..80 {
            append_sample(
                &mut log,
                &mut sequence,
                "agent.tool.completed",
                &format!(
                    "{{\"tool_id\":\"file.read\",\"ok\":true,\"preview\":\"{}\"}}",
                    "large evidence ".repeat(80)
                ),
            );
        }
        append_sample(
            &mut log,
            &mut sequence,
            "agent.turn.completed",
            "{\"turn_id\":\"large\"}",
        );
        for turn in 0..8 {
            append_sample(
                &mut log,
                &mut sequence,
                "agent.turn.started",
                &format!("{{\"turn_id\":\"recent_{turn}\"}}"),
            );
            append_sample(
                &mut log,
                &mut sequence,
                "agent.tool.completed",
                "{\"tool_id\":\"git.status\",\"ok\":true}",
            );
            append_sample(
                &mut log,
                &mut sequence,
                "agent.turn.ledger_updated",
                "{\"pending_tools\":0,\"completed_tools\":1}",
            );
            append_sample(
                &mut log,
                &mut sequence,
                "agent.turn.completed",
                &format!("{{\"turn_id\":\"recent_{turn}\"}}"),
            );
        }
        let raw_bytes = log.export_jsonl().len();
        let result = Compactor::default().compact(&log, 250_000, "deepseek_preflight");
        let projected_bytes = result.projection.summary_text.len()
            + result.projection.preserved_messages.join("\n").len();
        assert!(projected_bytes < raw_bytes / 2);
    }
}
