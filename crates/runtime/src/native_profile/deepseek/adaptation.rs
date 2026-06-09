//! DeepSeek V4 Pro adaptation layer — reasoning-aware continuation, dual protocol
//! fallback, and reasoning snapshot preservation during permission blocking.
//!
//! These are DeepSeek-specific optimizations that Claude Code does NOT need:
//! - Claude Code has no reasoning_content injection
//! - Claude Code has no dual Anthropic/OpenAI protocol
//! - Claude Code has no reasoning budget to preserve across blocks

use crate::native_profile::deepseek::policy::{ToolCallProtocolMetrics, ToolCallProtocolPolicy};
use crate::native_profile::deepseek::reasoning::ReasoningReplayManager;
use researchcode_kernel::model::NativeModelFamily;
use serde::{Deserialize, Serialize};

// ── Reasoning snapshot (preserved across permission blocks) ────────────────────

/// Captured reasoning state at the moment of permission blocking.
/// Saved with PendingNativeToolExecution and restored on resume.
/// This ensures DeepSeek's reasoning context is not lost during the
/// channel-blocked wait for user decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningSnapshot {
    /// Raw reasoning content for re-injection into continuation requests.
    pub raw_reasoning: String,
    /// Sanitized preview for UI display.
    pub sanitized_preview: String,
    /// Turn index when the snapshot was taken.
    pub turn_index: u32,
    /// Session ID for routing.
    pub session_id: String,
    /// The assistant message ID this reasoning is attached to.
    pub assistant_message_id: String,
}

impl ReasoningSnapshot {
    pub fn from_replay_manager(manager: &ReasoningReplayManager, session_id: &str) -> Option<Self> {
        let entry = manager.latest(session_id)?;
        Some(Self {
            raw_reasoning: entry.raw_reasoning.clone(),
            sanitized_preview: entry.sanitized_preview.clone(),
            turn_index: entry.turn_index,
            session_id: session_id.to_string(),
            assistant_message_id: entry.assistant_message_id.clone(),
        })
    }

    /// Restore this snapshot into the replay manager for the next continuation.
    pub fn restore(&self, manager: &mut ReasoningReplayManager) {
        manager.inject(
            &self.session_id,
            self.turn_index,
            &self.assistant_message_id,
            &self.raw_reasoning,
        );
    }
}

// ── Dual protocol fallback ─────────────────────────────────────────────────────

/// Protocol format used for the outgoing HTTP request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProtocolFormat {
    /// Anthropic-compatible Messages API (preferred — has cache control).
    #[default]
    Anthropic,
    /// OpenAI-compatible Chat Completions API (fallback).
    OpenAI,
}

/// Dual protocol fallback state for DeepSeek endpoints.
/// When Anthropic-format requests fail with 400, we retry in OpenAI format.
#[derive(Debug, Clone, Default)]
pub struct DualProtocolFallback {
    /// Current active protocol format.
    pub current_format: ProtocolFormat,
    /// Whether a fallback occurred on the last request.
    pub did_fallback: bool,
    /// Count of Anthropic→OpenAI fallbacks this session.
    pub fallback_count: u32,
    /// Whether to prefer OpenAI format (sticky after repeated Anthropic failures).
    pub prefer_openai: bool,
}

impl DualProtocolFallback {
    pub fn new() -> Self {
        Self {
            current_format: ProtocolFormat::Anthropic,
            did_fallback: false,
            fallback_count: 0,
            prefer_openai: false,
        }
    }

    /// Record a 400 error on the current protocol. Returns true if we should
    /// retry with the alternate protocol.
    pub fn on_400_error(&mut self) -> Option<ProtocolFormat> {
        match self.current_format {
            ProtocolFormat::Anthropic => {
                self.did_fallback = true;
                self.fallback_count += 1;
                self.current_format = ProtocolFormat::OpenAI;

                // If we've fallen back too many times, prefer OpenAI going forward.
                if self.fallback_count >= 5 {
                    self.prefer_openai = true;
                }

                Some(ProtocolFormat::OpenAI)
            }
            ProtocolFormat::OpenAI => {
                // Already on fallback — cannot retry further.
                None
            }
        }
    }

    /// Record a successful request. Resets the current format preference
    /// but preserves fallback statistics.
    pub fn on_success(&mut self, _format: ProtocolFormat) {
        self.did_fallback = false;
    }

