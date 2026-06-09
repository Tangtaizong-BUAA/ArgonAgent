use std::collections::HashMap;

use crate::event_log::EventLog;

/// Aggregated telemetry for the agent kernel decision layer.
#[derive(Debug, Clone, Default)]
pub struct AgentKernelTelemetry {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_zone_a_hits: u64,
    pub cache_zone_a_misses: u64,
    pub cache_zone_b_hits: u64,
    pub cache_zone_b_misses: u64,
    pub cache_zone_c_hits: u64,
    pub cache_zone_c_misses: u64,
    pub total_reasoning_tokens: u64,
    pub reasoning_replay_count: u64,
    pub reasoning_replay_size_kb: u64,
    pub dsml_leak_events: u64,
    pub alias_resolutions: HashMap<String, u64>,
    pub repair_applications: HashMap<String, u64>,
    pub compaction_count: u64,
    pub compaction_tokens_freed: u64,
    pub compactor_role_calls: u64,
    pub titler_role_calls: u64,
    pub summarizer_role_calls: u64,
    pub executor_role_calls: u64,
    pub flash_role_calls: u64,
    pub recovery_count: u64,
    pub recovery_success_count: u64,
    pub recovery_blocked_count: u64,
    pub http_retry_count: u64,
    pub retry_compact_count: u64,
    pub unknown_tool_names: HashMap<String, u64>,
}