    /// Get the preferred initial format for a new request.
    pub fn preferred_format(&self) -> ProtocolFormat {
        if self.prefer_openai {
            ProtocolFormat::OpenAI
        } else {
            ProtocolFormat::Anthropic
        }
    }
    /// Convert an Anthropic-format body JSON to OpenAI format.
    /// Returns the new body_json and new URL path.
    pub fn convert_anthropic_body_to_openai(
        body_json: &str,
        url: &str,
    ) -> Result<(String, String), String> {
        let mut value: serde_json::Value =
            serde_json::from_str(body_json).map_err(|e| format!("parse anthropic body: {e}"))?;

        // Extract system prompt from top-level field
        let system_text = value
            .as_object_mut()
            .and_then(|obj| obj.remove("system"))
            .and_then(|v| v.as_str().map(String::from));

        // Convert tool_choice from {"type":"auto"} to "auto"
        if let Some(tc) = value.get("tool_choice") {
            if tc.is_object() {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert(
                        "tool_choice".to_string(),
                        serde_json::Value::String("auto".to_string()),
                    );
                }
            }
        }

        // Convert messages: move system to messages array, convert tool_use/tool_result
        if let Some(messages) = value.get_mut("messages").and_then(|m| m.as_array_mut()) {
            // Prepend system message to messages array
            if let Some(sys) = system_text {
                if !sys.is_empty() {
                    messages.insert(0, serde_json::json!({"role": "system", "content": sys}));
                }
            }
            for msg in messages.iter_mut() {
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
                match role {
                    "assistant" => {
                        // Convert content array with tool_use blocks to tool_calls
                        if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                            let mut text_parts = Vec::new();
                            let mut tool_calls = Vec::new();
                            for block in content {
                                match block.get("type").and_then(|t| t.as_str()) {
                                    Some("tool_use") => {
                                        let id =
                                            block.get("id").and_then(|i| i.as_str()).unwrap_or("");
                                        let name = block
                                            .get("name")
                                            .and_then(|n| n.as_str())
                                            .unwrap_or("");
                                        let input = block
                                            .get("input")
                                            .cloned()
                                            .unwrap_or(serde_json::Value::Null);
                                        tool_calls.push(serde_json::json!({
                                            "id": id,
                                            "type": "function",
                                            "function": {
                                                "name": name,
                                                "arguments": serde_json::to_string(&input).unwrap_or_default()
                                            }
                                        }));
                                    }
                                    Some("text") => {
                                        if let Some(t) = block.get("text").and_then(|t| t.as_str())
                                        {
                                            text_parts.push(t.to_string());
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            let text_content = text_parts.join("\n");
                            if text_content.is_empty() && !tool_calls.is_empty() {
                                if let Some(obj) = msg.as_object_mut() {
                                    obj.insert("content".to_string(), serde_json::Value::Null);
                                }
                            } else if !text_content.is_empty() {
                                if let Some(obj) = msg.as_object_mut() {
                                    obj.insert(
                                        "content".to_string(),
                                        serde_json::Value::String(text_content),
                                    );
                                }
                            }
                            if !tool_calls.is_empty() {
                                if let Some(obj) = msg.as_object_mut() {
                                    obj.insert(
                                        "tool_calls".to_string(),
                                        serde_json::Value::Array(tool_calls),
                                    );
                                }
                            }
                        }
                    }
                    "user" => {
                        // Convert content array with tool_result blocks to tool role messages
                        // (handled by expanding user messages with tool_result into separate tool messages)
                    }
                    _ => {}
                }
            }
            // Expand user messages containing tool_result blocks into separate tool messages
            let mut expanded = Vec::new();
            for msg in messages.iter() {
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
                if role == "user" {
                    if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                        let mut text_parts = Vec::new();
                        let mut tool_results = Vec::new();
                        for block in content {
                            match block.get("type").and_then(|t| t.as_str()) {
                                Some("tool_result") => {
                                    let tool_use_id = block
                                        .get("tool_use_id")
                                        .and_then(|i| i.as_str())
                                        .unwrap_or("");
                                    let tc = block.get("content");
                                    let content_str = match tc {
                                        Some(serde_json::Value::String(s)) => s.clone(),
                                        Some(v) => serde_json::to_string(v).unwrap_or_default(),
                                        None => String::new(),
                                    };
                                    let is_error = block
                                        .get("is_error")
                                        .and_then(|e| e.as_bool())
                                        .unwrap_or(false);
                                    tool_results.push(serde_json::json!({
                                        "role": "tool",
                                        "tool_call_id": tool_use_id,
                                        "content": if is_error {
                                            format!("[TOOL_ERROR] {content_str}")
                                        } else {
                                            content_str
                                        }
                                    }));
                                }
                                Some("text") => {
                                    if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                        text_parts.push(t.to_string());
                                    }
                                }
                                _ => {}
                            }
                        }
                        if !text_parts.is_empty() {
                            expanded.push(serde_json::json!({
                                "role": "user",
                                "content": text_parts.join("\n")
                            }));
                        }
                        expanded.extend(tool_results);
                    } else {
                        expanded.push(msg.clone());
                    }
                } else {
                    expanded.push(msg.clone());
                }
            }
            if let Some(obj) = value.as_object_mut() {
                obj.insert("messages".to_string(), serde_json::Value::Array(expanded));
            }
        }

        // Add DeepSeek-specific OpenAI fields
        if let Some(obj) = value.as_object_mut() {
            if !obj.contains_key("thinking") {
                obj.insert(
                    "thinking".to_string(),
                    serde_json::json!({"type": "enabled"}),
                );
            }
            if !obj.contains_key("reasoning_effort") {
                obj.insert(
                    "reasoning_effort".to_string(),
                    serde_json::Value::String("high".to_string()),
                );
            }
            if obj.get("stream").and_then(|s| s.as_bool()).unwrap_or(false) {
                if !obj.contains_key("stream_options") {
                    obj.insert(
                        "stream_options".to_string(),
                        serde_json::json!({"include_usage": true}),
                    );
                }
            }
        }

        // Convert URL: Anthropic uses /messages, OpenAI uses /chat/completions
        let new_url = url.trim_end_matches("/messages").to_string() + "/chat/completions";

        Ok((
            serde_json::to_string(&value).map_err(|e| format!("serialize openai body: {e}"))?,
            new_url,
        ))
    }

    /// Convert an OpenAI-format body JSON back to Anthropic format.
    pub fn convert_openai_body_to_anthropic(
        body_json: &str,
        url: &str,
    ) -> Result<(String, String), String> {
        let mut value: serde_json::Value =
            serde_json::from_str(body_json).map_err(|e| format!("parse openai body: {e}"))?;

        // Extract system from messages array
        let mut system_text = String::new();
        if let Some(messages) = value.get_mut("messages").and_then(|m| m.as_array_mut()) {
            // Pull out system messages
            let mut non_system = Vec::new();
            for msg in messages.iter() {
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
                if role == "system" {
                    if let Some(c) = msg.get("content").and_then(|c| c.as_str()) {
                        if !system_text.is_empty() {
                            system_text.push_str("\n\n");
                        }
                        system_text.push_str(c);
                    }
                } else if role == "tool" {
                    // Convert tool message to user with tool_result content block
                    let tool_call_id = msg
                        .get("tool_call_id")
                        .and_then(|i| i.as_str())
                        .unwrap_or("");
                    let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
                    let is_error = content.starts_with("[TOOL_ERROR]");
                    let clean_content = if is_error { &content[13..] } else { content };
                    non_system.push(serde_json::json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": tool_call_id,
                            "content": clean_content,
                            "is_error": is_error
                        }]
                    }));
                } else if role == "assistant" {
                    // Convert tool_calls to tool_use content blocks
                    let mut content_blocks = Vec::new();
                    if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
                        if !text.is_empty() {
                            content_blocks.push(serde_json::json!({"type": "text", "text": text}));
                        }
                    }
                    if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
                        for tc in tool_calls {
                            let id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
                            let func = tc.get("function");
                            let name = func
                                .and_then(|f| f.get("name"))
                                .and_then(|n| n.as_str())
                                .unwrap_or("");
                            let args_str = func
                                .and_then(|f| f.get("arguments"))
                                .and_then(|a| a.as_str())
                                .unwrap_or("{}");
                            let input: serde_json::Value = serde_json::from_str(args_str)
                                .unwrap_or(serde_json::Value::Object(Default::default()));
                            content_blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": input
                            }));
                        }
                    }
                    let new_msg = if content_blocks.is_empty() {
                        msg.clone()
                    } else {
                        serde_json::json!({
                            "role": "assistant",
                            "content": content_blocks
                        })
                    };
                    non_system.push(new_msg);
                } else {
                    non_system.push(msg.clone());
                }
            }
            if let Some(obj) = value.as_object_mut() {
                obj.insert("messages".to_string(), serde_json::Value::Array(non_system));
            }
        }

        // Set system as top-level field
        if !system_text.is_empty() {
            if let Some(obj) = value.as_object_mut() {
                obj.insert("system".to_string(), serde_json::Value::String(system_text));
            }
        }

        // Convert tool_choice from "auto" to {"type":"auto"}
        if let Some(tc) = value.get("tool_choice") {
            if tc.is_string() && tc.as_str() == Some("auto") {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert(
                        "tool_choice".to_string(),
                        serde_json::json!({"type": "auto"}),
                    );
                }
            }
        }

        // Remove OpenAI-specific fields
        if let Some(obj) = value.as_object_mut() {
            obj.remove("thinking");
            obj.remove("reasoning_effort");
            obj.remove("stream_options");
        }

        // Convert URL back
        let new_url = url.trim_end_matches("/chat/completions").to_string() + "/messages";

        Ok((
            serde_json::to_string(&value).map_err(|e| format!("serialize anthropic body: {e}"))?,
            new_url,
        ))
    }
}