impl AgentKernelTelemetry {
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            self.cache_hits as f64 / total as f64
        }
    }

    pub fn zone_a_hit_rate(&self) -> f64 {
        hit_rate(self.cache_zone_a_hits, self.cache_zone_a_misses)
    }

    pub fn zone_b_hit_rate(&self) -> f64 {
        hit_rate(self.cache_zone_b_hits, self.cache_zone_b_misses)
    }

    pub fn zone_c_hit_rate(&self) -> f64 {
        hit_rate(self.cache_zone_c_hits, self.cache_zone_c_misses)
    }

    /// Aggregate from an event log by scanning for known event types.
    pub fn aggregate_from(event_log: &EventLog) -> Self {
        let mut telemetry = Self::default();
        for event in event_log.iter() {
            match event.event_type.as_str() {
                "agent.capability.cache_hit" => telemetry.cache_hits += 1,
                "agent.capability.cache_miss" => telemetry.cache_misses += 1,
                "deepseek.cache.zone_a.hit" => {
                    telemetry.cache_hits += 1;
                    telemetry.cache_zone_a_hits += 1;
                }
                "deepseek.cache.zone_a.miss" => {
                    telemetry.cache_misses += 1;
                    telemetry.cache_zone_a_misses += 1;
                }
                "deepseek.cache.zone_b.hit" => {
                    telemetry.cache_hits += 1;
                    telemetry.cache_zone_b_hits += 1;
                }
                "deepseek.cache.zone_b.miss" => {
                    telemetry.cache_misses += 1;
                    telemetry.cache_zone_b_misses += 1;
                }
                "deepseek.cache.zone_c.hit" => {
                    telemetry.cache_hits += 1;
                    telemetry.cache_zone_c_hits += 1;
                }
                "deepseek.cache.zone_c.miss" => {
                    telemetry.cache_misses += 1;
                    telemetry.cache_zone_c_misses += 1;
                }
                "agent.reasoning.token_count" => {
                    if let Some(tokens) = extract_u64_field(&event.payload_json, "tokens") {
                        telemetry.total_reasoning_tokens += tokens;
                    }
                }
                "agent.reasoning.replay_injected" | "reasoning.replay.injected" => {
                    telemetry.reasoning_replay_count += 1;
                    if let Some(chars) = extract_u64_field(&event.payload_json, "chars") {
                        telemetry.reasoning_replay_size_kb += chars.div_ceil(1024);
                    } else if let Some(bytes) = extract_u64_field(&event.payload_json, "bytes") {
                        telemetry.reasoning_replay_size_kb += bytes.div_ceil(1024);
                    }
                }
                "agent.dsml.leak" | "deepseek.dsml.leak" => telemetry.dsml_leak_events += 1,
                "tool.alias.resolved" => {
                    if let Some(alias) = extract_string_field(&event.payload_json, "requested") {
                        *telemetry.alias_resolutions.entry(alias).or_default() += 1;
                    }
                }
                "tool.input.repaired" => {
                    if let Some(rule) = extract_string_field(&event.payload_json, "repair_rule") {
                        *telemetry.repair_applications.entry(rule).or_default() += 1;
                    }
                }
                "agent.compaction.completed" | "context.compaction.completed" => {
                    telemetry.compaction_count += 1;
                    if let (Some(before), Some(after)) = (
                        extract_u64_field(&event.payload_json, "tokens_before"),
                        extract_u64_field(&event.payload_json, "tokens_after"),
                    ) {
                        telemetry.compaction_tokens_freed = telemetry
                            .compaction_tokens_freed
                            .saturating_add(before.saturating_sub(after));
                    }
                }
                "agent.executor.role_call" => telemetry.executor_role_calls += 1,
                "agent.compactor.role_call" | "context.compaction.projected" => {
                    telemetry.compactor_role_calls += 1
                }
                "agent.titler.role_call" => telemetry.titler_role_calls += 1,
                "agent.summarizer.role_call" => telemetry.summarizer_role_calls += 1,
                "deepseek.role_split.flash_savings" => {
                    if extract_bool_field(&event.payload_json, "flash_call").unwrap_or(false) {
                        telemetry.flash_role_calls += 1;
                    }
                }
                "agent.recovery.started" | "agent.loop_recovery" => telemetry.recovery_count += 1,
                "agent.recovery.completed" | "model.http_failure_recovery_succeeded" => {
                    telemetry.recovery_success_count += 1
                }
                "agent.recovery.blocked" => telemetry.recovery_blocked_count += 1,
                "model.http_retry_scheduled" => telemetry.http_retry_count += 1,
                "model.retry_compact_context" => telemetry.retry_compact_count += 1,
                "tool.name.unknown" => {
                    if let Some(tool_name) =
                        extract_string_field(&event.payload_json, "requested_tool")
                    {
                        *telemetry.unknown_tool_names.entry(tool_name).or_default() += 1;
                    }
                }
                _ => {}
            }
        }
        telemetry
    }

    pub fn to_payload_json(&self) -> String {
        format!(
            "{{\"cache_hit_rate\":{},\"cache_hits\":{},\"cache_misses\":{},\"cache_zone_a_hit_rate\":{},\"cache_zone_b_hit_rate\":{},\"cache_zone_c_hit_rate\":{},\"reasoning_tokens\":{},\"reasoning_replay_count\":{},\"reasoning_replay_size_kb\":{},\"dsml_leak_recovered\":{},\"alias_resolution_count\":{},\"repair_rule_applied_count\":{},\"compaction_count\":{},\"compaction_tokens_freed\":{},\"role_split_executor_calls\":{},\"role_split_compactor_calls\":{},\"role_split_titler_calls\":{},\"role_split_summarizer_calls\":{},\"role_split_flash_calls\":{},\"recovery_count\":{},\"recovery_success_count\":{},\"recovery_blocked_count\":{},\"http_retry_count\":{},\"retry_compact_count\":{},\"unknown_tool_count\":{}}}",
            self.cache_hit_rate(),
            self.cache_hits,
            self.cache_misses,
            self.zone_a_hit_rate(),
            self.zone_b_hit_rate(),
            self.zone_c_hit_rate(),
            self.total_reasoning_tokens,
            self.reasoning_replay_count,
            self.reasoning_replay_size_kb,
            self.dsml_leak_events,
            self.alias_resolutions.values().sum::<u64>(),
            self.repair_applications.values().sum::<u64>(),
            self.compaction_count,
            self.compaction_tokens_freed,
            self.executor_role_calls,
            self.compactor_role_calls,
            self.titler_role_calls,
            self.summarizer_role_calls,
            self.flash_role_calls,
            self.recovery_count,
            self.recovery_success_count,
            self.recovery_blocked_count,
            self.http_retry_count,
            self.retry_compact_count,
            self.unknown_tool_names.values().sum::<u64>(),
        )
    }

    /// Return a human-readable summary line for CLI display.
    pub fn summary_line(&self) -> String {
        format!(
            "cache_hit_rate={:.1}% hits={} misses={} reasoning_tokens={} dsml_leaks={} aliases={} repairs={} compactions={} compactor={} titler={} summarizer={} unknown_tools={}",
            self.cache_hit_rate() * 100.0,
            self.cache_hits,
            self.cache_misses,
            self.total_reasoning_tokens,
            self.dsml_leak_events,
            self.alias_resolutions.len(),
            self.repair_applications.len(),
            self.compaction_count,
            self.compactor_role_calls,
            self.titler_role_calls,
            self.summarizer_role_calls,
            self.unknown_tool_names.len(),
        )
    }
}