// ── DSML tool call parser metrics ──────────────────────────────────────────────

/// Tracks native JSON tool call parsing vs DSML XML fallback.
/// When native succeeds 5x with zero DSML fallbacks, DSML guidance can be
/// removed from the system prompt to save tokens.
#[derive(Debug, Clone, Default)]
pub struct DsmlMetricsTracker {
    pub metrics: ToolCallProtocolMetrics,
    /// Whether DSML guidance has been removed from the prompt.
    pub dsml_guidance_removed: bool,
}

impl DsmlMetricsTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_native_success(&mut self) {
        self.metrics.native_successes += 1;
    }

    pub fn record_dsml_fallback(&mut self) {
        self.metrics.dsml_fallbacks += 1;
    }

    pub fn record_parser_repair(&mut self) {
        self.metrics.parser_repairs += 1;
    }

    /// Check if DSML guidance should be removed from the prompt.
    /// Condition: ≥5 native successes, 0 DSML fallbacks, 0 repairs.
    pub fn should_remove_dsml_guidance(&self) -> bool {
        !self.dsml_guidance_removed && self.metrics.native_is_stable_enough_to_hide_dsml_guidance()
    }

    /// Mark DSML guidance as removed from the prompt.
    pub fn mark_dsml_guidance_removed(&mut self) {
        self.dsml_guidance_removed = true;
    }

    /// Check if DSML fallback rate has crossed the warning threshold (>20%).
    pub fn should_warn_dsml_fallback(&self, policy: &ToolCallProtocolPolicy) -> bool {
        self.metrics.should_warn_dsml_fallback(policy)
    }
}

// ── DeepSeek adaptation manager (orchestrates all DeepSeek-specific concerns) ──

/// Central manager for DeepSeek V4 Pro adaptation.
/// Aggregates reasoning replay, dual protocol, and DSML tracking.
#[derive(Debug, Clone)]
pub struct DeepSeekAdaptationManager {
    pub reasoning: ReasoningReplayManager,
    pub protocol: DualProtocolFallback,
    pub dsml: DsmlMetricsTracker,
    /// Whether reasoning replay is currently blocked (budget exceeded).
    pub reasoning_replay_blocked: bool,
    pub family: NativeModelFamily,
}

impl DeepSeekAdaptationManager {
    pub fn new(family: NativeModelFamily) -> Self {
        Self {
            reasoning: ReasoningReplayManager::default(),
            protocol: DualProtocolFallback::new(),
            dsml: DsmlMetricsTracker::new(),
            reasoning_replay_blocked: false,
            family,
        }
    }

    /// Capture a reasoning snapshot before permission blocking.
    pub fn capture_reasoning_snapshot(&self, session_id: &str) -> Option<ReasoningSnapshot> {
        ReasoningSnapshot::from_replay_manager(&self.reasoning, session_id)
    }

    /// Restore reasoning after permission resume.
    pub fn restore_reasoning_snapshot(&mut self, snapshot: &ReasoningSnapshot) {
        snapshot.restore(&mut self.reasoning);
    }

    /// Block reasoning replay (budget exceeded).
    pub fn block_reasoning_replay(&mut self) {
        self.reasoning_replay_blocked = true;
    }

    /// Unblock reasoning replay (new turn).
    pub fn unblock_reasoning_replay(&mut self) {
        self.reasoning_replay_blocked = false;
    }

    /// Preferred protocol format for the next request.
    pub fn preferred_format(&self) -> ProtocolFormat {
        self.protocol.preferred_format()
    }

    /// Handle a protocol-level error and return the fallback format if available.
    pub fn on_protocol_error(&mut self) -> Option<ProtocolFormat> {
        self.protocol.on_400_error()
    }

    /// Handle a successful request.
    pub fn on_success(&mut self, format: ProtocolFormat) {
        self.protocol.on_success(format);
    }
}

impl Default for DeepSeekAdaptationManager {
    fn default() -> Self {
        Self::new(NativeModelFamily::DeepSeek)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_snapshot_round_trips() {
        let mut manager = ReasoningReplayManager::default();
        manager.capture_raw_response(
            "test-session",
            1,
            "assistant-1",
            "Need to check .env for sk-secret-key",
        );

        let snapshot = ReasoningSnapshot::from_replay_manager(&manager, "test-session").unwrap();
        assert_eq!(
            snapshot.raw_reasoning,
            "Need to check .env for sk-secret-key"
        );
        assert!(snapshot.sanitized_preview.contains("[REDACTED"));
        assert_eq!(snapshot.turn_index, 1);

        // Restore into a fresh manager
        let mut fresh = ReasoningReplayManager::default();
        snapshot.restore(&mut fresh);
        let restored = fresh.latest("test-session").unwrap();
        assert_eq!(
            restored.raw_reasoning,
            "Need to check .env for sk-secret-key"
        );
        assert!(restored.sanitized_preview.contains("[REDACTED"));
    }

    #[test]
    fn dual_protocol_fallback_anthropic_to_openai() {
        let mut fallback = DualProtocolFallback::new();
        assert_eq!(fallback.current_format, ProtocolFormat::Anthropic);

        let next = fallback.on_400_error();
        assert_eq!(next, Some(ProtocolFormat::OpenAI));
        assert_eq!(fallback.fallback_count, 1);
    }

    #[test]
    fn dual_protocol_openai_fails_no_fallback_left() {
        let mut fallback = DualProtocolFallback::new();
        fallback.current_format = ProtocolFormat::OpenAI;
        assert_eq!(fallback.on_400_error(), None);
    }

    #[test]
    fn dual_protocol_prefers_openai_after_5_fallbacks() {
        let mut fallback = DualProtocolFallback::new();
        for _ in 0..5 {
            fallback.on_400_error();
            // Reset to Anthropic for next iteration to simulate repeated attempts
            if fallback.fallback_count < 5 {
                fallback.current_format = ProtocolFormat::Anthropic;
            }
        }
        assert!(fallback.prefer_openai);
    }

    #[test]
    fn dsml_guidance_removed_after_stable_native() {
        let mut tracker = DsmlMetricsTracker::new();
        tracker.metrics.native_successes = 5;
        tracker.metrics.dsml_fallbacks = 0;
        tracker.metrics.parser_repairs = 0;
        assert!(tracker.should_remove_dsml_guidance());
    }

    #[test]
    fn dsml_guidance_not_removed_with_fallbacks() {
        let mut tracker = DsmlMetricsTracker::new();
        tracker.metrics.native_successes = 10;
        tracker.metrics.dsml_fallbacks = 1;
        assert!(!tracker.should_remove_dsml_guidance());
    }

    #[test]
    fn adaptation_manager_snapshot_flow() {
        let mut manager = DeepSeekAdaptationManager::default();
        manager
            .reasoning
            .capture_raw_response("sess", 2, "msg-2", "reasoning content");

        let snapshot = manager.capture_reasoning_snapshot("sess").unwrap();
        assert_eq!(snapshot.raw_reasoning, "reasoning content");

        // Simulate block + resume
        let mut resumed = DeepSeekAdaptationManager::default();
        resumed.restore_reasoning_snapshot(&snapshot);
        assert!(resumed.reasoning.latest("sess").is_some());
    }
}