fn hit_rate(hits: u64, misses: u64) -> f64 {
    let total = hits + misses;
    if total == 0 {
        0.0
    } else {
        hits as f64 / total as f64
    }
}

fn extract_u64_field(payload_json: &str, field: &str) -> Option<u64> {
    let marker = format!("\"{field}\":");
    let start = payload_json.find(&marker)? + marker.len();
    let val_str: String = payload_json[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    val_str.parse().ok()
}

fn extract_string_field(payload_json: &str, field: &str) -> Option<String> {
    let marker = format!("\"{field}\":\"");
    let start = payload_json.find(&marker)? + marker.len();
    let end = payload_json[start..].find('"')?;
    Some(payload_json[start..start + end].to_string())
}

fn extract_bool_field(payload_json: &str, field: &str) -> Option<bool> {
    let marker = format!("\"{field}\":");
    let start = payload_json.find(&marker)? + marker.len();
    let rest = payload_json[start..].trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use researchcode_kernel::{Actor, KernelEvent};

    fn make_event(
        sequence: u64,
        event_type: &str,
        payload_json: &str,
        prev_hash: Option<&str>,
    ) -> KernelEvent {
        KernelEvent {
            event_id: format!("ev_{sequence}"),
            schema_version: "v0".to_string(),
            project_id: "proj".to_string(),
            session_id: None,
            task_id: None,
            sequence,
            event_type: event_type.to_string(),
            actor: Actor::Runtime,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            payload_json: payload_json.to_string(),
            prev_hash: prev_hash.map(str::to_string),
            hash: format!("h_{sequence}"),
        }
    }

    #[test]
    fn cache_hit_rate_zero_when_no_events() {
        let t = AgentKernelTelemetry::default();
        assert_eq!(t.cache_hit_rate(), 0.0);
    }

    #[test]
    fn cache_hit_rate_half() {
        let mut t = AgentKernelTelemetry::default();
        t.cache_hits = 5;
        t.cache_misses = 5;
        assert!((t.cache_hit_rate() - 0.5).abs() < 0.001);
    }

    #[test]
    fn aggregate_from_scans_event_types() {
        let mut log = EventLog::default();
        log.append(make_event(1, "agent.capability.cache_hit", "{}", None))
            .ok();
        log.append(make_event(
            2,
            "agent.capability.cache_hit",
            "{}",
            Some("h_1"),
        ))
        .ok();
        log.append(make_event(
            3,
            "agent.capability.cache_miss",
            "{}",
            Some("h_2"),
        ))
        .ok();
        log.append(make_event(
            4,
            "deepseek.cache.zone_a.hit",
            "{}",
            Some("h_3"),
        ))
        .ok();
        log.append(make_event(
            5,
            "deepseek.cache.zone_b.miss",
            "{}",
            Some("h_4"),
        ))
        .ok();
        log.append(make_event(6, "deepseek.dsml.leak", "{}", Some("h_5")))
            .ok();

        let telemetry = AgentKernelTelemetry::aggregate_from(&log);
        assert_eq!(telemetry.cache_hits, 3);
        assert_eq!(telemetry.cache_misses, 2);
        assert_eq!(telemetry.dsml_leak_events, 1);
    }

    #[test]
    fn aggregate_from_counts_alias_resolutions() {
        let mut log = EventLog::default();
        log.append(make_event(
            1,
            "tool.alias.resolved",
            r#"{"requested":"readFile","canonical":"file.read"}"#,
            None,
        ))
        .ok();

        let telemetry = AgentKernelTelemetry::aggregate_from(&log);
        assert_eq!(telemetry.alias_resolutions.get("readFile"), Some(&1));
    }

    #[test]
    fn aggregate_from_tracks_unknown_tools() {
        let mut log = EventLog::default();
        log.append(make_event(
            1,
            "tool.name.unknown",
            r#"{"requested_tool":"madeUpTool","canonical":"madeUpTool"}"#,
            None,
        ))
        .ok();

        let telemetry = AgentKernelTelemetry::aggregate_from(&log);
        assert_eq!(telemetry.unknown_tool_names.get("madeUpTool"), Some(&1));
    }

    #[test]
    fn summary_line_includes_all_fields() {
        let mut t = AgentKernelTelemetry::default();
        t.cache_hits = 10;
        t.cache_misses = 2;
        let line = t.summary_line();
        assert!(line.contains("cache_hit_rate="));
        assert!(line.contains("hits=10"));
        assert!(line.contains("misses=2"));
    }
}
